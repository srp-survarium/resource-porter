#!/usr/bin/env python3
"""Prepare a reversible Factory day water-foam patch without copying the database."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

RESOURCE = 'material_instances/ported/v010w/water/water_green.material'
SOURCE = 'material_instances/water/water_green.material'
KEY = 'material.forward.effect.foam_intensity.value'


def prepare(exe, archive, source_db, output):
    output.mkdir(parents=True, exist_ok=True)
    def leaves(db, path):
        result = subprocess.check_output([str(exe), 'dump-db-config', '--db', str(db),
                                          '--path', path, '--leaves'], text=True)
        return dict(line.split('\t', 1) for line in result.splitlines())
    original, current = leaves(source_db, SOURCE), leaves(archive, RESOURCE)
    effect = 'material.forward.effect.effect_id'
    assert original[effect] == current[effect] == 'string:"forward_default_simpe_water"'
    assert KEY not in original and current[KEY] == 'i32:1'
    baseline, fixedpath = output/'baseline.material', output/'fixed.material'
    subprocess.run([str(exe), 'extract', '--db', str(archive), '--path', RESOURCE,
                    '--output', str(baseline)], check=True, stdout=subprocess.DEVNULL)
    raw = baseline.read_bytes()
    fixed, offset = bytearray(raw), 0
    for part in KEY.split('.'):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', raw, offset)
        assert kind == 3
        found = []
        for i in range(count):
            child = table + 24*i
            name = struct.unpack_from('<Q', raw, child+8)[0]
            if raw[name:raw.index(0, name)].decode() == part:
                found.append(child)
        assert len(found) == 1
        offset = found[0]
    assert struct.unpack_from('<H', raw, offset+20)[0] == 1
    assert struct.unpack_from('<Q', raw, offset)[0] == 1
    struct.pack_into('<Q', fixed, offset, 0)
    assert sum(a != b for a, b in zip(raw, fixed)) == 1
    fixedpath.write_bytes(fixed)
    with archive.open('rb') as stream:
        header = stream.read(24)
        fat = stream.read(struct.unpack_from('<I', header, 20)[0])
    stat = subprocess.check_output([str(exe), 'stat', '--db', str(archive),
                                    '--path', RESOURCE], text=True)
    assert 'compressed: false, inlined: false' in stat
    position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
    needle = struct.pack('<I4xQ', len(raw), position)
    at = fat.find(needle)
    assert at >= 0 and fat.find(needle, at+1) < 0
    entry = dict(resource=RESOURCE, position=position, crc_offset=at+24+16,
                 baseline=baseline.name, fixed=fixedpath.name,
                 baseline_sha256=hashlib.sha256(raw).hexdigest(),
                 fixed_sha256=hashlib.sha256(fixed).hexdigest())
    (output/'patch.json').write_text(json.dumps(dict(
        scope='Factory v010w water only: disable foam absent from the source shader; preserve all other bytes.',
        entries=[entry]), indent=2)+'\n')
    (output/'contracts.json').write_text(json.dumps(dict(source=original, before=current), indent=2)+'\n')
    print(output/'patch.json')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('exe', 'archive', 'source-db', 'output'):
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    prepare(args.exe, args.archive, args.source_db, args.output)
