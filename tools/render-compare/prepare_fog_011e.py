#!/usr/bin/env python3
"""Prepare captured, static weather inputs for the original-fog visual experiment."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct


def prepare(draw, output):
    if output.exists() or not str(draw).endswith('.fog-add-draw'):
        raise ValueError('expected fog-add capture and a new output directory')
    hashes = {}

    def read(path):
        data = path.read_bytes()
        hashes[str(path.resolve())] = hashlib.sha256(data).hexdigest()
        return data

    def part(suffix):
        return read(Path(str(draw) + suffix))

    shaders = {}
    for name, digest in (
        ('add', 'ee83aa2c2684ae2b02c7c03f37d45e34c8f365011b41db1b9e9047988ed82f23'),
        ('mul', '49e6272166ee7f16a9fe1b1da8a0590e85de07e2964479267dfff1608d7dba4a'),
    ):
        path = Path(str(draw).replace('.fog-add-draw', f'.fog-{name}-draw') + '.dxbc')
        data = read(path)
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError('shader is not the original 0.11e fog program')
        shaders[f'fog-{name}.dxbc'] = data
    constants = part('.cb0.bin')
    if len(constants) != 128:
        raise ValueError('unexpected source fog constant layout')
    parameters = constants[80:112]
    values = struct.unpack('<8f', parameters)
    if not all(math.isfinite(v) for v in values) or values[4] <= 0 or values[5] <= 0:
        raise ValueError('invalid source atmosphere parameters')
    assets = dict(shaders, **{'parameters.bin': parameters})
    targets = {row['slot']: row for row in json.loads(part('.targets.json'))}
    for slot, name in ((1, 'rayleigh.bin'), (2, 'mie.bin')):
        target = targets[slot]
        expected = dict(width=256, height=128, format=10, view_format=10, mip_levels=1, array_size=1, written=True)
        if any(target.get(k) != v for k, v in expected.items()):
            raise ValueError('unexpected source atmosphere table format')
        data = part(f'.rt{slot:02d}.bin')
        if len(data) != 256 * 128 * 8:
            raise ValueError('invalid atmosphere table contents')
        # Both original weather captures contain half NaNs in the last two
        # Mie columns' RGB. Preserve those captured bytes; do not "repair" the
        # source fixture. No other nonfinite table values are supported.
        for index, (value,) in enumerate(struct.iter_unpack('<e', data)):
            if not math.isfinite(value) and not (
                slot == 2 and (index // 4) % 256 >= 254 and index % 4 < 3
                and struct.unpack_from('<H', data, index * 2)[0] == 0x7fff
            ):
                raise ValueError('unexpected nonfinite atmosphere table value')
        assets[name] = data
    output.mkdir(parents=True)
    for name, data in assets.items():
        (output / name).write_bytes(data)
    receipt = dict(scope='Captured weather LUTs and original fog shaders for visual A/B; not production assets.',
                   inputs=hashes, assets={name: hashlib.sha256(data).hexdigest() for name, data in assets.items()},
                   sun_and_inscatter=values,
                   source_table_note='Original Mie RGB NaNs in columns 254/255 are preserved.')
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('draw', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(json.dumps(prepare(args.draw, args.output), indent=2))
