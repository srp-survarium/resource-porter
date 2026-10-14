#!/usr/bin/env python3
"""Bake only the source fog fields which native cloud metadata does not upload."""
import argparse
import hashlib
import json
import math
from pathlib import Path

from prepare_cloud_lighting_patch import SOURCE_SHA, config


def prepare(source, output):
    data = source.read_bytes()
    if hashlib.sha256(data).hexdigest() != SOURCE_SHA:
        raise ValueError('not the pinned Radar020 donor project')
    environments = [o['properties'] for o in config(data)['client_objects']
                    if o.get('game_object_type') == 'environment']
    if len(environments) != 1:
        raise ValueError('expected one authored environment')
    weather = environments[0]
    fields = {'SOURCE_CLOUD_FOG_POWER': 'sky_clouds_fog_power',
              'SOURCE_CLOUD_FOG_UP_LIMIT': 'sky_clouds_fog_up_limit'}
    values = {macro: float(weather[key]) for macro, key in fields.items()}
    if not all(math.isfinite(value) and value > 0 for value in values.values()):
        raise ValueError('invalid authored cloud fog fields')
    template = Path(__file__).resolve().parents[1]/'render-compare/shaders/factory_020e_clouds.hlsl'
    declarations = ''.join(f'#define {macro} {value:.9g}\n' for macro, value in values.items())
    output.mkdir(exist_ok=False)
    shader = output/'prepared.hlsl'
    shader.write_text(declarations+'\n'+template.read_text())
    report = dict(source_sha256=SOURCE_SHA, authored_fields={key: weather[key] for key in fields.values()},
                  template_sha256=hashlib.sha256(template.read_bytes()).hexdigest(),
                  prepared_sha256=hashlib.sha256(shader.read_bytes()).hexdigest(),
                  reason='native shader metadata omits unused fog row; color and fog color remain uniforms')
    (output/'manifest.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    prepare(args.source, args.output)
