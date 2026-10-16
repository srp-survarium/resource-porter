#!/usr/bin/env python3
"""Top-level tags of a SWF: (code, character id or None, payload bytes).

    swf_tags.py ORIGINAL.swf PATCHED.swf   # every original tag is kept
                                           # byte-identical except the ones
                                           # the HUD minimap patch rewrites
"""
import struct
import sys
import zlib

DEFINE_BITS_JPEG3, DO_ABC2, SYMBOL_CLASS = 35, 82, 76


def tags(data):
    if data[:3] == b'CWS':
        data = data[:8] + zlib.decompress(data[8:])
    elif data[:3] != b'FWS':
        raise ValueError('not an uncompressed or zlib SWF')
    nbits = data[8] >> 3
    at = 8 + (5 + 4 * nbits + 7) // 8 + 4
    out = []
    while at < len(data):
        head = struct.unpack_from('<H', data, at)[0]; at += 2
        code, size = head >> 6, head & 0x3f
        if size == 0x3f:
            size = struct.unpack_from('<I', data, at)[0]; at += 4
        payload = data[at:at + size]; at += size
        out.append((code, payload))
        if code == 0:
            break
    return out


def compare(original, patched):
    """Original tags not in the patched file, and patched tags not in the
    original (as code lists). The patch may only rewrite the DoABC2 holding
    survarium.controls.Minimap and the SymbolClass, and add JPEG3 images."""
    a, b = tags(original), tags(patched)
    sb = set(p for _, p in b)
    missing = [(c, p) for c, p in a if p not in sb]
    sa = set(p for _, p in a)
    added = [(c, p) for c, p in b if p not in sa]
    return missing, added


if __name__ == '__main__':
    missing, added = compare(open(sys.argv[1], 'rb').read(), open(sys.argv[2], 'rb').read())
    print('changed or removed original tags:', [(c, p[:40]) for c, p in missing if c != DO_ABC2] or '',
          [p[4:4 + p[4:].index(0)] for c, p in missing if c == DO_ABC2])
    print('new tags:', sorted(c for c, _ in added))
    bad = [c for c, p in missing if not (c == SYMBOL_CLASS or (c == DO_ABC2 and b'survarium/controls/Minimap\0' in p[:64]))]
    bad += [c for c, _ in added if c not in (DEFINE_BITS_JPEG3, DO_ABC2, SYMBOL_CLASS)]
    sys.exit(1 if bad or len(missing) != 2 else 0)
