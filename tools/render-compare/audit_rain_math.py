#!/usr/bin/env python3
"""Replay frozen original rain transforms through Python and the host C helper."""
import argparse
import ctypes
import importlib.util
import json
import math
from pathlib import Path
import sys


class Motion(ctypes.Structure):
    _fields_ = [('previous', ctypes.c_double*3), ('offsets', ctypes.c_double*2), ('initialized', ctypes.c_int)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path, default=Path(__file__).with_name('fixtures')/'factory_rain_motion.json')
    parser.add_argument('--motion-tools', type=Path, required=True)
    parser.add_argument('--c-math-library', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    sys.path.insert(0, str(args.motion_tools))
    spec = importlib.util.spec_from_file_location('rain_motion_review', args.motion_tools/'legacy_rain_motion.py')
    helper = importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)
    library = ctypes.CDLL(str(args.c_math_library)); pointer = ctypes.POINTER(ctypes.c_float)
    library.layers.argtypes = [ctypes.c_float, pointer, pointer, pointer]
    library.advance.argtypes = [ctypes.POINTER(Motion), pointer, pointer, pointer]
    vector = lambda values: (ctypes.c_float*len(values))(*values)
    fixture = json.loads(args.fixture.read_text()); state = Motion(); reports = []
    for pose in fixture['poses']:
        origin = [row[3] for row in pose['layers'][0]['world_rows']]
        world, radii = vector([0]*60), vector([0]*5)
        library.layers(pose['scene_time'], vector(origin), world, radii)
        actual = [v for layer in pose['layers'] for row in layer['world_rows'] for v in row]
        python = [v for layer in helper.factory_layers(pose['scene_time'], origin) for row in layer['world_rows'] for v in row]
        assert list(world) == list(vector(python))
        matrix_error = max(abs(a-b) for a,b in zip(actual, world))
        radius_error = max(abs(layer['radius']-radii[i]) for i,layer in enumerate(pose['layers']))
        assert matrix_error < fixture['matrix_tolerance'] and radius_error < fixture['radius_tolerance']
        eye = list(vector(pose['camera_position'])); view = pose['view_rows']
        if not state.initialized:
            delta = [eye[0]-origin[0], eye[2]-origin[2]]
            for i,axis in enumerate([view[8:11], view[:3]]):
                state.offsets[i] = (delta[0]*axis[0]+delta[1]*axis[2])/math.hypot(axis[0], axis[2])
            state.previous[:] = eye; state.initialized = 1
        predicted_origin = vector([0]*3)
        library.advance(ctypes.byref(state), vector(eye), vector(view), predicted_origin)
        # A stationary second update handles source frac's negative-integer edge.
        library.advance(ctypes.byref(state), vector(eye), vector(view), predicted_origin)
        library.layers(pose['scene_time'], predicted_origin, world, radii)
        sequence_error = max(abs(a-b) for a,b in zip(actual, world))
        assert sequence_error < fixture['matrix_tolerance']
        reports.append({'pose': pose['name'], 'layer_matrix_error': matrix_error,
                        'radius_error': radius_error, 'sequence_matrix_error': sequence_error,
                        'sequence_origin_error': max(abs(a-b) for a,b in zip(origin, predicted_origin))})
    result = {'scope': fixture['scope'], 'poses': reports,
              'captured_matrices': sum(len(p['layers']) for p in fixture['poses'])}
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print('Verified', result['captured_matrices'], 'matrices and', len(reports), 'sequential poses')


if __name__ == '__main__':
    main()
