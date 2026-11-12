#!/usr/bin/env python3
"""Prepare unchanged 0.11e skylight shaders with captured inputs for GPU replay.

The default requires captured geometry and depth/stencil coverage. The explicit
--reconstructed experiment omits coverage and reconstructs the quad/VS constant.
Compare all pixels and the subset changed by the captured draw in either case.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

from replay_post import require, compare as compare_packet

VS_SHA = '9dab4eced49b2980e4e70843a6f42e9f59a84bf6e696121bc0c931e685a9f6f4'
PS_SHA = 'ced5345f1cf23b1c8ce9eb050debc71c4fb281f0072aae2d813cc7a5d2232b11'


def prepare(draw, vertex_shader, output, reconstructed=False):
    require(not output.exists(), 'refusing to overwrite a replay or reference')
    require(str(draw).endswith('.skylight-draw'), 'expected a skylight-draw base')
    hashes = {}

    def read(path):
        data = path.read_bytes()
        hashes[str(path.resolve())] = hashlib.sha256(data).hexdigest()
        return data

    def part(suffix):
        return read(Path(str(draw) + suffix))

    def words(*values):
        return struct.pack('<' + 'I' * len(values), *values)

    def blob(data):
        require(0 < len(data) <= 64 * 1024 * 1024, 'unsupported blob extent')
        return words(len(data)) + data

    ps, vs = part('.dxbc'), read(vertex_shader)
    require(hashlib.sha256(ps).hexdigest() == PS_SHA, 'not the original skylight PS')
    require(hashlib.sha256(vs).hexdigest() == VS_SHA, 'not the original skylight VS')
    cb0, cb1 = part('.cb0.bin'), part('.cb1.bin')
    require(len(cb0) == 160 and 224 <= len(cb1) <= 65536 and len(cb1) % 16 == 0,
            'skylight constant extent differs')
    require(all(math.isfinite(x[0]) for x in struct.iter_unpack('<f', cb0)), 'nonfinite weather constants')
    require(not json.loads(part('.blend.json'))['enabled'], 'blended skylight is unsupported')
    stem = str(draw)[:-len('-draw')]
    before_rows = json.loads(read(Path(stem + '-before.targets.json')))
    after_rows = json.loads(read(Path(stem + '-output.targets.json')))
    require(before_rows == after_rows and len(before_rows) == 1, 'skylight target changed')
    target = before_rows[0]
    width, height = target['width'], target['height']
    require(0 < width <= 4096 and 0 < height <= 4096 and target['slot'] == 0 and
            target['format'] == 24 and target['array_size'] == target['mip_levels'] == 1 and
            target['bytes_per_pixel'] == 4 and target['written'], 'unsupported skylight target')
    for phase in ('before', 'output'):
        require(json.loads(read(Path(stem + '-' + phase + '.view.json'))) == {'format': 24, 'dimension': 4},
                'unexpected target view')

    # Original VS forwards clip position and UV; all basis outputs depend only
    # on skylight_parameters6. Preserve the original shader's arithmetic.
    vertices = b''.join(struct.pack('<6f', x, y, 0, 1, u, v)
                        for x, y, u, v in [(-1, 1, 0, 0), (1, 1, 1, 0),
                                            (-1, -1, 0, 1), (1, -1, 1, 1)])
    indices = struct.pack('<6H', 0, 1, 2, 2, 1, 3)
    vertex_constant = cb0[128:144]
    header = (width, height, 24, 24, 4, 1, 0, 1, 6, 0, 0, 24, 0, 57, 0)
    if not reconstructed:
        require(part('.vs.dxbc') == vs, 'captured skylight VS differs')
        vertex_constant = part('.vs.cb0.bin')
        require(vertex_constant == cb0[128:144], 'captured skylight basis constant differs')
        state = json.loads(part('.post-state.json'))
        require(state['viewport_count'] == 1 and state['viewport'] == [0, 0, width, height, 0, 1] and
                state['target_view_format'] == 24 and state['target_dimension'] == 4 and
                state['fill_mode'] == 3 and state['cull_mode'] == 3 and not state['front_ccw'] and
                state['depth_clip'] and not state['scissor'] and state['depth_bias'] == state['depth_bias_clamp'] == state['slope_bias'] == 0 and
                state['depth_bound'] and not state['blend_enabled'] and state['write_mask'] == 7 and
                state['sample_mask'] == 0xffffffff, 'unsupported captured skylight state')
        geometry = json.loads(part('.geometry.json'))
        require(geometry['indexed'] and geometry['count'] == 6 and geometry['stride'] == 24 and
                geometry['index_format'] == 57 and geometry['topology'] == 4, 'unsupported skylight geometry')
        vertices, indices = part('.vertices.bin'), part('.indices.bin')
        first = geometry['index_offset'] + geometry['start'] * 2
        require(first >= 0 and first + 12 <= len(indices), 'skylight indices out of bounds')
        for index in struct.unpack_from('<6H', indices, first):
            offset = geometry['vertex_offset'] + (index + geometry['base_vertex']) * 24
            require(offset >= 0 and offset + 24 <= len(vertices), 'skylight vertex out of bounds')
            require(all(math.isfinite(x) for x in struct.unpack_from('<6f', vertices, offset)), 'nonfinite skylight vertex')
        header = (width, height, 24, 24, 4, 3, 0, 1, 6, geometry['start'], geometry['base_vertex'] & 0xffffffff,
                  24, geometry['vertex_offset'], 57, geometry['index_offset'])
    packet = bytearray(b'SKY011E1' if reconstructed else b'SKY011E2')
    packet += words(*header)
    packet += blob(ps) + blob(vs)
    packet += words(1, 0) + blob(vertex_constant)
    packet += words(2, 0) + blob(cb0) + words(1) + blob(cb1)
    packet += blob(vertices) + blob(indices)
    textures = json.loads(part('.targets.json'))
    require([t['slot'] for t in textures] == [0, 1, 2], 'unexpected skylight input slots')
    packet += words(3)
    for t, fmt, size in zip(textures, (54, 24, 49), (2, 4, 2)):
        require(t['width'] == width and t['height'] == height and
                t['format'] == t['view_format'] == fmt and t['view_dimension'] == 4 and
                t['array_size'] == t['mip_levels'] == t['view_mip_levels'] == 1 and
                t['view_most_detailed_mip'] == 0 and t['bytes_per_pixel'] == size and t['written'],
                'skylight input contract changed')
        data = part(f'.rt{t["slot"]:02}.bin')
        require(len(data) == width * height * size, 'skylight input extent differs')
        packet += words(t['slot'], 2, width, height, 1, 1, fmt, fmt, 0, 1)
        packet += words(width * size, width * height * size) + blob(data)
    samplers = [s for s in json.loads(part('.samplers.json')) if s['slot'] in (0, 1, 2)]
    require(sorted(s['slot'] for s in samplers) == [0, 1, 2], 'missing/duplicate skylight sampler')
    packet += words(3)
    for s in samplers:
        packet += words(s['slot']) + struct.pack('<4If2I6f', s['filter'], *s['address'], s['mip_bias'],
                                               s['max_anisotropy'], s['comparison'], *s['border'],
                                               s['min_lod'], s['max_lod'])
    before, after = (read(Path(stem + '-' + phase + '.rt00.bin')) for phase in ('before', 'output'))
    require(len(before) == len(after) == width * height * 4, 'target extent differs')
    packet += blob(before)
    if not reconstructed:
        depth = json.loads(part('.depth-state.json'))
        expected = dict(bound=True, reference=0, depth_enabled=False, depth_function=2, depth_write_mask=1,
                        stencil_enabled=True, read_mask=255, write_mask=0, front=[1, 1, 1, 6], back=[1, 1, 1, 6],
                        view_format=45, view_dimension=3, view_flags=0, mip_slice=0)
        require(depth == expected, 'unsupported skylight depth/stencil contract')
        # Construct the ABI structure from typed metadata; require the captured
        # bytes to agree as a separate consistency check (including padding).
        ds = struct.pack('<4I2B2x8I', 0, 1, 2, 1, 255, 0, *depth['front'], *depth['back'])
        require(part('.depth-state.bin') == ds, 'depth/stencil metadata differs from binary')
        targets = json.loads(part('.depth.targets.json'))
        require(len(targets) == 1 and targets[0]['width'] == width and targets[0]['height'] == height and
                targets[0]['format'] == 44 and targets[0]['array_size'] == targets[0]['mip_levels'] == 1 and
                targets[0]['bytes_per_pixel'] == 4 and targets[0]['written'], 'unsupported depth/stencil surface')
        depth_data = part('.depth.rt00.bin')
        require(len(depth_data) == width * height * 4, 'depth/stencil surface extent differs')
        packet += words(7, 0) + blob(ds) + blob(depth_data)
    receipt = dict(source_draw=str(draw), pixel_shader_sha256=PS_SHA, vertex_shader_sha256=VS_SHA,
                   width=width, height=height, format=24, pixel_bytes=4,
                   packet_sha256=hashlib.sha256(packet).hexdigest(),
                   reference_sha256=hashlib.sha256(after).hexdigest(),
                   before_sha256=hashlib.sha256(before).hexdigest(), files=hashes,
                   reconstructed_geometry=reconstructed,
                   scope=('Original skylight shaders with reconstructed clip quad and VS constant; no depth/stencil coverage.' if reconstructed else
                          'Original skylight shaders, captured geometry/constants, RGB write mask and stencil coverage.') + ' GPU comparison required.')
    output.mkdir(parents=True)
    for name, data in [('packet.bin', packet), ('reference.bin', after), ('before.bin', before)]:
        (output / name).write_bytes(data)
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return {k: v for k, v in receipt.items() if k != 'files'}


def compare(prepared, rendered):
    result = compare_packet(prepared, rendered)
    receipt = json.loads((prepared / 'receipt.json').read_text())
    before = (prepared / 'before.bin').read_bytes()
    require(hashlib.sha256(before).hexdigest() == receipt['before_sha256'], 'before target changed')
    reference, actual = (prepared / 'reference.bin').read_bytes(), rendered.read_bytes()
    counts = dict(changed=0, unchanged=0)
    differences = dict(changed=0, unchanged=0)
    rgb_differences = dict(changed=0, unchanged=0)
    maximum_by_group = dict(changed=0, unchanged=0)
    maximum, outside_step = 0, 0
    for (a,), (b,), (c,) in zip(struct.iter_unpack('<I', before), struct.iter_unpack('<I', reference),
                              struct.iter_unpack('<I', actual)):
        group = 'changed' if a != b else 'unchanged'
        counts[group] += 1
        differences[group] += b != c
        rgb_differences[group] += (b & 0x3fffffff) != (c & 0x3fffffff)
        error = max(abs(((b >> shift) & 1023) - ((c >> shift) & 1023)) for shift in (0, 10, 20))
        maximum = max(maximum, error)
        maximum_by_group[group] = max(maximum_by_group[group], error)
        outside_step += error > 1
    result.update(pixels=counts, differing_pixels=differences, max_rgb_10bit_error=maximum,
                  differing_rgb_pixels=rgb_differences, max_rgb_10bit_error_by_group=maximum_by_group,
                  pixels_outside_one_10bit_step=outside_step,
                  scope=('Unchanged original skylight programs; reconstructed quad, no stencil/depth coverage.' if receipt.get('reconstructed_geometry', True) else
                         'Unchanged original skylight programs with captured geometry and stencil coverage.') + ' Not native integration.')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    p = commands.add_parser('prepare')
    p.add_argument('draw', type=Path)
    p.add_argument('vertex_shader', type=Path)
    p.add_argument('output', type=Path)
    p.add_argument('--reconstructed', action='store_true', help='explicit diagnostic without captured geometry/stencil')
    p = commands.add_parser('compare')
    p.add_argument('prepared', type=Path)
    p.add_argument('rendered', type=Path)
    args = parser.parse_args()
    result = prepare(args.draw, args.vertex_shader, args.output, args.reconstructed) if args.command == 'prepare' else compare(args.prepared, args.rendered)
    print(json.dumps(result, indent=2))
