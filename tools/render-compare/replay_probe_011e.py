#!/usr/bin/env python3
"""Replay an unchanged original 0.11e probe draw, both MRTs and stencil writes."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct

from extract_011e_weather_shaders import reflection
from replay_post import require, TEXEL_LAYOUT

PIXEL_SHADERS = {
    '9dd5d3bf4807c947420a67a839d19ea22e9faa09b4fe941f27ccd56fad76664c': 540,
    'a0298850953c9a3def86ae290a34bb6b7d432766a026e6b4ec349ad562f59592': 36,
}
VERTEX_SHADER = 'c2d433e1303804731c5cdca124d47a9c6eda6bb94c4ecf59129902c599b9f01e'


def prepare(draw, fixture, output):
    require(not output.exists(), 'refusing to overwrite probe evidence')
    match = re.fullmatch(r'(.+)\.probe-draw(-\d+)?', str(draw))
    require(match is not None, 'not a probe draw base')
    frame, variant = match.groups()
    variant = variant or ''
    files = {}

    def read(path):
        data = path.read_bytes()
        files[str(path.resolve())] = hashlib.sha256(data).hexdigest()
        return data

    def part(suffix):
        return read(Path(str(draw) + suffix))

    def meta(suffix):
        return json.loads(part(suffix))

    def target(label, suffix):
        return read(Path(frame + '.probe-' + label + variant + suffix))

    def words(*values):
        return struct.pack('<' + 'I' * len(values), *values)

    def blob(data):
        require(0 < len(data) <= 64 * 1024 * 1024, 'blob extent differs')
        return words(len(data)) + data

    manifest = json.loads(read(fixture / 'manifest.json'))['shaders']
    ps, vs = part('.dxbc'), part('.vs.dxbc')
    ps_sha, vs_sha = hashlib.sha256(ps).hexdigest(), hashlib.sha256(vs).hexdigest()
    require(ps_sha in PIXEL_SHADERS and vs_sha == VERTEX_SHADER, 'original probe shader differs')
    for code, digest in ((ps, ps_sha), (vs, vs_sha)):
        require(digest in manifest and code == read(fixture / manifest[digest]['file']), 'not the archived program')
    before_rows = json.loads(target('before', '.targets.json'))
    after_rows = json.loads(target('output', '.targets.json'))
    require(before_rows == after_rows and len(before_rows) == 2, 'probe MRT contracts differ')
    width, height = before_rows[0]['width'], before_rows[0]['height']
    require(0 < width <= 4096 and 0 < height <= 4096, 'unsupported dimensions')
    for slot, row in enumerate(before_rows):
        require(row['slot'] == slot and row['format'] == 24 and row['bytes_per_pixel'] == 4 and
                row['width'] == width and row['height'] == height and row['written'] and
                row['array_size'] == row['mip_levels'] == 1, 'MRT extent differs')
    state, geometry, depth = meta('.post-state.json'), meta('.geometry.json'), meta('.depth-state.json')
    require(state['viewport_count'] == 1 and state['viewport'] == [0, 0, width, height, 0, 1] and
            state['target_view_format'] == 24 and state['target_dimension'] == 4 and state['fill_mode'] == 3 and
            state['cull_mode'] in (2, 3) and not state['front_ccw'] and state['depth_clip'] and not state['scissor'] and
            state['depth_bias'] == state['depth_bias_clamp'] == state['slope_bias'] == 0 and state['depth_bound'] and
            state['blend_enabled'] and state['write_mask'] == 7 and state['sample_mask'] == 0xffffffff,
            'unsupported probe raster/target state')
    front = [1, 6, 6, 3] if state['cull_mode'] == 3 else [1, 1, 6, 3]
    expected_depth = dict(bound=True, reference=255, depth_enabled=state['cull_mode'] == 3,
                          depth_function=4 if state['cull_mode'] == 3 else 2,
                          depth_write_mask=0 if state['cull_mode'] == 3 else 1,
                          stencil_enabled=True, read_mask=64, write_mask=255, front=front, back=front,
                          view_format=45, view_dimension=3, view_flags=0, mip_slice=0)
    require(depth == expected_depth, 'probe depth/stencil state differs')
    ds = struct.pack('<4I2B2x8I', int(depth['depth_enabled']), depth['depth_write_mask'], depth['depth_function'], 1, 64, 255, *front, *front)
    require(part('.depth-state.bin') == ds, 'depth metadata disagrees with bytes')
    require(geometry['indexed'] and geometry['count'] == PIXEL_SHADERS[ps_sha] and geometry['stride'] == 12 and
            geometry['index_format'] == 57 and geometry['topology'] == 4, 'probe volume geometry differs')
    vertices, indices = part('.vertices.bin'), part('.indices.bin')
    first, count = geometry['index_offset'] + geometry['start'] * 2, geometry['count']
    require(0 <= first and first + count * 2 <= len(indices), 'index range invalid')
    for index in struct.unpack_from('<' + 'H' * count, indices, first):
        offset = geometry['vertex_offset'] + (index + geometry['base_vertex']) * 12
        require(0 <= offset and offset + 12 <= len(vertices), 'vertex range invalid')
        require(all(math.isfinite(v) for v in struct.unpack_from('<3f', vertices, offset)), 'nonfinite vertex')
    packet = bytearray(b'PRB011E1')
    packet += words(width, height, 24, 24, 4, state['cull_mode'], 0, 1, count, geometry['start'],
                    geometry['base_vertex'] & 0xffffffff, 12, geometry['vertex_offset'], 57, geometry['index_offset'])
    packet += blob(ps) + blob(vs)
    for code, prefix in ((vs, '.vs'), (ps, '')):
        info = reflection(code)
        constants = sorted((r['slot'], info['constants'][name]['size']) for name, r in info['resources'].items() if r['kind'] == 0)
        require(len(constants) <= 4 and all(slot < 4 for slot, _ in constants), 'constant slots differ')
        packet += words(len(constants))
        for slot, size in constants:
            data = part(f'{prefix}.cb{slot}.bin')
            require(len(data) == size, 'constant buffer size differs')
            packet += words(slot) + blob(data)
    packet += blob(vertices) + blob(indices)
    textures = meta('.targets.json')
    # The original leaves probe-index slot 5 unbound for some volumes. Preserve
    # that state; creating a substitute texture would change the shader input.
    require(len(textures) in (5, 6) and [t['slot'] for t in textures] == list(range(len(textures))) and
            [t['format'] for t in textures] == [54, 24, 28, 77, 49, 61][:len(textures)], 'probe input slots/formats differ')
    packet += words(len(textures))
    for t in textures:
        slot, w, h, mips, fmt = (t[k] for k in ('slot', 'width', 'height', 'mip_levels', 'format'))
        cube = slot == 3
        require(t['written'] and t['view_format'] == fmt and t['view_most_detailed_mip'] == 0 and
                t['view_mip_levels'] == mips and t['view_dimension'] == (9 if cube else 4) and
                t['array_size'] == (6 if cube else 1), 'probe input view differs')
        require((cube and w == h and 0 < w <= 1024 and 1 <= mips <= 11 and t.get('written_subresources') == 6 * mips) or
                (not cube and (w, h) == (width, height) and mips == 1), 'input dimensions/mips differ')
        packet += words(slot, 4 if cube else 2, w, h, 6 if cube else 1, mips, fmt, fmt, 0, mips)
        block, size = TEXEL_LAYOUT[fmt]
        for face in range(6 if cube else 1):
            for mip in range(mips):
                mw, mh = max(1, w >> mip), max(1, h >> mip)
                pitch = ((mw + block - 1) // block) * size
                slice_size = pitch * ((mh + block - 1) // block)
                suffix = f'.rt{slot:02}.face{face}.mip{mip:02}.bin' if cube else f'.rt{slot:02}.bin'
                data = part(suffix)
                require(len(data) == slice_size, 'texture subresource extent differs')
                packet += words(pitch, slice_size) + blob(data)
    active = {r['slot'] for r in reflection(ps)['resources'].values() if r['kind'] == 3}
    samplers = [s for s in meta('.samplers.json') if s['slot'] in active]
    require(sorted(s['slot'] for s in samplers) == sorted(active), 'samplers missing/duplicated')
    packet += words(len(samplers))
    for s in samplers:
        packet += words(s['slot']) + struct.pack('<4If2I6f', s['filter'], *s['address'], s['mip_bias'],
                                               s['max_anisotropy'], s['comparison'], *s['border'], s['min_lod'], s['max_lod'])
    before = [target('before', f'.rt{slot:02}.bin') for slot in range(2)]
    after = [target('output', f'.rt{slot:02}.bin') for slot in range(2)]
    require(all(len(data) == width * height * 4 for data in before + after), 'MRT payload extent differs')
    bs, factors = part('.blend-state.bin'), part('.blend-factors.bin')
    require(len(bs) == 264 and len(factors) == 16, 'blend state extent differs')
    blend_words = struct.unpack('<66I', bs)
    require(blend_words[:2] == (0, 0) and blend_words[2:10] == (1, 5, 6, 1, 2, 1, 1, 7), 'probe blend differs')
    require(all(math.isfinite(x) for x in struct.unpack('<4f', factors)), 'nonfinite blend factor')
    depth_rows = meta('.depth.targets.json')
    require(len(depth_rows) == 1 and depth_rows[0]['format'] == 44 and depth_rows[0]['width'] == width and
            depth_rows[0]['height'] == height and depth_rows[0]['written'] and
            depth_rows[0]['mip_levels'] == depth_rows[0]['array_size'] == 1, 'depth texture differs')
    depth_before, depth_after = part('.depth.rt00.bin'), target('output', '.depth.rt00.bin')
    require(len(depth_before) == len(depth_after) == width * height * 4, 'depth/stencil payload extent differs')
    packet += blob(before[0]) + blob(before[1]) + blob(bs) + blob(factors) + words(255) + blob(ds) + blob(depth_before)
    receipt = dict(source_draw=str(draw), files=files, present=geometry['present'], variant=int(variant[1:] or 0),
                   pixel_shader_sha256=ps_sha, vertex_shader_sha256=vs_sha, width=width, height=height,
                   packet_sha256=hashlib.sha256(packet).hexdigest(),
                   reference_sha256=[hashlib.sha256(data).hexdigest() for data in after + [depth_after]],
                   depth_before_sha256=hashlib.sha256(depth_before).hexdigest(),
                   scope='One original blended probe draw with captured previous accumulators and stencil; not native integration.')
    output.mkdir(parents=True)
    (output / 'packet.bin').write_bytes(packet)
    (output / 'reference.depth-before.bin').write_bytes(depth_before)
    for name, data in zip(('reference.bin', 'reference.rt01.bin', 'reference.depth.bin'), after + [depth_after]):
        (output / name).write_bytes(data)
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return {k: v for k, v in receipt.items() if k != 'files'}


def compare(prepared, rendered):
    receipt = json.loads((prepared / 'receipt.json').read_text())
    checks = []
    for suffix, digest in zip(('', '.rt01.bin', '.depth.bin'), receipt['reference_sha256']):
        reference = (prepared / ('reference' + (suffix or '.bin'))).read_bytes()
        require(hashlib.sha256(reference).hexdigest() == digest, 'reference changed')
        actual = Path(str(rendered) + suffix).read_bytes()
        require(len(actual) == len(reference), 'rendered extent differs')
        checks.append(dict(target=suffix or 'diffuse', byte_identical=actual == reference,
                           differing_bytes=sum(a != b for a, b in zip(actual, reference)),
                           sha256=hashlib.sha256(actual).hexdigest()))
    source_before = (prepared / 'reference.depth-before.bin').read_bytes()
    require(hashlib.sha256(source_before).hexdigest() == receipt['depth_before_sha256'], 'depth input changed')
    gpu_before = Path(str(rendered) + '.depth-before.bin').read_bytes()
    source_after = (prepared / 'reference.depth.bin').read_bytes()
    gpu_after = Path(str(rendered) + '.depth.bin').read_bytes()
    require(len(source_before) == len(gpu_before) == len(source_after) == len(gpu_after), 'depth extents differ')
    source_preserved = gpu_preserved = True
    minimum, maximum = 0, 0
    for (sb,), (gb,), (sa,), (ga,) in zip(*(struct.iter_unpack('<I', data) for data in
                                           (source_before, gpu_before, source_after, gpu_after))):
        source_preserved = source_preserved and (sb & 0xffffff) == (sa & 0xffffff)
        gpu_preserved = gpu_preserved and (gb & 0xffffff) == (ga & 0xffffff)
        delta = (gb & 0xffffff) - (sb & 0xffffff)
        minimum, maximum = min(minimum, delta), max(maximum, delta)
    stencil_exact = gpu_after[3::4] == source_after[3::4]
    uploaded_stencil_exact = gpu_before[3::4] == source_before[3::4]
    depth = dict(source_unchanged=source_preserved, gpu_unchanged=gpu_preserved,
                 uploaded_stencil_byte_identical=uploaded_stencil_exact, stencil_byte_identical=stencil_exact,
                 upload_depth_delta_min=minimum, upload_depth_delta_max=maximum,
                 note='D24 upload/readback is measured before drawing; depth writes are disabled.')
    passed = all(c['byte_identical'] for c in checks[:2]) and stencil_exact and uploaded_stencil_exact and \
        source_preserved and gpu_preserved and minimum >= 0 and maximum <= 1
    return dict(checks=checks, depth=depth, passed=passed, scope=receipt['scope'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    command = commands.add_parser('prepare')
    for name in ('draw', 'fixture', 'output'):
        command.add_argument(name, type=Path)
    command = commands.add_parser('compare')
    for name in ('prepared', 'rendered'):
        command.add_argument(name, type=Path)
    args = parser.parse_args()
    result = prepare(args.draw, args.fixture, args.output) if args.command == 'prepare' else compare(args.prepared, args.rendered)
    print(json.dumps(result, indent=2))
    if args.command == 'compare' and not result['passed']:
        raise SystemExit(1)
