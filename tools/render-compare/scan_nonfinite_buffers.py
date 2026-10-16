#!/usr/bin/env python3
"""Count NaN and Inf texels in a 0.26g stage-buffer capture.

    scan_nonfinite_buffers.py CAPTURE_DIR [--pose NAME]

Reads every `<pose>.<stage>.targets.json` (and the pose's own
`<pose>.targets.json`) written by the capture hook (render_buffers.c,
SRP_CAPTURE_BUFFERS=1, a mode-1 pose with "buffers": true) and decodes the
float render targets it dumped (`.rtNN.bin`): R32G32B32A32, R16G16B16A16,
R32G32, R11G11B10, R16G16, R32 and R16 FLOAT. Prints one line per target
with NaN and Inf counts and the bounding box of the bad texels, so the first
stage whose output holds a non-finite value names the pass that made it.
Integer and UNORM targets cannot hold NaN and are skipped. Needs numpy.
"""
import argparse
import glob
import json
import os
import sys

import numpy as np

FLOAT_FORMATS = {2: ('<f4', 4), 10: ('<f2', 4), 16: ('<f4', 2), 26: ('r11g11b10', 3),
                 34: ('<f2', 2), 41: ('<f4', 1), 54: ('<f2', 1)}


def r11g11b10(raw):
    """Decode R11G11B10_FLOAT to float32 (5-bit exponents, no sign)."""
    def channel(bits, mantissa):
        exponent = (bits >> mantissa) & 31
        m = (bits & ((1 << mantissa) - 1)).astype(np.float32)
        out = np.where(exponent == 0, m / (1 << mantissa) * 2.0 ** -14,
                       (1 + m / (1 << mantissa)) * np.exp2(exponent.astype(np.float32) - 15))
        out = np.where((exponent == 31) & (m == 0), np.inf, out)
        return np.where((exponent == 31) & (m != 0), np.nan, out)
    v = raw.view('<u4')
    return np.stack([channel(v & 0x7ff, 6), channel((v >> 11) & 0x7ff, 6), channel(v >> 22, 5)], -1)


def scan(directory, pose=None):
    rows = []
    for index in sorted(glob.glob(os.path.join(directory, '*targets.json'))):
        base = index[:-len('.targets.json')]
        if pose and not os.path.basename(base).startswith(pose):
            continue
        try:
            entries = json.load(open(index))
        except (OSError, json.JSONDecodeError):
            continue
        for entry in entries:
            fmt = FLOAT_FORMATS.get(entry.get('format'))
            path = f"{base}.rt{entry['slot']:02d}.bin"
            if not fmt or not entry.get('written') or not os.path.exists(path):
                continue
            w, h = entry['width'], entry['height']
            raw = np.fromfile(path, dtype=np.uint8)
            if fmt[0] == 'r11g11b10':
                if raw.size != w * h * 4:
                    continue
                texels = r11g11b10(raw).reshape(h, w, 3)
            else:
                dtype = np.dtype(fmt[0])
                if raw.size != w * h * fmt[1] * dtype.itemsize:
                    continue
                texels = raw.view(dtype).astype(np.float32).reshape(h, w, fmt[1])
            nan = np.isnan(texels).any(-1)
            inf = np.isinf(texels).any(-1)
            bad = np.argwhere(nan | inf)
            box = (bad.min(0).tolist()[::-1] + bad.max(0).tolist()[::-1]) if len(bad) else None
            rows.append((os.path.basename(base), entry['slot'], entry.get('name', ''), entry['format'],
                         int(nan.sum()), int(inf.sum()), box))
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('capture_dir')
    ap.add_argument('--pose')
    a = ap.parse_args()
    rows = scan(a.capture_dir, a.pose)
    for stage, slot, name, fmt, nan, inf, box in rows:
        flag = '  <-- non-finite' if nan or inf else ''
        print(f'{stage:60s} rt{slot:02d} {name[:40]:40s} fmt {fmt:3d} nan {nan:8d} inf {inf:8d} {box or ""}{flag}')
    bad = sum(1 for r in rows if r[4] or r[5])
    print(f'{len(rows)} float targets, {bad} with non-finite texels')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
