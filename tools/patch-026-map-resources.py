#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Create a separate 0.26g development client accepting modified map resources.

The known SRP client already skips archival certificate-chain validation.
At VA 0x135d3, skip only the resource RSA/hash verifier call (0x12cb0),
returning success (zero); the caller still releases the certificate/store.
The four cdecl arguments are cleaned up by the following add esp,0x10.
This does not change TLS/login checks or the multiplayer simulation hash.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path

KNOWN_SRP = '45baf0a591cf6535a9f7afa073ae59551979d92904670a4e1adf70336c1004e9'
VA = 0x135d3
BEFORE = bytes.fromhex('e8d8f6ffff')
AFTER = bytes.fromhex('31c0909090')


def patch(data):
    if hashlib.sha256(data).hexdigest() != KNOWN_SRP:
        raise ValueError('Expected the reviewed SRP 0.26g client; input SHA-256 differs')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    opt = pe + 24
    base = struct.unpack_from('<I', data, opt + 28)[0]
    count = struct.unpack_from('<H', data, pe + 6)[0]
    sections = opt + struct.unpack_from('<H', data, pe + 20)[0]
    for n in range(count):
        _, rva, size, raw = struct.unpack_from('<IIII', data, sections + 40*n + 8)
        if base + rva <= VA and VA + 8 <= base + rva + size:
            offset = raw + VA - base - rva
            if data[offset:offset+8] != BEFORE + bytes.fromhex('83c410'):
                raise ValueError('Resource verifier call/stack cleanup changed')
            result = data[:offset] + AFTER + data[offset+len(BEFORE):]
            return result, offset
    raise ValueError('Resource verifier is not in a file-backed PE section')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    if args.input.resolve() == args.output.resolve() or args.output.exists():
        parser.error('Use a new output filename; retain the original executable')
    result, offset = patch(args.input.read_bytes())
    args.output.write_bytes(result)
    record = dict(input_sha256=KNOWN_SRP, output_sha256=hashlib.sha256(result).hexdigest(),
                  va=hex(VA), offset=hex(offset), before=BEFORE.hex(), after=AFTER.hex())
    args.output.with_suffix('.patch.json').write_text(json.dumps(record, indent=2)+'\n')
    print(record['output_sha256'], args.output)


if __name__ == '__main__':
    main()
