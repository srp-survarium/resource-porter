#!/usr/bin/env python3
"""Compare authored caster resources around the frozen Radar020 stair view.

This checks conversion, not runtime caster selection or shadow-map visibility.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import config


MODELS = ('level/03/antenna/antenna_base', 'level/03/antenna_tree',
          'level/03/antenna_tree_leaves2')


def normalized(value):
    if isinstance(value, str):
        return value.replace('ported/v020e/', '')
    if isinstance(value, list):
        return [normalized(v) for v in value]
    if isinstance(value, dict):
        return {k: normalized(v) for k, v in value.items()}
    return value


def shadow_fields(value, prefix=''):
    fields = {}
    for key, item in value.items():
        path = prefix + key
        if isinstance(item, dict):
            fields.update(shadow_fields(item, path + '.'))
        elif (path.endswith('.value') and
              any(word in path for word in ('shadow', 'alpha', 'transparency', 'two_sided'))):
            fields[path] = item
    return fields


def audit(source, overlay):
    report = {'scope': __doc__.strip(), 'pose': {'position': [-8, 7.6, -13],
              'forward': [0, .12, -1], 'fov': 65}, 'models': {}, 'materials': {}}
    materials = set()
    for model in MODELS:
        old = source / 'models' / (model + '.model')
        new = overlay / 'models/ported/v020e' / (model + '.model')
        original = config((old / 'settings').read_bytes())
        converted = config((new / 'settings').read_bytes())
        if normalized(converted) != original:
            raise ValueError('model settings changed: ' + model)
        geometry = {}
        for path in sorted(old.glob('render/**/converted_model')):
            data = path.read_bytes()
            relative = path.relative_to(old)
            if (new / relative).read_bytes() != data:
                raise ValueError('geometry changed: ' + str(path))
            geometry[str(relative)] = {'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        if not geometry:
            raise ValueError('no render geometry: ' + model)
        report['models'][model] = {'settings_equal_after_namespace': True, 'geometry': geometry}
        materials.update(item['material_name'] for item in original['material_settings'].values())
    for material in sorted(materials):
        old = source / 'material_instances' / (material + '.material')
        new = overlay / 'material_instances/ported/v020e' / (material + '.material')
        original = shadow_fields(config(old.read_bytes()))
        converted = normalized(shadow_fields(config(new.read_bytes())))
        mismatches = {k: [v, converted.get(k)] for k, v in original.items() if converted.get(k) != v}
        report['materials'][material] = {'source_fields': original, 'changed_fields': mismatches,
            'source_sha256': hashlib.sha256(old.read_bytes()).hexdigest(),
            'converted_sha256': hashlib.sha256(new.read_bytes()).hexdigest(),
            'target_only_fields': {k: v for k, v in converted.items() if k not in original}}
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('overlay', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = audit(args.source, args.overlay)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'models': len(result['models']), 'materials': len(result['materials']),
                      'changed_materials': [name for name, m in result['materials'].items() if m['changed_fields']],
                      'surfaces': sum(len(m['geometry']) for m in result['models'].values())}))
