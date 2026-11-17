#!/usr/bin/env python3
"""Compare every built shader's unmarked DXBC path with its pinned native input.

Usage: check_021_shader_fallback.py PORTER NATIVE_DB PACK MANIFEST CACHE_DIR
NATIVE_DB may be a layered .split list. No game or donor capture is started.
"""
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys


def instructions(blob):
    dxbc = blob[blob.index(b"DXBC"):]
    u32 = lambda at: struct.unpack_from("<I", dxbc, at)[0]
    for index in range(u32(28)):
        at = u32(32 + 4 * index)
        if dxbc[at:at + 4] != b"SHDR":
            continue
        size = u32(at + 4)
        words = struct.unpack("<" + "I" * (size // 4), dxbc[at + 8:at + 8 + size])
        result, cursor = [], 2
        while cursor < len(words):
            count = (words[cursor] >> 24) & 127
            assert count, "unsupported custom instruction"
            result.append(words[cursor:cursor + count])
            cursor += count
        assert cursor == len(words)
        return result
    raise ValueError("missing SHDR")


def unmarked(ops, markers, selected=None):
    """Resolve explicit marker guards; None selects the native fallback."""
    result, cursor, guards = [], 0, 0
    while cursor < len(ops):
        op = ops[cursor]
        # The rain converter preserves native r0.w except for named rain
        # profiles: ne r9.x, marker, selected; and r0.w, r0.w, r9.x.
        if (len(op) == 8 and op[:7] == (0x08000039, 0x00100012, 9,
                0x0020803a, 1, 18, 0x00004001) and op[-1] in markers):
            assert ops[cursor + 1] == (0x07000001, 0x00100082, 0, 0x0010003a, 0, 0x0010000a, 9)
            assert selected != op[-1], "rain-branch comparison needs explicit conditional evaluation"
            cursor += 2
            guards += 1
            continue
        if not (len(op) == 8 and op[0] == 0x08000018
                and op[3:7] == (0x0020803a, 1, 18, 0x00004001)
                and op[-1] in markers):
            result.append(op)
            cursor += 1
            continue
        assert ops[cursor + 1][0] == 0x0304001f
        # The condition must read exactly the scalar written by eq.
        register = op[2]
        component = {0x00100012: 0x0010000a, 0x00100082: 0x0010003a}[op[1]]
        assert ops[cursor + 1][1:] == (component, register)
        depth, end, otherwise = 1, cursor + 2, None
        while depth:
            opcode = ops[end][0] & 2047
            if opcode == 31:
                depth += 1
            if opcode == 21:
                depth -= 1
            if opcode == 18 and depth == 1:
                otherwise = end
            end += 1
        if selected == op[-1]:
            branch = ops[cursor + 2:otherwise if otherwise is not None else end - 1]
        elif otherwise is not None:
            branch = ops[otherwise + 1:end - 1]
        else:
            branch = []
        if branch:
            body, nested = unmarked(branch, markers, selected)
            result.extend(body)
            guards += nested
        guards += 1
        cursor = end
    return result, guards


def check_declarations(path, old, new):
    old_cb = [op for op in old if op[0] & 2047 == 89]
    new_cb = [op for op in new if op[0] & 2047 == 89]
    added = {(0x04000059, 0x00208e46, 1, 19), (0x04000059, 0x00208e46, 1, 26)}
    expanded_families = (
        "ssao_hemisphere_optimized.ps", "point_light_accumulator.ps", "spot_light_accumulator.ps")
    expand = path.split("/")[-2] in expanded_families
    for op in old_cb:
        assert op in new_cb or (expand and op == (0x04000059, 0x00208e46, 1, 15)
                               and (0x04000059, 0x00208e46, 1, 19) in new_cb), (path, "native CB changed")
    assert all(op in old_cb or op in added for op in new_cb), (path, "unexpected CB")
    old_temps = [op[1] for op in old if op[0] & 2047 == 104]
    new_temps = [op[1] for op in new if op[0] & 2047 == 104]
    assert len(old_temps) == len(new_temps) == 1 and new_temps[0] >= old_temps[0]


def main():
    porter, native_db, pack_path, manifest_path, cache_path = sys.argv[1:]
    pack, cache = Path(pack_path), Path(cache_path)
    cache.mkdir(parents=True, exist_ok=True)
    profiles = json.loads(Path(manifest_path).read_text())["profiles"]
    markers = {struct.unpack("<I", struct.pack("<f", p["marker"]))[0] for p in profiles}
    assert len(markers) == len(profiles)
    operations = json.loads((pack / "pack.json").read_text())["operations"]
    for operation in operations:
        path = operation["path"]
        dest = cache / operation["base_sha256"]
        if not dest.exists():
            subprocess.run([porter, "extract", "--db", native_db, "--path", path,
                            "--output", str(dest)], check=True, stdout=subprocess.DEVNULL)
        native = dest.read_bytes()
        assert hashlib.sha256(native).hexdigest() == operation["base_sha256"], path
        old = instructions(native)
        new, guards = unmarked(instructions((pack / "files" / path).read_bytes()), markers)
        rain = {struct.unpack("<I", struct.pack("<f", p["marker"]))[0]
                for p in profiles if "/rain/" in p["project"]}
        raw = instructions((pack / "files" / path).read_bytes())
        rain_guards = [op[-1] for op in raw if len(op) == 8 and op[:7] ==
                       (0x08000039, 0x00100012, 9, 0x0020803a, 1, 18, 0x00004001)]
        expected_rain = rain if "/composition.ps/" in path else set()
        assert set(rain_guards) == expected_rain and len(rain_guards) == len(expected_rain), path
        assert guards == len(markers) + len(expected_rain), (path, guards)
        check_declarations(path, old, new)
        # Scratch allocation and the guarded branch's extra/expanded static CB
        # are declarations only. Every executable native instruction must match.
        normalize = lambda ops: [op for op in ops if op[0] & 2047 not in (104, 89)]
        assert normalize(old) == normalize(new), (path, "native instructions differ")
        print("PASS", path)
    print(f"All {len(operations)} resources preserve the native instruction path outside the manifest markers.")


if __name__ == "__main__":
    main()
