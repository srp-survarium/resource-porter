#!/usr/bin/env python3
"""Prepare exact 0.11e SSAO accumulation, history-mask and temporal draw replays.

Captured history is an input, not a simulated temporal sequence. Both temporal
outputs (AO and depth) are compared. Never overwrite an input or prior replay.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

from extract_011e_weather_shaders import reflection
from replay_post import require, TEXEL_LAYOUT, compare as compare_primary

CLIP_VS = '81ac78836039e3e79ee8e5fa150d9689ea18417b50b24453ba39325bc4c116af'
EYE_VS = '3f081e9239ab97ad23d8f329e15703e398804b18e74a90c5c95de6062c1a7cb0'
PASSES = {
    'ssao-accumulation': ('0420583bf092cbf6d2cda26fa446e58be777980f32c5bc7af40649317a562f85', EYE_VS, 36, [34], [54, 87]),
    'ssao-history-mask': ('c8e117ebc6186d7184631beab59ca9981be8362452f5aa845348d19fede383c6', CLIP_VS, 24, [49], [54, 54]),
    'ssao-temporal': ('5a022edc69d5e2c3e9452dea90de45cd8c07fb45040b621daacb441ddf680326', CLIP_VS, 24, [49, 54], [54, 34, 49, 49]),
}


def prepare(draw, fixture, output):
    require(not output.exists(), 'refusing to overwrite replay/reference')
    matching = [name for name in PASSES if str(draw).endswith('.' + name + '-draw')]
    require(len(matching) == 1, 'not a supported original SSAO draw')
    phase = matching[0]
    ps_sha, vs_sha, stride, output_formats, input_formats = PASSES[phase]
    hashes = {}

    def read(path):
        data = path.read_bytes()
        hashes[str(path.resolve())] = hashlib.sha256(data).hexdigest()
        return data

    def part(suffix):
        return read(Path(str(draw) + suffix))

    def meta(suffix):
        return json.loads(part(suffix))

    def words(*values):
        return struct.pack('<' + 'I' * len(values), *values)

    def blob(data):
        require(0 < len(data) <= 64 * 1024 * 1024, 'blob extent differs')
        return words(len(data)) + data

    manifest = json.loads(read(fixture / 'manifest.json'))['shaders']
    ps, vs = part('.dxbc'), part('.vs.dxbc')
    for code, digest in ((ps, ps_sha), (vs, vs_sha)):
        require(hashlib.sha256(code).hexdigest() == digest, 'captured shader differs')
        require(digest in manifest and read(fixture / manifest[digest]['file']) == code, 'not the archived program')
    stem = str(draw)[:-5]
    before_rows = json.loads(read(Path(stem + '-before.targets.json')))
    after_rows = json.loads(read(Path(stem + '-output.targets.json')))
    require(before_rows == after_rows and [t['format'] for t in before_rows] == output_formats, 'output contract differs')
    width, height = before_rows[0]['width'], before_rows[0]['height']
    require(0 < width <= 4096 and 0 < height <= 4096, 'unsupported dimensions')
    for slot, t in enumerate(before_rows):
        require(t['slot'] == slot and t['width'] == width and t['height'] == height and
                t['array_size'] == t['mip_levels'] == 1 and t['written'] and
                TEXEL_LAYOUT[t['format']] == (1, t['bytes_per_pixel']), 'output extent differs')
    state, geometry = meta('.post-state.json'), meta('.geometry.json')
    scale = 2 if phase == 'ssao-accumulation' else 1
    require(state['viewport_count'] == 1 and state['viewport'] == [0, 0, width * scale, height * scale, 0, 1], 'viewport differs')
    require(state['target_dimension'] == 4 and state['target_view_format'] == output_formats[0] and
            state['fill_mode'] == 3 and state['cull_mode'] == 3 and not state['front_ccw'] and state['depth_clip'] and
            not state['scissor'] and state['depth_bias'] == state['depth_bias_clamp'] == state['slope_bias'] == 0 and
            not state['blend_enabled'] and state['write_mask'] == 15 and state['sample_mask'] == 0xffffffff and
            state['depth_bound'] == (phase == 'ssao-temporal'), 'unsupported original draw state')
    require(geometry['indexed'] and geometry['count'] == 6 and geometry['stride'] == stride and
            geometry['index_format'] == 57 and geometry['topology'] == 4, 'unsupported SSAO quad')
    vertices, indices = part('.vertices.bin'), part('.indices.bin')
    first = geometry['index_offset'] + geometry['start'] * 2
    require(first >= 0 and first + 12 <= len(indices), 'indices out of bounds')
    for index in struct.unpack_from('<6H', indices, first):
        offset = geometry['vertex_offset'] + (index + geometry['base_vertex']) * stride
        require(offset >= 0 and offset + stride <= len(vertices), 'vertex out of bounds')
        require(all(math.isfinite(v) for v in struct.unpack_from('<' + 'f' * (stride // 4), vertices, offset)), 'nonfinite vertex')
    packet = bytearray(b'SSA011E1')
    packet += words(width, height, output_formats[0], output_formats[0], before_rows[0]['bytes_per_pixel'],
                    3, 0, 1, 6, geometry['start'], geometry['base_vertex'] & 0xffffffff, stride,
                    geometry['vertex_offset'], 57, geometry['index_offset'])
    packet += blob(ps) + blob(vs)
    for code, prefix in ((vs, '.vs'), (ps, '')):
        info = reflection(code)
        constants = sorted((r['slot'], info['constants'][name]['size']) for name, r in info['resources'].items() if r['kind'] == 0)
        require(len(constants) <= 4 and all(slot < 4 for slot, _ in constants), 'constant slots differ')
        packet += words(len(constants))
        for slot, required in constants:
            data = part(f'{prefix}.cb{slot}.bin')
            require(required <= len(data) <= 65536 and len(data) % 16 == 0, 'constant extent differs')
            packet += words(slot) + blob(data)
    packet += blob(vertices) + blob(indices)
    textures = meta('.targets.json')
    require([t['slot'] for t in textures] == list(range(len(input_formats))) and
            [t['format'] for t in textures] == input_formats, 'input slots/formats differ')
    packet += words(len(textures))
    require(phase != 'ssao-temporal' or (width % 2 == 0 and height % 2 == 0), 'odd temporal dimensions unsupported')
    expected_sizes = ([(width * 2, height * 2), (4, 4)] if phase == 'ssao-accumulation' else
                      [(width * 2, height * 2)] * 2 if phase == 'ssao-history-mask' else
                      [(width, height), (width // 2, height // 2), (width, height), (width // 2, height // 2)])
    for t in textures:
        require(t['written'] and t['format'] == t['view_format'] and t['view_dimension'] == 4 and
                t['array_size'] == t['mip_levels'] == t['view_mip_levels'] == 1 and t['view_most_detailed_mip'] == 0,
                'input view contract differs')
        w, h, size = t['width'], t['height'], t['bytes_per_pixel']
        require((w, h) == expected_sizes[t['slot']], 'input resolution differs from the pass contract')
        require(0 < w <= 4096 and 0 < h <= 4096 and TEXEL_LAYOUT[t['format']] == (1, size), 'input extent differs')
        data = part(f'.rt{t["slot"]:02}.bin')
        require(len(data) == w * h * size, 'input data extent differs')
        packet += words(t['slot'], 2, w, h, 1, 1, t['format'], t['view_format'], 0, 1)
        packet += words(w * size, w * h * size) + blob(data)
    active = {r['slot'] for r in reflection(ps)['resources'].values() if r['kind'] == 3}
    samplers = [s for s in meta('.samplers.json') if s['slot'] in active]
    require(sorted(s['slot'] for s in samplers) == sorted(active), 'missing/duplicate sampler')
    packet += words(len(samplers))
    for s in samplers:
        packet += words(s['slot']) + struct.pack('<4If2I6f', s['filter'], *s['address'], s['mip_bias'],
                                               s['max_anisotropy'], s['comparison'], *s['border'], s['min_lod'], s['max_lod'])
    before, after = [], []
    for slot, t in enumerate(before_rows):
        for label, rows in (('before', before), ('output', after)):
            data = read(Path(stem + f'-{label}.rt{slot:02}.bin'))
            require(len(data) == width * height * t['bytes_per_pixel'], 'target payload differs')
            rows.append(data)
    packet += blob(before[0]) + struct.pack('<6f', *state['viewport']) + words(len(before) - 1)
    if len(before) == 2:
        packet += words(output_formats[1], before_rows[1]['bytes_per_pixel']) + blob(before[1])
    depth = meta('.depth-state.json')
    require(depth['bound'] == state['depth_bound'], 'depth binding differs')
    packet += words(int(depth['bound']))
    if depth['bound']:
        require(depth == dict(bound=True, reference=0, depth_enabled=True, depth_function=4, depth_write_mask=0,
                             stencil_enabled=True, read_mask=255, write_mask=0, front=[1, 1, 1, 6], back=[1, 1, 1, 6],
                             view_format=45, view_dimension=3, view_flags=0, mip_slice=0), 'depth/stencil state differs')
        ds = struct.pack('<4I2B2x8I', 1, 0, 4, 1, 255, 0, *depth['front'], *depth['back'])
        require(part('.depth-state.bin') == ds, 'depth metadata differs from raw state')
        targets = meta('.depth.targets.json')
        require(len(targets) == 1 and targets[0]['width'] == width and targets[0]['height'] == height and
                targets[0]['format'] == 44 and targets[0]['array_size'] == targets[0]['mip_levels'] == 1 and
                targets[0]['bytes_per_pixel'] == 4 and targets[0]['written'], 'depth surface differs')
        data = part('.depth.rt00.bin')
        require(len(data) == width * height * 4, 'depth surface extent differs')
        packet += words(0) + blob(ds) + blob(data)
    receipt = dict(phase=phase, source_draw=str(draw), pixel_shader_sha256=ps_sha, vertex_shader_sha256=vs_sha,
                   width=width, height=height, format=output_formats[0], pixel_bytes=before_rows[0]['bytes_per_pixel'],
                   packet_sha256=hashlib.sha256(packet).hexdigest(), reference_sha256=hashlib.sha256(after[0]).hexdigest(),
                   secondary_sha256=hashlib.sha256(after[1]).hexdigest() if len(after) == 2 else None, files=hashes,
                   scope='One original SSAO draw with captured inputs/history, geometry, viewport and coverage. Not a live temporal sequence.')
    output.mkdir(parents=True)
    (output / 'packet.bin').write_bytes(packet)
    (output / 'reference.bin').write_bytes(after[0])
    if len(after) == 2:
        (output / 'reference.rt01.bin').write_bytes(after[1])
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return {k: v for k, v in receipt.items() if k != 'files'}


def compare(prepared, rendered):
    result = compare_primary(prepared, rendered)
    receipt = json.loads((prepared / 'receipt.json').read_text())
    if receipt['secondary_sha256']:
        reference = (prepared / 'reference.rt01.bin').read_bytes()
        require(hashlib.sha256(reference).hexdigest() == receipt['secondary_sha256'], 'secondary reference changed')
        actual = Path(str(rendered) + '.rt01.bin').read_bytes()
        require(len(actual) == len(reference), 'secondary output extent differs')
        result['secondary'] = dict(byte_identical=reference == actual, total_bytes=len(reference),
                                   differing_bytes=sum(a != b for a, b in zip(reference, actual)),
                                   rendered_sha256=hashlib.sha256(actual).hexdigest())
    result['scope'] = receipt['scope']
    result['all_targets_byte_identical'] = result['byte_identical'] and result.get('secondary', {}).get('byte_identical', True)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    p = commands.add_parser('prepare')
    p.add_argument('draw', type=Path)
    p.add_argument('fixture', type=Path)
    p.add_argument('output', type=Path)
    p = commands.add_parser('compare')
    p.add_argument('prepared', type=Path)
    p.add_argument('rendered', type=Path)
    args = parser.parse_args()
    result = prepare(args.draw, args.fixture, args.output) if args.command == 'prepare' else compare(args.prepared, args.rendered)
    print(json.dumps(result, indent=2))
