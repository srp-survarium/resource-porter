#!/usr/bin/env python3
"""Prepare (never apply) the three source-backed Factory day material repairs."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess


CHANGES = {
    'characters/human/remains/skeleton': {'use_tspecular_intensity': (False, True)},
    'characters/human/remains/rag': {'use_tspecular_intensity': (False, True)},
    'tiled/metal/metal_radio': {'use_tfresnel': (False, True)},
}


def leaf(data, path):
    offset = 0
    for part in path.split('.'):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        assert kind == 3, path
        matches = []
        for index in range(count):
            child = table + 24 * index
            name = struct.unpack_from('<Q', data, child + 8)[0]
            if data[name:data.index(0, name)].decode() == part:
                matches.append(child)
        assert len(matches) == 1, path
        offset = matches[0]
    return offset


def boolean(data, path):
    offset = leaf(data, path)
    assert struct.unpack_from('<H', data, offset + 20)[0] == 0, path
    value = struct.unpack_from('<Q', data, offset)[0]
    assert value in (0, 1), path
    return offset, bool(value)


def prepare(args):
    args.output.mkdir(parents=True, exist_ok=True)
    with args.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    entries = []
    for index, (name, switches) in enumerate(CHANGES.items()):
        resource = f'material_instances/ported/v010w/{name}.material'
        source = args.output / f'{index}-source.material'
        baseline = args.output / f'{index}-baseline.material'
        fixed_path = args.output / f'{index}-fixed.material'
        for db, path, output in [(args.source, f'material_instances/{name}.material', source),
                                 (args.archive, resource, baseline)]:
            subprocess.run([str(args.porter), 'extract', '--db', str(db), '--path', path,
                            '--output', str(output)], check=True, stdout=subprocess.DEVNULL)
        original, raw = source.read_bytes(), baseline.read_bytes()
        fixed, changes = bytearray(raw), []
        for switch, (before, after) in switches.items():
            path = f'material.g_stage.effect.{switch}.value'
            offset, current = boolean(raw, path)
            _, authored = boolean(original, path)
            assert current == before and authored == after, (resource, path, current, authored)
            struct.pack_into('<Q', fixed, offset, int(authored))
            changes.append({'path': path, 'offset': offset, 'before': current, 'after': authored})
        if name.startswith('characters/human/remains/'):
            # The native alpha-test + intensity-mask + constant-roughness PS is
            # absent. Its otherwise-identical textured-roughness PS is shipped.
            # A zero-width authored range yields the same constant without
            # adding a shader or altering the texture's channels/options.
            path = 'material.g_stage.effect.use_troughness.value'
            assert boolean(raw, path)[1] and not boolean(original, path)[1]
            constant_path = 'material.g_stage.effect.constant_roughness.value'
            constant_offset = leaf(original, constant_path)
            assert struct.unpack_from('<H', original, constant_offset + 20)[0] == 2
            constant = struct.unpack_from('<f', original, constant_offset)[0]
            assert constant == .5
            for field, expected in [('constant_roughness_min', .1), ('constant_roughness_max', .2)]:
                path = f'material.g_stage.effect.{field}.value'
                offset = leaf(raw, path)
                assert struct.unpack_from('<H', raw, offset + 20)[0] == 2
                current = struct.unpack_from('<f', raw, offset)[0]
                assert abs(current - expected) < 1e-7
                struct.pack_into('<f', fixed, offset, constant)
                changes.append({'path': path, 'offset': offset, 'before': current, 'after': constant,
                                'source_path': constant_path,
                                'reason': 'zero-width range preserves authored constant with shipped shader'})
        fixed_path.write_bytes(fixed)
        stat = subprocess.check_output([str(args.porter), 'stat', '--db', str(args.archive),
                                        '--path', resource], text=True)
        assert 'compressed: false, inlined: false' in stat
        position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
        needle = struct.pack('<I4xQ', len(raw), position)
        found = fat.find(needle)
        assert found >= 0 and fat.find(needle, found + 1) < 0
        entries.append({'resource': resource, 'position': position, 'crc_offset': found + 40,
                        'baseline': baseline.name, 'fixed': fixed_path.name,
                        'baseline_sha256': hashlib.sha256(raw).hexdigest(),
                        'fixed_sha256': hashlib.sha256(fixed).hexdigest(),
                        'source': source.name, 'source_sha256': hashlib.sha256(original).hexdigest(),
                        'changes': changes})
    report = {'scope': 'Factory 0.10b day only; three source feature switches and four range endpoints '
                       'set to the authored constant roughness in three materials. No texture, lighting '
                       'or other namespace changes. Stop client before apply/revert.',
              'entries': entries}
    (args.output / 'patch.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'materials': len(entries), 'fields': sum(len(e['changes']) for e in entries),
                      'plan': str(args.output / 'patch.json')}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    prepare(parser.parse_args())
