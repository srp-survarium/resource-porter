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
    'characters/human/remains/skeleton': {'use_troughness': (True, False),
                                          'use_tspecular_intensity': (False, True)},
    'characters/human/remains/rag': {'use_troughness': (True, False),
                                     'use_tspecular_intensity': (False, True)},
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
        for name, (before, after) in switches.items():
            path = f'material.g_stage.effect.{name}.value'
            offset, current = boolean(raw, path)
            _, authored = boolean(original, path)
            assert current == before and authored == after, (resource, path, current, authored)
            struct.pack_into('<Q', fixed, offset, int(authored))
            changes.append({'path': path, 'offset': offset, 'before': current, 'after': authored})
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
    report = {'scope': 'Factory 0.10b day only; five authored feature switches in three materials. '
                       'No texture, scalar, lighting or other namespace changes. Stop client before apply/revert.',
              'entries': entries}
    (args.output / 'patch.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'materials': len(entries), 'switches': sum(len(e['changes']) for e in entries),
                      'plan': str(args.output / 'patch.json')}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    prepare(parser.parse_args())
