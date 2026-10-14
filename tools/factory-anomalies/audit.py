#!/usr/bin/env python3
"""Inventory authored anomaly behavior and scene links from an extracted project.

This is a data audit, not evidence of runtime damage or synchronization.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import config

KINDS = {'generic_anomaly_core', 'damage_zone', 'artefact_container'}


def audit(path):
    data = path.read_bytes()
    project = config(data)
    objects = project['client_objects']
    names = {obj['full_name']: obj for obj in objects if 'full_name' in obj}
    errors = []
    def require(name, kind):
        if names.get(name, {}).get('game_object_type') != kind:
            errors.append({'name': name, 'expected_type': kind})
    anomalies = []
    for obj in objects:
        kind = obj['game_object_type']
        if kind not in KINDS:
            continue
        if kind != 'generic_anomaly_core':
            for geometry in obj.get('collision_geometries', []):
                require(geometry['name'], 'collision_geometry')
            continue
        zones = []
        for state in obj['states']:
            for group in state['groups']:
                zones.extend(group['zones'])
        containers = obj.get('artefact_containers', []) if obj['artefacts_enabled'] else []
        for name in zones:
            require(name, 'damage_zone')
        for name in containers:
            require(name, 'artefact_container')
        def zone_info(name):
            z = names.get(name, {})
            keys = ('position', 'damage_type', 'max_hit', 'continuous_hit',
                    'interval_in_msec', 'hit_curve', 'speed_curve',
                    'collision_geometries', 'effect_zones', 'sound_zones')
            return {'name': name, **{k: v for k, v in z.items() if k in keys or 'particle' in k or 'sound' in k}}
        anomalies.append({'controller': obj,
                          'zones': [zone_info(n) for n in dict.fromkeys(zones)],
                          'containers': [names.get(n, {'full_name': n}) for n in containers]})
    selected = [o for o in objects if o['game_object_type'] in KINDS]
    return {'project': str(path), 'sha256': hashlib.sha256(data).hexdigest(),
            'counts': dict(Counter(o['game_object_type'] for o in selected)),
            'link_errors': errors, 'anomalies': anomalies}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--converted-project', type=Path, help='Check preservation of authored anomaly objects')
    args = parser.parse_args()
    result = audit(args.project)
    if args.converted_project:
        original = config(args.project.read_bytes())
        converted = config(args.converted_project.read_bytes())
        def normalize(value):
            if isinstance(value, str):
                return value.replace('ported/v020e/', '')
            if isinstance(value, list):
                return [normalize(x) for x in value]
            if isinstance(value, dict):
                return {k: normalize(v) for k, v in value.items() if k != 'is_temporary'}
            return value
        def relevant(project):
            return {o['full_name']: normalize(o) for o in project['client_objects']
                    if o['game_object_type'] in KINDS}
        a, b = relevant(original), relevant(converted)
        changed = [name for name in sorted(a.keys() | b.keys()) if a.get(name) != b.get(name)]
        result['converted_comparison'] = {'objects': len(a), 'changed_objects': changed}
        result['link_errors'].extend(audit(args.converted_project)['link_errors'])
        result['link_errors'].extend({'changed_authored_object': n} for n in changed)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'counts': result['counts'], 'link_errors': result['link_errors']}))
    return bool(result['link_errors'])


if __name__ == '__main__':
    raise SystemExit(main())
