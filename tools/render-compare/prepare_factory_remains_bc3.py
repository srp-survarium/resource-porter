#!/usr/bin/env python3
"""Prepare a reversible, same-size BC3 streaming candidate for two Factory DDSs.

RGB compression blocks, mip dimensions and the appended sRGB option are retained.
Only explicit BC2 alpha is fitted to BC3 palettes. Never writes the archive.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

import numpy as np


SOURCE_HASHES = {
    'skeleton': 'b1e79f85ade5e16650c79eda19aae9ce9d01dbcc145c0bf20f79d8efd0dfc834',
    'rag': 'ca56e64a5f9b8677513ae8421ba7f2668d038e3ab21271c629605655c9f3ca74',
}


def transcode(raw):
    assert raw[:4] == b'DDS ' and raw[84:88] == b'DXT3'
    height, width = struct.unpack_from('<II', raw, 12)
    mips = struct.unpack_from('<I', raw, 28)[0]
    assert (width, height, mips, raw[-1]) == (2048, 2048, 10, 1)
    # Source cutout alpha uses 0..7 and15 only. Six-step BC3 palettes also
    # supply exact0/255; fit nearby endpoints without changing color blocks.
    endpoints = [(a, b) for a in range(0, 120, 17) for b in range(a, 120, 17)]
    palettes = np.array([[a, b, *[( (5-k)*a+k*b)/5 for k in range(1, 5)], 0, 255]
                         for a, b in endpoints], dtype=np.float64)
    values = np.arange(16)*17
    errors = (values[None, :, None]-palettes[:, None, :])**2
    # Both source and native materials use alpha_ref=.25. Preserve its
    # texel classification too; filtering across edges remains approximate.
    for threshold in (63.75, 127.5):
        crosses = (values[None, :, None] >= threshold) != (palettes[:, None, :] >= threshold)
        errors[crosses] = 1e12
    indices = errors.argmin(axis=2).astype(np.uint64)
    costs = errors.min(axis=2)
    fixed = bytearray(raw)
    fixed[84:88] = b'DXT5'
    offset, report = 128, []
    for mip in range(mips):
        count = ((width+3)//4)*((height+3)//4)
        blocks = np.frombuffer(raw, np.uint8, count=count*16, offset=offset).reshape(-1, 16)
        alpha = np.stack((blocks[:, :8] & 15, blocks[:, :8] >> 4), axis=-1).reshape(-1, 16)
        assert np.all((alpha <= 7) | (alpha == 15))
        output = blocks.copy()
        absolute_sum, squared_sum, maximum, changed, crossings, authored_crossings = 0., 0., 0., 0, 0, 0
        for begin in range(0, count, 4096):
            part = alpha[begin:begin+4096]
            histogram = np.stack([(part == value).sum(axis=1) for value in range(16)], axis=1)
            best = (histogram @ costs.T).argmin(axis=1)
            encoded_indices = indices[best[:, None], part]
            encoded = np.sum(encoded_indices << (np.arange(16, dtype=np.uint64)*3), axis=1)
            output[begin:begin+len(part), :2] = np.array(endpoints, dtype=np.uint8)[best]
            output[begin:begin+len(part), 2:8] = ((encoded[:, None] >> (np.arange(6, dtype=np.uint64)*8)) & 255).astype(np.uint8)
            decoded = palettes[best[:, None], encoded_indices]
            difference = decoded-part.astype(float)*17
            absolute_sum += np.abs(difference).sum()
            squared_sum += (difference**2).sum()
            maximum = max(maximum, float(np.abs(difference).max()))
            changed += int((np.abs(difference) > 1e-9).sum())
            crossings += int(((decoded >= 127.5) != (part >= 8)).sum())
            authored_crossings += int(((decoded >= 63.75) != (part >= 4)).sum())
        assert np.array_equal(output[:, 8:], blocks[:, 8:]) and crossings == authored_crossings == 0
        fixed[offset:offset+count*16] = output.tobytes()
        report.append({'mip': mip, 'width': width, 'height': height,
                       'rgb_blocks_identical': True, 'alpha_half_threshold_crossings': crossings,
                       'alpha_authored_quarter_threshold_crossings': authored_crossings,
                       'alpha_changed_texels': changed, 'alpha_mae_255': absolute_sum/(count*16),
                       'alpha_rms_255': (squared_sum/(count*16))**.5, 'alpha_max_error_255': maximum})
        offset += count*16
        width, height = max(1, width//2), max(1, height//2)
    assert offset == len(raw)-1 and fixed[-1] == raw[-1]
    return bytes(fixed), report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with args.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    entries = []
    for name, expected in SOURCE_HASHES.items():
        resource = f'textures/ported/v010w/character/human/remains/{name}.dds'
        baseline = args.output / f'{name}-baseline.dds'
        fixed_path = args.output / f'{name}-fixed.dds'
        subprocess.run([str(args.porter), 'extract', '--db', str(args.archive), '--path', resource,
                        '--output', str(baseline)], check=True, stdout=subprocess.DEVNULL)
        raw = baseline.read_bytes()
        assert hashlib.sha256(raw).hexdigest() == expected
        fixed, report = transcode(raw)
        fixed_path.write_bytes(fixed)
        stat = subprocess.check_output([str(args.porter), 'stat', '--db', str(args.archive),
                                        '--path', resource], text=True)
        assert 'compressed: false, inlined: false' in stat
        position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
        needle = struct.pack('<I4xQ', len(raw), position)
        found = fat.find(needle)
        assert found >= 0 and fat.find(needle, found+1) < 0
        entries.append({'resource': resource, 'position': position, 'crc_offset': found+40,
                        'baseline': baseline.name, 'fixed': fixed_path.name,
                        'baseline_sha256': expected, 'fixed_sha256': hashlib.sha256(fixed).hexdigest(),
                        'mips': report})
    plan = {'scope': 'Factory day remains only. BC2 to supported BC3 streaming format; RGB blocks and sRGB '
                     'unchanged. Alpha interpolation differs slightly. Candidate requires runtime SRV and image '
                     'validation. Stop client before applying/reverting.', 'entries': entries}
    (args.output/'patch.json').write_text(json.dumps(plan, indent=2)+'\n')
    print(json.dumps({'plan': str(args.output/'patch.json'), 'textures': len(entries)}))


if __name__ == '__main__':
    main()
