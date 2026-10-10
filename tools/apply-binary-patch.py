#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Apply a fixed-size binary patch after validating the original bytes."""

from __future__ import annotations

import argparse
import os
import shutil
import tempfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, type=Path)
    parser.add_argument("--offset", required=True, type=lambda value: int(value, 0))
    parser.add_argument("--expected", required=True, type=Path)
    parser.add_argument("--replacement", required=True, type=Path)
    parser.add_argument("--backup", required=True, type=Path)
    args = parser.parse_args()

    expected = args.expected.read_bytes()
    replacement = args.replacement.read_bytes()
    if len(expected) != len(replacement):
        raise SystemExit(
            f"patch length mismatch: expected={len(expected)} replacement={len(replacement)}"
        )

    target = args.target.read_bytes()
    end = args.offset + len(expected)
    if end > len(target):
        raise SystemExit(
            f"patch range 0x{args.offset:x}..0x{end:x} exceeds target size {len(target)}"
        )

    current = target[args.offset:end]
    if current == replacement:
        print("patch already applied")
        return
    if current != expected:
        raise SystemExit(f"target bytes do not match expected bytes at 0x{args.offset:x}")

    args.backup.parent.mkdir(parents=True, exist_ok=True)
    if args.backup.exists():
        backup = args.backup.read_bytes()
        if backup[args.offset:end] != expected:
            raise SystemExit(f"existing backup is not the expected original: {args.backup}")
    else:
        shutil.copy2(args.target, args.backup)

    patched = bytearray(target)
    patched[args.offset:end] = replacement

    mode = args.target.stat().st_mode
    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{args.target.name}.",
        dir=args.target.parent,
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(patched)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, args.target)
    finally:
        temporary.unlink(missing_ok=True)

    print(
        f"patched {len(replacement)} bytes at "
        f"0x{args.offset:x}; backup={args.backup}"
    )


if __name__ == "__main__":
    main()
