#!/usr/bin/env python3
"""Original Factory rain-shell rotation, checked against live 0.10b draws.

The input origin includes stage_rain's camera-motion offsets. This module does
not replace that stateful calculation or the renderer's lighting/shadow inputs.
"""
import argparse
import json
import math
import struct
from pathlib import Path
from audit_reference_rain import audit


def random_values(seed=1000):
    while True:
        seed = (0x08088405 * seed + 1) & 0xffffffff
        yield (seed >> 12) / 1048576


def base_offsets():
    random = random_values()
    # The original compiled float2 constructor evaluates its second argument
    # first. All 35 captured transforms confirm this order, not textual x/y order.
    result = []
    for _ in range(100):
        y = next(random) * 2 - 1
        x = next(random) * 2 - 1
        result.append((x, y))
    return result


def multiply(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(3)) for c in range(3)] for r in range(3)]


def camera_origin(position, forward, right, previous, offsets):
    """Advance the original horizontal camera-relative rain scroll state."""
    def horizontal(vector):
        length = math.hypot(vector[0], vector[2])
        return [vector[0]/length, vector[2]/length] if length else [0, 0]

    axes = [horizontal(forward), horizontal(right)]
    delta = [position[0]-previous[0], position[2]-previous[2]]
    result = []
    for offset, axis in zip(offsets, axes, strict=True):
        offset += sum(a*b for a,b in zip(delta,axis,strict=True))*.5
        # help_math::frac uses magnitude/truncation, not the usual signed frac.
        if offset >= 1:
            offset -= math.trunc(offset)
        elif offset <= -1:
            offset = 1-(abs(offset)-abs(math.trunc(offset)))
        result.append(offset)
    origin = [position[0]-sum(result[i]*axes[i][0] for i in range(2)), position[1],
              position[2]-sum(result[i]*axes[i][1] for i in range(2))]
    return origin, result


def factory_layers(scene_time, origin):
    """Five authored layers; matrices are uploaded row-major float3x4 m_W."""
    if len(origin) != 3 or not all(math.isfinite(x) for x in [scene_time, *origin]):
        raise ValueError('finite scene time and three-component origin required')
    offsets, random = base_offsets(), random_values()
    multiplier = 1.0
    layers = []
    for index in range(5, 10):
        radius = index * multiplier
        scale = 2 * radius
        yaw = next(random) * math.tau
        tilt_x = math.cos(scene_time * .2) * .1 + offsets[index][0] * .1
        tilt_z = math.sin(scene_time * .2) * .1 + offsets[index][1] * .1
        cx, sx = math.cos(tilt_x), math.sin(tilt_x)
        cy, sy = math.cos(yaw), math.sin(yaw)
        cz, sz = math.cos(tilt_z), math.sin(tilt_z)
        rotation = multiply(multiply([[cz, sz, 0], [-sz, cz, 0], [0, 0, 1]],
                                    [[1, 0, 0], [0, cx, sx], [0, -sx, cx]]),
                            [[cy, 0, -sy], [0, 1, 0], [sy, 0, cy]])
        world = [[rotation[r][c] * scale * (10 if c == 1 else 1) for c in range(3)] + [origin[r]]
                 for r in range(3)]
        layers.append({'index': index, 'radius': radius, 'world_rows': world})
        multiplier *= 1.1
    return layers


def verify_motion(report):
    capture = audit(report)
    if len(capture['layers']) != 5:
        raise ValueError('expected five Factory source layers')
    prefix = str(report).removesuffix('.rain-counts.json')
    scene_time = struct.unpack_from('<f', Path(prefix + '.rain-draw.cb1.bin').read_bytes(), 284)[0]
    origin = [row[3] for row in capture['layers'][0]['world_rows']]
    expected = factory_layers(scene_time, origin)
    errors = []
    for actual, prediction in zip(capture['layers'], expected, strict=True):
        if any(abs(actual[key] - value) > 1e-5 for key, value in [('density', .8), ('speed', 2)]):
            raise ValueError('different weather settings')
        if actual['uv_scale'] != [1, 1] or abs(actual['radius'] - prediction['radius']) > 1e-4:
            raise ValueError('different layer scaling')
        error = max(abs(a - b) for row_a, row_b in zip(actual['world_rows'], prediction['world_rows'], strict=True)
                    for a, b in zip(row_a, row_b, strict=True))
        if error > .0002:
            raise ValueError(f'{report}: original motion mismatch {error}')
        errors.append(error)
    return {'report': str(report), 'scene_time': scene_time, 'max_matrix_error': max(errors),
            'layer_errors': errors,
            'scope': 'Source scale/rotation/clock replay; origin supplied from capture, camera-motion offsets not validated.'}


def verify_sequence(results_path):
    records = json.loads(results_path.read_text())
    previous, offsets, results = None, None, []
    for record in records:
        prefix = str(results_path.parent/'poses'/(record['pose']['name']+'.bmp'))
        capture = audit(Path(prefix+'.rain-counts.json'))
        constants = Path(prefix+'.rain-draw.cb1.bin').read_bytes()
        view = struct.unpack_from('<12f', constants, 128)
        scene_time = struct.unpack_from('<f', constants, 284)[0]
        position = record['status']['position']
        forward, right = view[8:11], view[:3]
        actual_origin = [row[3] for row in capture['layers'][0]['world_rows']]
        if previous is None:
            delta = [position[0]-actual_origin[0],position[2]-actual_origin[2]]
            offsets = [sum(a*b for a,b in zip(delta,[axis[0],axis[2]],strict=True))/math.hypot(axis[0],axis[2])
                       for axis in [forward,right]]
            previous = position
        origin, offsets = camera_origin(position,forward,right,previous,offsets)
        # Captures settle for many frames; a negative integer offset becomes
        # one first, then zero on the next stationary frame in the source.
        origin, offsets = camera_origin(position,forward,right,position,offsets)
        predicted = factory_layers(scene_time,origin)
        error = max(abs(a-b) for actual,expected in zip(capture['layers'],predicted,strict=True)
                    for row_a,row_b in zip(actual['world_rows'],expected['world_rows'],strict=True)
                    for a,b in zip(row_a,row_b,strict=True))
        if error > .0002:
            raise ValueError(f'{prefix}: sequential motion mismatch {error}')
        results.append({'pose':record['pose']['name'],'max_matrix_error':error,'offsets':offsets})
        previous = position
    return {'views':results,'scope':'Full source matrix replay across captured pose transitions; initial scroll state inferred from first capture.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reports', nargs='*', type=Path)
    parser.add_argument('--sequence-results',type=Path)
    args = parser.parse_args()
    if not args.reports and not args.sequence_results:
        parser.error('provide rain reports or --sequence-results')
    output = {'draws':[verify_motion(report) for report in args.reports]}
    if args.sequence_results:
        output['sequence']=verify_sequence(args.sequence_results)
    print(json.dumps(output, indent=2))
