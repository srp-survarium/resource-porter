#!/usr/bin/env python3
"""Prove adding map selectors preserves every existing shader instruction path.

Arguments: OLD_PACK OLD_MANIFEST NEW_PACK NEW_MANIFEST.
Only explicit marker guards and scratch/static-CB declarations are normalized.
All other instructions, including native control flow and constants, must match.
"""
import json
from pathlib import Path
import struct
import sys

from check_021_shader_fallback import instructions, unmarked, check_declarations


def markers(path):
    profiles = json.loads(path.read_text())["profiles"]
    return {entry["project"]: struct.unpack("<I", struct.pack("<f", entry["marker"]))[0]
            for entry in profiles}


def main():
    old_pack, old_manifest, new_pack, new_manifest = map(Path, sys.argv[1:])
    old_markers, new_markers = markers(old_manifest), markers(new_manifest)
    assert old_markers.items() <= new_markers.items(), "existing selectors changed"
    old_ops = json.loads((old_pack / "pack.json").read_text())["operations"]
    new_ops = json.loads((new_pack / "pack.json").read_text())["operations"]
    assert old_ops == new_ops, "resource set or native fingerprints changed"
    normalize = lambda ops: [op for op in ops if op[0] & 2047 not in (104, 89)]
    for operation in old_ops:
        path = operation["path"]
        before = instructions((old_pack / "files" / path).read_bytes())
        after = instructions((new_pack / "files" / path).read_bytes())
        check_declarations(path, before, after)
        for project, selected in old_markers.items():
            old_path, _ = unmarked(before, set(old_markers.values()), selected)
            new_path, _ = unmarked(after, set(new_markers.values()), selected)
            assert normalize(old_path) == normalize(new_path), (path, project, "existing rendering changed")
        print("PASS existing profiles", path)
    print(f"All {len(old_ops)} shader resources preserve every instruction for the {len(old_markers)} existing profiles.")


if __name__ == "__main__":
    main()
