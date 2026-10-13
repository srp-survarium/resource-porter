#!/usr/bin/env python3
"""Check private source-rain draws and compare phase-independent contribution statistics."""
import argparse
import array
import ctypes
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import struct
import sys


def read_json(path):
    return json.loads(path.read_text())


def floats(path):
    data = path.read_bytes()
    return struct.unpack('<' + 'f' * (len(data) // 4), data)


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def inverse_view(values):
    # Same guarded orthonormal transpose as the reviewed runtime implementation.
    view = [values[32+i*4:36+i*4] for i in range(3)]
    error = max(abs(sum(view[r][k]*view[c][k] for k in range(3))-(r == c))
                for r in range(3) for c in range(3))
    assert error < 1e-5
    inverse = [[view[c][r] for c in range(3)] +
               [f32(-sum(f32(view[k][r]*view[k][3]) for k in range(3)))] for r in range(3)]
    inverse.append([0, 0, 0, 1])
    assert max(abs(inverse[r][3]-values[80+r]) for r in range(3)) < .001
    return inverse


def color_words(path, width, height):
    data = array.array('I')
    data.frombytes(path.read_bytes())
    if sys.byteorder != 'little':
        data.byteswap()
    assert data.itemsize == 4 and len(data) == width * height
    return data


def unsigned_float(value, mantissa_bits):
    exponent, mantissa = value >> mantissa_bits, value & ((1 << mantissa_bits)-1)
    if exponent == 31:
        return math.inf if not mantissa else math.nan
    return math.ldexp(mantissa / (1 << mantissa_bits), -14) if not exponent else math.ldexp(1+mantissa/(1 << mantissa_bits), exponent-15)


LUMA_R = [unsigned_float(i, 6)*.2126 for i in range(2048)]
LUMA_G = [unsigned_float(i, 6)*.7152 for i in range(2048)]
LUMA_B = [unsigned_float(i, 5)*.0722 for i in range(1024)]


def luma(word):
    value = LUMA_R[word & 2047] + LUMA_G[(word >> 11) & 2047] + LUMA_B[word >> 22]
    assert math.isfinite(value)
    return value


def contribution(before, after, width, rectangle, step):
    x0, y0, x1, y1 = rectangle
    delta, pre = [], []
    for y in range((y0+step-1)//step*step, y1, step):
        for x in range((x0+step-1)//step*step, x1, step):
            index = y*width+x
            a = luma(before[index]); b = luma(after[index])
            pre.append(a); delta.append(b-a)
    absolute = sorted(abs(v) for v in delta)
    count = len(delta)
    return {'samples': count, 'before_mean_luma': sum(pre)/count,
            'mean_delta': sum(delta)/count, 'mean_absolute_delta': sum(absolute)/count,
            'rms_delta': math.sqrt(sum(v*v for v in delta)/count),
            'p95_absolute_delta': absolute[min(count-1, int(count*.95))],
            'changed_fraction': sum(v != 0 for v in delta)/count,
            'negative_fraction': sum(v < 0 for v in delta)/count}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True, help='Frozen original poses directory')
    parser.add_argument('--target', type=Path, required=True)
    parser.add_argument('--motion-tools', type=Path, required=True, help='Directory containing legacy_rain_motion.py and its audit imports')
    parser.add_argument('--c-math-library', type=Path, required=True, help='Host library exporting layers(float,float*,float*,float*)')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--step', type=int, default=1)
    args = parser.parse_args()
    if args.step < 1:
        parser.error('step must be positive')
    sys.path.insert(0, str(args.motion_tools))
    spec = importlib.util.spec_from_file_location('rain_motion_review', args.motion_tools/'legacy_rain_motion.py')
    motion = importlib.util.module_from_spec(spec); spec.loader.exec_module(motion)
    from verify_rain_capture import verify
    library = ctypes.CDLL(str(args.c_math_library))
    pointer = ctypes.POINTER(ctypes.c_float)
    library.layers.argtypes = [ctypes.c_float, pointer, pointer, pointer]
    vector = lambda values: (ctypes.c_float*len(values))(*values)
    rows = []
    for metadata_path in sorted(args.target.glob('*.source-rain.json')):
        stem = metadata_path.name.removesuffix('.source-rain.json')
        target = lambda suffix: args.target/(stem+suffix)
        old = lambda suffix: args.reference/(stem+suffix)
        metadata = read_json(metadata_path)
        capture = verify(target('.rain-counts.json'))
        assert capture['observed_draws'] == [0, 1, 0]
        assert metadata['present'] == metadata['position_present'] == metadata['diffuse_present'] == capture['present']
        assert metadata['shell_draws'] == 5 and metadata['indices_per_shell'] == 432
        identities = {}
        for name, a, b in [
            ('private_before_is_native_before', '.source-rain-0.rt00.bin', '.rain-drops-before.rt00.bin'),
            ('sampled_frame_is_private_before', '.source-rain-inputs.rt02.bin', '.source-rain-0.rt00.bin'),
            ('lighting_is_same_frame_complete_accumulator', '.source-rain-inputs.rt04.bin', '.sun-encoded-2.rt00.bin'),
            ('shadow_is_native_rain_shadow', '.source-rain-inputs.rt03.bin', '.rain-drops-draw.rt01.bin'),
        ]:
            data = target(a).read_bytes()
            assert data == target(b).read_bytes(), (stem, name)
            identities[name] = hashlib.sha256(data).hexdigest()
        formats = read_json(target('.source-rain-inputs.targets.json'))
        assert [item['view_format'] for item in formats] == [54, 78, 26, 56, 24]
        assert all(item['written'] and item['view_most_detailed_mip'] == 0 for item in formats)
        globals_native = floats(target('.rain-drops-draw.vs-cb1.bin'))
        globals_source = floats(target('.source-rain-globals.bin'))
        assert len(globals_native) == 104 and len(globals_source) == 76
        assert globals_source[:44] == globals_native[:44]
        assert globals_source[60:63] == globals_native[80:83]
        assert globals_source[68:71] == globals_native[40:43]
        assert globals_source[71] == globals_native[100]
        assert globals_source[72:76] == globals_native[56:60]
        inv = inverse_view(globals_native)
        inverse_error = max(abs(a-b) for a,b in zip(globals_source[44:56], (v for row in inv[:3] for v in row)))
        assert inverse_error < .00005
        inv = [list(globals_source[44+i*4:48+i*4]) for i in range(3)] + [[0, 0, 0, 1]]
        shadow = floats(target('.rain-drops-draw.cb0.bin'))
        expected_shadow = [f32(sum(f32(shadow[r*4+k]*inv[k][c]) for k in range(4)))
                           for r in range(4) for c in range(4)]
        origin = [floats(target('.source-rain-0.world.bin'))[31+i*4] for i in range(3)]
        assert max(abs(a-b) for a,b in zip(origin, metadata['origin'])) < 1e-5
        prediction = motion.factory_layers(globals_source[71], origin)
        c_world, c_radii = vector([0]*60), vector([0]*5)
        library.layers(globals_source[71], vector(origin), c_world, c_radii)
        layer_checks = []
        for i in range(5):
            world = floats(target(f'.source-rain-{i}.world.bin'))
            parameters = floats(target(f'.source-rain-{i}.parameters.bin'))
            assert not any(world[:28]) and len(world) == 40
            python_world = [f32(v) for row in prediction[i]['world_rows'] for v in row]
            assert tuple(python_world) == tuple(c_world[i*12:(i+1)*12])
            world_error = max(abs(a-b) for a,b in zip(world[28:40], python_world))
            assert world_error < .0002, (stem, i, world_error)
            assert parameters[:4] == globals_native[76:80]
            assert parameters[4:6] == (1, 1) and parameters[7:9] == (f32(.8), 2)
            radius_error = abs(parameters[6]-c_radii[i])
            assert radius_error < .0001
            error = max(abs(a-b) for a,b in zip(parameters[12:28], expected_shadow))
            assert error < 2e-7, (stem, i, error)
            layer_checks.append({'layer': i, 'world_c_python_max_error': world_error, 'radius_error': radius_error, 'shadow_matrix_error': error})
        # The intercepted native draw occurs after private rendering/restoration.
        geometry = read_json(target('.rain-drops-draw.geometry.json'))
        assert geometry['present'] == metadata['present'] and geometry['stride'] == 36
        assert geometry['topology'] == 4 and geometry['index_format'] == 57
        native_vs = target('.rain-drops-draw.vs.dxbc').read_bytes()
        native_ps = target('.rain-drops-draw.dxbc').read_bytes()
        assert native_vs[4:20].hex() == 'fa59916a2fd744fca0a72c975316d6fd'
        assert native_ps[4:20].hex() == '06358d2eb09e93973771dd433dacd6c4'
        samplers = {s['slot']: s for s in read_json(target('.rain-drops-draw.samplers.json'))}
        assert samplers[0]['filter'] == 21 and samplers[0]['address'] == [1, 1, 1]
        assert samplers[1]['filter'] == 148 and samplers[1]['address'] == [3, 3, 3]
        native_views = {s['slot']: s for s in read_json(target('.rain-drops-draw.targets.json'))}
        assert (native_views[0]['width'], native_views[0]['height'], native_views[0]['view_format']) == (32, 64, 78)
        assert (native_views[1]['width'], native_views[1]['height'], native_views[1]['view_format']) == (128, 128, 56)
        description = read_json(target('.source-rain-0.targets.json'))[0]
        width, height = description['width'], description['height']
        assert description['format'] == 26
        rectangles = {'full': [0, 0, width, height]}
        rectangles.update({f'grid-{y}-{x}': [x*width//3, y*height//3, (x+1)*width//3, (y+1)*height//3]
                           for y in range(3) for x in range(3)})
        private = [color_words(target(f'.source-rain-{i}.rt00.bin'), width, height) for i in range(6)]
        original_before = color_words(old('.rain-before.rt00.bin'), width, height)
        original_after = color_words(old('.rain-4-output.rt00.bin'), width, height)
        native_after = color_words(target('.rain-drops-output.rt00.bin'), width, height)
        verify(old('.rain-counts.json'))
        assert read_json(old('.rain-counts.json'))['observed_draws'] == [5, 0, 0]
        contributions = {}
        for name, rectangle in rectangles.items():
            contributions[name] = {'rectangle': rectangle,
                'original_shells': contribution(original_before, original_after, width, rectangle, args.step),
                'private_shells': contribution(private[0], private[5], width, rectangle, args.step),
                'native_particles': contribution(private[0], native_after, width, rectangle, args.step)}
        layer_contributions = [contribution(private[i], private[i+1], width, rectangles['full'], args.step) for i in range(5)]
        rows.append({'pose': stem, 'frame': metadata['present'], 'identities': identities,
                     'layers': layer_checks, 'inverse_view_max_error': inverse_error, 'native_geometry_after_restore': geometry,
                     'layer_contributions': layer_contributions, 'regions': contributions,
                     'source_time': floats(old('.rain-draw.cb1.bin'))[71],
                     'target_time': globals_source[71], 'target_origin': origin})
        print(stem, 'identities/constants PASS; mean rain:',
              {key: value['mean_delta'] for key, value in contributions['full'].items() if key != 'rectangle'}, flush=True)
    if not rows:
        raise ValueError('no source-rain capture metadata found')
    report = {'target': str(args.target), 'reference': str(args.reference), 'sample_step': args.step,
              'scope': 'Raw HDR contribution distributions in a predefined uniform3x3 grid, not pixel-phase parity. Original/current clock, scroll history, pre-rain color, lighting and depth differ. Private draw counts are runtime-declared5x432 plus five observed target steps; native draw count/geometry is independently intercepted. Unchanged native before and restored shader/IA/SRV/sampler evidence do not independently capture every OM/raster state.',
              'poses': rows}
    args.output.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
