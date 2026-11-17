#!/usr/bin/env python3
"""Independently disassemble every original light branch and compare its math.

PORTER DONOR_DB NATIVE_DB PACK MANIFEST CACHE_DIR VKD3D_COMPILER
Only screen UV reconstruction and reflected binding slots may differ.
"""
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import sys


def u32(data, at):
    return struct.unpack_from("<I", data, at)[0]


def dxbc(blob):
    offset = 4 + 264 * u32(blob, 0)
    assert blob[offset:offset + 4] == b"DXBC"
    return blob[offset:]


def bindings(blob):
    data = dxbc(blob)
    for index in range(u32(data, 28)):
        at = u32(data, 32 + 4 * index)
        if data[at:at + 4] != b"RDEF":
            continue
        r = data[at + 8:at + 8 + u32(data, at + 4)]
        result = {}
        for index in range(u32(r, 8)):
            at = u32(r, 12) + 32 * index
            start = u32(r, at)
            name = r[start:r.index(b"\0", start)].decode()
            kind, slot, count = u32(r, at + 4), u32(r, at + 20), u32(r, at + 24)
            assert count == 1
            result[(kind, name)] = slot
        return result
    raise ValueError("missing reflection")


def main():
    porter, donor, native, pack, manifest, cache, compiler = sys.argv[1:]
    pack, cache = Path(pack), Path(cache)
    cache.mkdir(parents=True, exist_ok=True)
    markers = [p["marker"] for p in json.loads(Path(manifest).read_text())["profiles"]]
    pins = Path(__file__).resolve().parents[2] / "src/weather_fog_021/local_lights.tsv"

    def extract(db, path, digest):
        out = cache / digest
        if not out.exists():
            subprocess.run([porter, "extract", "--db", db, "--path", path,
                            "--output", str(out)], check=True, stdout=subprocess.DEVNULL)
        blob = out.read_bytes()
        assert hashlib.sha256(blob).hexdigest() == digest, path
        return blob

    def disassemble(blob):
        path = cache / (hashlib.sha256(blob).hexdigest() + ".dxbc")
        path.write_bytes(dxbc(blob))
        text = subprocess.check_output([compiler, "-x", "dxbc-tpf", "-b", "d3d-asm", str(path)], text=True)
        return [line.strip() for line in text.splitlines()]

    count = 0
    for line in pins.read_text().splitlines():
        if line.startswith("#"):
            continue
        source_path, native_path, source_hash, native_hash = line.split("\t")
        source = extract(donor, source_path, source_hash)
        target = extract(native, native_path, native_hash)
        source_bindings, native_bindings = bindings(source), bindings(target)
        slots = {}
        for key, slot in source_bindings.items():
            prefix = {0: "cb", 2: "t", 3: "s"}[key[0]]
            slots[prefix + str(slot)] = prefix + str(native_bindings[key])
        original = disassemble(source)
        start = next(i for i, op in enumerate(original) if op.startswith("dcl_temps ")) + 1
        assert original[start] == "div r0.xy, v1.xyxx, v1.w"
        expected = ["mul r0.xy, v0.xyxx, cb1[14].zwzz"]
        for op in original[start + 1:]:
            expected.append(re.sub(r"\b(?:cb|t|s)\d+\b", lambda m: slots[m.group()], op))
        adapted = disassemble((pack / "files" / native_path).read_bytes())
        guards = [i for i, op in enumerate(adapted) if op.startswith("eq ") and "cb1[18].w" in op]
        assert len(guards) == len(markers)
        for index, at in enumerate(guards):
            value = float(re.search(r"l\(([^)]+)\)$", adapted[at]).group(1))
            assert value == markers[index]
            assert adapted[at + 1].startswith("if_nz ")
            actual = adapted[at + 2:at + 2 + len(expected)]
            assert actual == expected, (native_path, index, "original arithmetic changed")
            assert adapted[at + 2 + len(expected)] == "endif"
        print("PASS original arithmetic", native_path)
        count += 1
    assert count == 32
    print("All 32 original light programs are preserved for every marker, with only UV/binding adaptation.")


if __name__ == "__main__":
    main()
