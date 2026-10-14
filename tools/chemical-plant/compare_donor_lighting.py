#!/usr/bin/env python3
"""Compare authored Chemical lighting by object identity, with decoded evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from audit_collision_region import config


def changes(old, new, path=''):
    if isinstance(old, dict) and isinstance(new, dict):
        for key in sorted(old.keys() | new.keys()):
            yield from changes(old.get(key), new.get(key), f'{path}.{key}'.lstrip('.'))
    elif old != new:
        yield {'path': path, 'old': old, 'new': new}


def compare(porter, old_db, new_db, output):
    output.mkdir(parents=True, exist_ok=True)
    records = {}
    for label, db in [('old', old_db), ('new', new_db)]:
        def extract(resource):
            destination = output / label / resource
            destination.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run([str(porter), 'extract', '--db', str(db), '--path', resource,
                            '--output', str(destination)], check=True, capture_output=True)
            data = destination.read_bytes()
            decoded = config(data)
            destination.with_name(destination.name + '.json').write_text(json.dumps(decoded, indent=2)+'\n')
            return decoded, hashlib.sha256(data).hexdigest()

        project, project_hash = extract('projects/level_04/client_project')
        kinds = {'light', 'environment', 'sky', 'environment_probe'}
        objects = [obj for obj in project['objects'] if obj['game_object_type'] in kinds]
        identities = {obj['full_name']: obj for obj in objects}
        if len(identities) != len(objects):
            raise ValueError('duplicate lighting object identity')
        environment = [obj for obj in objects if obj['game_object_type'] == 'environment']
        if len(environment) != 1:
            raise ValueError('expected exactly one environment')
        post_path = 'material_instances/' + environment[0]['post_effect'] + '.material'
        post, post_hash = extract(post_path)
        records[label] = dict(archive=str(db), project_sha256=project_hash,
                              objects=identities, post_path=post_path, post_sha256=post_hash,
                              post=post)
    old, new = records['old'], records['new']
    keys = old['objects'].keys() | new['objects'].keys()
    report = dict(inputs=records, object_changes={
        key: list(changes(old['objects'].get(key), new['objects'].get(key)))
        for key in sorted(keys) if old['objects'].get(key) != new['objects'].get(key)},
        post_changes=list(changes(old['post'], new['post'])))
    (output/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(dict(changed_objects=len(report['object_changes']),
                          post_changes=len(report['post_changes']), report=str(output/'comparison.json'))))


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('porter', type=Path)
    ap.add_argument('old_db', type=Path)
    ap.add_argument('new_db', type=Path)
    ap.add_argument('output', type=Path)
    args = ap.parse_args()
    compare(args.porter, args.old_db, args.new_db, args.output)
