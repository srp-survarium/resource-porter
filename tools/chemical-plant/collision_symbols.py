#!/usr/bin/env python3
"""Read collision-related PUB32 records from large-directory MSF 7 PDBs."""
import argparse
import json
import math
from pathlib import Path
import struct


def public_symbols(path, patterns=None):
    if patterns is None:
        patterns = ("create_bt_primitive", "create_compound_shape", "create_primitive_shape",
                    "create_primitives_shape", "initialize@bt_character_controller",
                    "create_game_objects@project_cooker_simple")
    with path.open('rb') as f:
        header = f.read(4096)
        if not header.startswith(b'Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0'):
            raise ValueError('not MSF 7')
        block, _, _, size, _ = struct.unpack_from('<5I', header, 32)
        def blocks(ids):
            result = bytearray()
            for i in ids:
                f.seek(i*block); result.extend(f.read(block))
            return bytes(result)
        n = math.ceil(size/block)
        maps = struct.unpack_from('<'+'I'*math.ceil(n*4/block), header, 52)
        directory = blocks(struct.unpack_from('<'+'I'*n, blocks(maps)))[:size]
        count = struct.unpack_from('<I', directory)[0]
        sizes = struct.unpack_from('<'+'I'*count, directory, 4)
        offset = 4+4*count; streams = []
        for s in sizes:
            n = math.ceil(s/block) if s != 0xffffffff else 0
            ids = struct.unpack_from('<'+'I'*n, directory, offset); offset += n*4
            streams.append((ids, s))
        dbi = blocks(streams[3][0])[:streams[3][1]]
        symbol_stream = struct.unpack_from('<H', dbi, 20)[0]
        ids, size = streams[symbol_stream]; data = blocks(ids)[:size]
        offset = 0
        while offset+4 <= len(data):
            length, kind = struct.unpack_from('<HH', data, offset)
            if length < 2 or offset+length+2 > len(data): raise ValueError('invalid symbol record')
            if kind == 0x110e:
                _, address, segment = struct.unpack_from('<IIH', data, offset+4)
                name = data[offset+14:offset+length+2].split(b'\0')[0].decode('utf8', 'replace')
                if any(x in name for x in patterns):
                    yield dict(name=name, segment=segment, offset=address)
            offset += length+2


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__); ap.add_argument('pdb', type=Path)
    a = ap.parse_args(); print(json.dumps(list(public_symbols(a.pdb)), indent=2))
