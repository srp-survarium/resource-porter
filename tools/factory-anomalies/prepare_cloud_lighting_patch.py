#!/usr/bin/env python3
"""Prepare a guarded Radar020 cloud-lighting-only overlay; never edit the DB."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_collision_region import config

SOURCE_SHA = 'f6a20958204063e2341678c19620beba37674a5e6a59b24158013838b263a1b2'
TARGET_SHA = '4d99a7c02e6846e82f483bed5470b655a15bf73fb1cbba570e9e665d1f77a765'
RESOURCE = 'projects/v020e_level_03/day_2/client_project'
FIELD = 'use_sky_clouds_lighting'


def prepare(source, database, porter, output):
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA:
        raise ValueError('not the pinned original020e Radar day project')
    output.mkdir(exist_ok=False)
    before = output/'before-client-project.bin'
    command = [str(porter)]
    subprocess.run(command+['extract', '--db', str(database), '--path', RESOURCE,
                            '--output', str(before)], check=True)
    original = before.read_bytes()
    if hashlib.sha256(original).hexdigest() != TARGET_SHA:
        raise ValueError('converted project changed; audit before preparing a patch')
    donor = config(source.read_bytes())
    target = config(original)
    environments = {o['full_name']: o for o in donor['client_objects']
                    if o.get('game_object_type') == 'environment'}
    expected = copy.deepcopy(target)
    patches = []
    for i, obj in enumerate(target['client_objects']):
        if obj.get('game_object_type') != 'environment':
            continue
        src = environments[obj['full_name']]['properties']
        if FIELD in src:
            continue
        if obj['properties'].get(FIELD) is not True:
            raise ValueError('expected inherited enabled cloud-lighting switch')
        expected['client_objects'][i]['properties'][FIELD] = False
        patches += ['--set-bool', f'client_objects.{i}.properties.{FIELD}=false']
    if not patches:
        raise ValueError('no inherited cloud-lighting switches')
    bundle = output/'bundle'
    subprocess.run(command+['build-config-patch', '--source-db', str(database),
        '--resource', RESOURCE, '--output', str(bundle)]+patches, check=True)
    after = (bundle/'overlay'/RESOURCE).read_bytes()
    if config(after) != expected:
        raise ValueError('patch changed unrelated project values')
    if len(after) != len(original) or sum(a != b for a, b in zip(original, after)) != len(patches)//2:
        raise ValueError('expected exactly one changed byte per boolean')
    stat = subprocess.check_output(command+['stat', '--db', str(database), '--path', RESOURCE], text=True)
    if 'compressed: false, inlined: false' not in stat:
        raise ValueError('in-place experiment requires an uncompressed standalone entry')
    position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
    with database.open('rb') as stream:
        header = stream.read(24)
        fat = stream.read(struct.unpack_from('<I', header, 20)[0])
        stream.seek(position)
        if stream.read(len(original)) != original:
            raise ValueError('archive changed during preparation')
    needle = struct.pack('<I4xQ', len(original), position)
    at = fat.find(needle)
    if at < 0 or fat.find(needle, at+1) >= 0:
        raise ValueError('archive entry is not unique')
    entry = dict(resource=RESOURCE, position=position, crc_offset=at+40,
        baseline=before.name, fixed='bundle/overlay/'+RESOURCE,
        baseline_sha256=TARGET_SHA, fixed_sha256=hashlib.sha256(after).hexdigest())
    (output/'patch.json').write_text(json.dumps(dict(entries=[entry]), indent=2)+'\n')
    report = dict(resource=RESOURCE, source_sha256=SOURCE_SHA,
        before_sha256=TARGET_SHA, after_sha256=hashlib.sha256(after).hexdigest(),
        before_size=len(original), after_size=len(after), switches=len(patches)//2,
        only_expected_values_changed=True, database_modified=False)
    (output/'guard.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'database', 'porter', 'output'):
        p.add_argument(name, type=Path)
    a = p.parse_args()
    prepare(a.source, a.database, a.porter, a.output)
