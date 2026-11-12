#!/usr/bin/env python3
"""Prepare and compare an unchanged original post pass, without starting a game.

Capture with SRP_CAPTURE_POST_GEOMETRY=1 and the stage observer. Preparation
refuses missing mips/state or unsupported geometry rather than approximating
them. Run replay_post.exe PACKET OUTPUT under the guarded headless GPU rig;
then use compare. Original files are read only and hashed in the receipt.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

from extract_011e_weather_shaders import reflection

VERTEX_SHA = '29abfee6fb6bbf38274bcaca3dce5bb647a6099751b83a0beb6e2e571b2e3714'
PRESENT_SHA = '685c00b3451a1fb3291f40696e7b4ae34b0b18f6bad32ab62e634b5f8f9cb451'
TEXEL_LAYOUT = {2: (1, 16), **{f: (1, 8) for f in (10, 16, 19, 20)},
                **{f: (1, 4) for f in (44, 45, 34, 41, 24, 26, 28, 29, 87, 90, 91)},
                **{f: (1, 2) for f in (54, 53, 56, 49)}, 61: (1, 1),
                **{f: (4, 8) for f in (70, 71, 72)},
                **{f: (4, 16) for f in (73, 74, 75, 76, 77, 78)}}


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def prepare(draw, fixture, output, texture_overrides=None, geometry_draw=None):
    require(not output.exists(), 'refusing to overwrite a replay or reference')
    presentation = str(draw).endswith('.present-draw')
    require(presentation or str(draw).endswith('.post-draw'), 'expected a post or present capture base')
    require(presentation == (geometry_draw is not None), 'presentation requires an explicit matching post geometry capture')
    if presentation:
        require(str(draw)[:-len('.present-draw')] == str(geometry_draw)[:-len('.post-draw')] and
                str(geometry_draw).endswith('.post-draw'), 'presentation geometry must belong to the same captured frame')
    hashes = {}

    def read(path):
        data = path.read_bytes()
        hashes[str(path.resolve())] = hashlib.sha256(data).hexdigest()
        return data

    def part(suffix):
        return read(Path(str(draw) + suffix))

    def metadata(suffix):
        return json.loads(part(suffix))

    def geometry_part(suffix):
        return read(Path(str(geometry_draw or draw) + suffix))

    def u32(*values):
        return struct.pack('<' + 'I' * len(values), *values)

    def blob(data):
        require(0 < len(data) <= 64 * 1024 * 1024, 'invalid blob extent')
        return u32(len(data)) + data

    manifest = json.loads(read(fixture / 'manifest.json'))
    ps, vs = part('.dxbc'), geometry_part('.vs.dxbc')
    ps_sha = hashlib.sha256(ps).hexdigest()
    source = manifest['shaders'].get(ps_sha)
    # The observer's present label is the gamma copy_image pass. The archive's
    # separately named present.ps is a plain blit with a different signature.
    family = 'copy_image.ps' if presentation else 'complex_post_process_blend.ps'
    require(source and source['name'] == family, 'not the expected archived original shader')
    require(not presentation or ps_sha == PRESENT_SHA, 'not the observed original gamma copy')
    require(read(fixture / source['file']) == ps, 'shader fixture differs from the captured program')
    require(hashlib.sha256(vs).hexdigest() == VERTEX_SHA, 'not the original copy_image vertex program')
    ps_info, vs_info = reflection(ps), reflection(vs)
    state = json.loads(geometry_part('.post-state.json'))
    geometry = json.loads(geometry_part('.geometry.json'))
    stem = str(draw)[:-len('-draw')]
    before_rows = json.loads(read(Path(stem + '-before.targets.json')))
    after_rows = json.loads(read(Path(stem + '-output.targets.json')))
    require(len(before_rows) == len(after_rows) == 1, 'expected one post target')
    target, after_target = before_rows[0], after_rows[0]
    if presentation:
        # The original full-screen quad is reused explicitly; its correctness
        # for presentation is established only by the subsequent GPU comparison.
        state['target_view_format'] = target['format']
    require(target == after_target and target['slot'] == 0 and target.get('written'), 'post target changed or was not captured')
    width, height, pixel_bytes = target['width'], target['height'], target['bytes_per_pixel']
    require(0 < width <= 4096 and 0 < height <= 4096 and pixel_bytes in (4, 8, 16), 'unsupported post target')
    require(TEXEL_LAYOUT.get(target['format']) == (1, pixel_bytes), 'target format/extent mismatch')
    require(state['viewport_count'] == 1 and state['viewport'] == [0, 0, width, height, 0, 1], 'not a full-target viewport')
    require(state['target_dimension'] == 4 and target['array_size'] == 1, 'not a single 2D target')
    require(not state['depth_bound'] and not state['blend_enabled'] and state['write_mask'] == 15 and state['sample_mask'] == 0xffffffff,
            'unsupported post depth/blend state')
    require(state['fill_mode'] == 3 and state['cull_mode'] in (1, 2, 3) and not state['scissor'] and
            state['depth_bias'] == state['depth_bias_clamp'] == state['slope_bias'] == 0,
            'unsupported post rasterizer')
    require(geometry['indexed'] and geometry['count'] == 6 and geometry['stride'] == 28 and geometry['topology'] == 4,
            'not the original indexed TL quad')
    require(geometry['index_format'] in (42, 57), 'unsupported index format')
    vertices, indices = geometry_part('.vertices.bin'), geometry_part('.indices.bin')
    index_bytes, index_code = (4, 'I') if geometry['index_format'] == 42 else (2, 'H')
    first = geometry['index_offset'] + geometry['start'] * index_bytes
    require(first >= 0 and first + 6 * index_bytes <= len(indices), 'quad indices out of bounds')
    selected = struct.unpack_from('<6' + index_code, indices, first)
    for index in selected:
        offset = geometry['vertex_offset'] + (geometry['base_vertex'] + index) * 28
        require(offset >= 0 and offset + 28 <= len(vertices), 'quad vertex out of bounds')
        require(all(math.isfinite(x) for x in struct.unpack_from('<4f', vertices, offset)), 'nonfinite quad position')

    packet = bytearray(b'PST011E1')
    packet += u32(width, height, target['format'], state['target_view_format'], pixel_bytes,
                  state['cull_mode'], int(state['front_ccw']), int(state['depth_clip']),
                  geometry['count'], geometry['start'], geometry['base_vertex'] & 0xffffffff,
                  geometry['stride'], geometry['vertex_offset'], geometry['index_format'], geometry['index_offset'])
    packet += blob(ps) + blob(vs)
    for info, prefix in [(vs_info, '.vs'), (ps_info, '')]:
        buffers = sorted((r['slot'], info['constants'][name]['size']) for name, r in info['resources'].items() if r['kind'] == 0)
        require(len(buffers) <= 4 and all(slot < 4 for slot, _ in buffers), 'unsupported constant slots')
        packet += u32(len(buffers))
        for slot, required in buffers:
            data = (geometry_part if prefix else part)(f'{prefix}.cb{slot}.bin')
            require(required <= len(data) <= 65536 and len(data) % 16 == 0, 'constant buffer extent mismatch')
            packet += u32(slot) + blob(data)
    packet += blob(vertices) + blob(indices)

    active_textures = {r['slot'] for r in ps_info['resources'].values() if r['kind'] == 2}
    targets = [t for t in metadata('.targets.json') if t['slot'] in active_textures]
    require(len({t['slot'] for t in targets}) == len(targets) <= 16, 'duplicate/excess texture slots')
    substitutions = {}
    for name, replacement in (texture_overrides or {}).items():
        resource = ps_info['resources'].get(name)
        require(resource and resource['kind'] == 2, 'not an original texture: ' + name)
        other_info = reflection(read(Path(str(replacement) + '.dxbc')))
        other = other_info['resources'].get(name)
        require(other and other['kind'] == 2, 'replacement shader lacks texture: ' + name)
        original = [t for t in targets if t['slot'] == resource['slot']]
        other_targets = json.loads(read(Path(str(replacement) + '.targets.json')))
        selected = [t for t in other_targets if t['slot'] == other['slot']]
        require(len(original) == len(selected) == 1, 'ambiguous or absent texture: ' + name)
        # This is an input sensitivity experiment, not a claim that two engines
        # assign the same meaning to these bytes. Keep the storage contract exact.
        keys = ('width', 'height', 'depth', 'format', 'array_size', 'mip_levels',
                'view_format', 'view_dimension', 'view_most_detailed_mip',
                'view_mip_levels', 'block_width', 'bytes_per_block', 'bytes_per_pixel')
        require(all(original[0].get(k) == selected[0].get(k) for k in keys),
                'replacement texture storage differs: ' + name)
        require(selected[0].get('written'), 'replacement texture was not captured: ' + name)
        substitutions[resource['slot']] = dict(name=name, draw=str(replacement), slot=other['slot'])
    packet += u32(len(targets))
    for texture in sorted(targets, key=lambda t: t['slot']):
        require(texture.get('written'), 'active texture was not captured')
        dimension = texture['view_dimension']
        require(dimension in (4, 8), 'unsupported texture view dimension')
        depth = texture.get('depth', 1)
        mip_count = texture['mip_levels']
        first_mip = texture['view_most_detailed_mip']
        view_mips = texture['view_mip_levels']
        if view_mips == 0xffffffff:
            view_mips = mip_count - first_mip
        require(1 <= mip_count <= 13 and 0 <= first_mip < mip_count and 1 <= view_mips <= mip_count - first_mip, 'invalid mip range')
        require(dimension != 4 or texture['array_size'] == 1, 'texture arrays are unsupported')
        block = texture.get('block_width', 1)
        element = texture['bytes_per_block'] if block != 1 else texture['bytes_per_pixel']
        require(block in (1, 4) and element in (1, 2, 4, 8, 16), 'invalid texel/block size')
        require(TEXEL_LAYOUT.get(texture['format']) == (block, element), 'texture format/extent mismatch')
        slot = texture['slot']
        packet += u32(slot, 3 if dimension == 8 else 2, texture['width'], texture['height'], depth,
                      mip_count, texture['format'], texture['view_format'], first_mip, view_mips)
        for mip in range(mip_count):
            w, h, d = max(1, texture['width'] >> mip), max(1, texture['height'] >> mip), max(1, depth >> mip)
            pitch, rows = ((w + block - 1) // block) * element, (h + block - 1) // block
            kind = 'vol' if dimension == 8 else 'rt'
            replacement = substitutions.get(slot)
            input_slot = replacement['slot'] if replacement else slot
            suffix = f'.{kind}{input_slot:02}.bin' if mip == 0 else f'.{kind}{input_slot:02}.mip{mip:02}.bin'
            data = read(Path(replacement['draw'] + suffix)) if replacement else part(suffix)
            require(len(data) == pitch * rows * d, 'texture mip extent mismatch: ' + suffix)
            packet += u32(pitch, pitch * rows) + blob(data)

    active_samplers = {r['slot'] for r in ps_info['resources'].values() if r['kind'] == 3}
    samplers = [s for s in metadata('.samplers.json') if s['slot'] in active_samplers]
    require(len({s['slot'] for s in samplers}) == len(samplers) <= 16, 'duplicate/excess sampler slots')
    packet += u32(len(samplers))
    for sampler in sorted(samplers, key=lambda s: s['slot']):
        packet += u32(sampler['slot'])
        packet += struct.pack('<4If2I6f', sampler['filter'], *sampler['address'], sampler['mip_bias'],
                              sampler['max_anisotropy'], sampler['comparison'], *sampler['border'],
                              sampler['min_lod'], sampler['max_lod'])
    before, after = read(Path(stem + '-before.rt00.bin')), read(Path(stem + '-output.rt00.bin'))
    require(len(before) == len(after) == width * height * pixel_bytes, 'target payload extent mismatch')
    packet += blob(before)
    output.mkdir(parents=True)
    (output / 'packet.bin').write_bytes(packet)
    (output / 'reference.bin').write_bytes(after)
    receipt = dict(source_draw=str(draw), pixel_shader_sha256=ps_sha, vertex_shader_sha256=VERTEX_SHA,
                   width=width, height=height, pixel_bytes=pixel_bytes, format=target['format'],
                   packet_sha256=hashlib.sha256(packet).hexdigest(), reference_sha256=hashlib.sha256(after).hexdigest(),
                   unbound_texture_slots=sorted(active_textures - {t['slot'] for t in targets}),
                   unbound_sampler_slots=sorted(active_samplers - {s['slot'] for s in samplers}), files=hashes,
                   substitutions=list(substitutions.values()),
                   geometry_source=str(geometry_draw or draw),
                   geometry_inherited=presentation,
                   scope=('Original shaders with explicitly substituted captured textures; input sensitivity only, '
                          'not native integration or renderer parity. Original output retained as comparison baseline.'
                          if substitutions else
                          'Original presentation with the same frame\'s post quad; geometry reuse requires a GPU comparison.'
                          if presentation else 'Prepared original-pass replay, not yet GPU executed or compared.'))
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return {k: v for k, v in receipt.items() if k != 'files'}


def compare(prepared, rendered):
    receipt = json.loads((prepared / 'receipt.json').read_text())
    reference = (prepared / 'reference.bin').read_bytes()
    require(hashlib.sha256(reference).hexdigest() == receipt['reference_sha256'], 'reference changed')
    require(hashlib.sha256((prepared / 'packet.bin').read_bytes()).hexdigest() == receipt['packet_sha256'], 'replay inputs changed')
    replay = rendered.read_bytes()
    require(len(replay) == len(reference), 'rendered target extent differs')
    different = sum(a != b for a, b in zip(reference, replay))
    result = dict(byte_identical=reference == replay, differing_bytes=different, total_bytes=len(reference),
                  source_shader_sha256=receipt['pixel_shader_sha256'], rendered_sha256=hashlib.sha256(replay).hexdigest(),
                  scope=('Input substitution compared to the original output; differences are expected. '
                         'Not native integration or renderer parity.' if receipt.get('substitutions') else
                         'One post draw with captured inputs; not whole-renderer or moving-rain parity.'))
    if receipt['format'] in (2, 10):
        code = 'f' if receipt['format'] == 2 else 'e'
        pairs = zip(struct.iter_unpack('<' + code, reference), struct.iter_unpack('<' + code, replay))
        maximum, total, nonfinite, count = 0.0, 0.0, 0, 0
        for (a,), (b,) in pairs:
            count += 1
            if not math.isfinite(a) or not math.isfinite(b):
                nonfinite += 1
            else:
                difference = abs(a - b)
                maximum = max(maximum, difference)
                total += difference
        result.update(max_absolute_component_error=maximum, mean_absolute_component_error=total / count,
                      nonfinite_component_pairs=nonfinite)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    p = sub.add_parser('prepare')
    p.add_argument('draw', type=Path)
    p.add_argument('fixture', type=Path)
    p.add_argument('output', type=Path)
    p.add_argument('--texture', nargs=2, action='append', default=[], metavar=('NAME', 'CAPTURE'),
                   help='sensitivity experiment: substitute this named texture from another draw')
    p.add_argument('--geometry-from', type=Path,
                   help='for presentation only: reuse this same frame\'s original post quad')
    p = sub.add_parser('compare')
    p.add_argument('prepared', type=Path)
    p.add_argument('rendered', type=Path)
    args = parser.parse_args()
    if args.operation == 'prepare':
        overrides = {name: Path(capture) for name, capture in args.texture}
        require(len(overrides) == len(args.texture), 'duplicate texture substitution')
        result = prepare(args.draw, args.fixture, args.output, overrides, args.geometry_from)
    else:
        result = compare(args.prepared, args.rendered)
    print(json.dumps(result, indent=2))
    if args.operation == 'compare' and not result['byte_identical']:
        raise SystemExit(1)
