#!/usr/bin/env python3
"""Match captured native local-light uploads to original project light objects."""
import argparse
import json
import math
import struct
from pathlib import Path

def constants(path):
    code = path.read_bytes()
    if code[:4] != b'DXBC':
        raise ValueError(path)
    count = struct.unpack_from('<I', code, 28)[0]
    offsets = struct.unpack_from('<' + 'I' * count, code, 32)
    offset = next(o for o in offsets if code[o:o + 4] == b'RDEF')
    size = struct.unpack_from('<I', code, offset + 4)[0]
    data = code[offset + 8:offset + 8 + size]
    def name(at):
        return data[at:data.index(0, at)].decode()
    count, offset = struct.unpack_from('<II', data, 8)
    slots = {}
    for i in range(count):
        entry = struct.unpack_from('<8I', data, offset + i * 32)
        if entry[1] == 0:
            slots[name(entry[0])] = entry[5]
    count, offset = struct.unpack_from('<II', data)
    result = {}
    for i in range(count):
        label, variables, begin = struct.unpack_from('<III', data, offset + i * 24)
        raw = path.with_suffix(f'.cb{slots[name(label)]}.bin').read_bytes()
        for j in range(variables):
            label, at, length, flags = struct.unpack_from('<IIII', data, begin + j * 24)
            if flags & 2:
                result[name(label)] = struct.unpack_from('<' + 'f' * (length // 4), raw, at)
    return code[4:20].hex(), result

def authored_lights(value):
    if isinstance(value, dict):
        if all(key in value for key in ('light_type', 'position', 'color', 'range', 'intensity')):
            yield value
        else:
            for child in value.values():
                yield from authored_lights(child)
    elif isinstance(value, list):
        for child in value:
            yield from authored_lights(child)

def audit(project, directory):
    lights = list(authored_lights(json.loads(project.read_text())))
    if not lights:
        raise ValueError('no authored light objects')
    rows = []
    for path in sorted(directory.glob('*.local-light-draw-*.dxbc')):
        if path.name.endswith('.vs.dxbc'):
            continue
        digest, values = constants(path)
        if digest != '33204e907519b2ad0afa798e6154aebc':
            raise ValueError('unverified active spot identity: ' + str(path))
        matrix, position = values['m_V2W'], values['light_position']
        world = [sum(matrix[i * 4 + j] * position[j] for j in range(3)) + matrix[i * 4 + 3] for i in range(3)]
        light = min(lights, key=lambda light: math.dist(light['position'], world))
        cosine = values['light_spot_penumbra_half_angle_cosine'][0]
        expected_range = light['range'] / math.sqrt(1 - cosine * cosine)
        row = {'draw': path.name, 'digest': digest, 'light': light.get('name'), 'world': world,
               'position_error': math.dist(world, light['position']),
               'linear_color_error': max(abs((round(c * 255) / 255) ** 2.2 - v) for c, v in zip(light['color'], values['light_color'])),
               'range_error': abs(expected_range - values['light_range'][0]),
               'intensity_error': abs(light['intensity'] - values['light_intensity'][0]),
               'attenuation_error': abs(light['attenuation_power'] - values['light_attenuation_power'][0]),
               'diffuse_influence': light['diffuse_influence_factor']}
        if row['position_error'] > .001 or row['diffuse_influence'] != 1:
            raise ValueError('source spot mapping requires review: ' + str(row))
        rows.append(row)
    if not rows:
        raise ValueError('no active local-light captures')
    fields = ('position_error', 'linear_color_error', 'range_error', 'intensity_error', 'attenuation_error')
    return {'draws': rows, 'maximum_errors': {field: max(r[field] for r in rows) for field in fields},
            'scope': 'Upload contracts; source/native BRDF and shadow sampling differ.'}

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('captures', type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.project, args.captures), indent=2))
