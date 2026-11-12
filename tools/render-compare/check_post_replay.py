#!/usr/bin/env python3
"""Exercise post replay preparation with real original bytecode and synthetic data.

This checks rejection of incomplete captures and altered programs; it does not
run a GPU or provide visual parity evidence. Pass the extracted 0.11e fixture.
"""
import argparse
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

from replay_post import prepare, compare, VERTEX_SHA

POST_SHA = '8696aa4bd4c7ca1091957cd27659b1658e0a461d1a7c8014f2ba82a19fcae105'


def check(fixture):
    source = json.loads((fixture / 'manifest.json').read_text())['shaders']
    with tempfile.TemporaryDirectory(prefix='post-replay-check-') as temporary:
        root = Path(temporary)
        draw = root / 'synthetic.bmp.post-draw'

        def write(suffix, data):
            path = Path(str(draw) + suffix)
            path.write_bytes(data if isinstance(data, bytes) else json.dumps(data).encode())
            return path

        write('.dxbc', (fixture / source[POST_SHA]['file']).read_bytes())
        write('.vs.dxbc', (fixture / source[VERTEX_SHA]['file']).read_bytes())
        write('.vs.cb0.bin', bytes(304))
        write('.cb0.bin', bytes(304))
        write('.cb1.bin', bytes(304))
        geometry = dict(indexed=1, count=6, start=1, base_vertex=1, stride=28,
                        vertex_offset=8, index_offset=2, index_format=57, topology=4)
        write('.geometry.json', geometry)
        vertices = bytes(8 + 28)
        for x, y, u, v in [(0, 4, 0, 1), (0, 0, 0, 0), (4, 4, 1, 1), (4, 0, 1, 0)]:
            vertices += struct.pack('<4fI2f', x, y, 0, 1, 0xffffffff, u, v)
        write('.vertices.bin', vertices)
        write('.indices.bin', bytes(4) + struct.pack('<6H', 0, 1, 2, 2, 1, 3))
        state = dict(viewport_count=1, viewport=[0, 0, 4, 4, 0, 1], target_dimension=4,
                     target_view_format=10, fill_mode=3, cull_mode=1, front_ccw=False,
                     depth_clip=True, scissor=False, depth_bias=0, depth_bias_clamp=0,
                     slope_bias=0, depth_bound=False, blend_enabled=False, write_mask=15, sample_mask=0xffffffff)
        write('.post-state.json', state)
        target = [dict(slot=0, name='draw_target', width=4, height=4, format=10,
                       array_size=1, mip_levels=1, bytes_per_pixel=8, written=True, hresult=0)]
        for phase in ('before', 'after'):
            (root / f'synthetic.bmp.post-{phase}.targets.json').write_text(json.dumps(target))
            (root / f'synthetic.bmp.post-{phase}.rt00.bin').write_bytes(bytes(4 * 4 * 8))
        textures = []
        for slot in range(8):
            compressed, volume = slot in (1, 5), slot == 7
            dimension, width, height, depth = (8, 2, 2, 2) if volume else (4, 4, 4, 1)
            mips = 3 if compressed else 1
            texture = dict(slot=slot, width=width, height=height, format=71 if compressed else 28,
                           array_size=1, mip_levels=mips, view_format=71 if compressed else 28,
                           view_dimension=dimension, view_most_detailed_mip=0, view_mip_levels=mips, written=True)
            if compressed:
                texture.update(block_width=4, block_height=4, bytes_per_block=8)
            else:
                texture['bytes_per_pixel'] = 4
            if volume:
                texture['depth'] = depth
            for mip in range(mips):
                suffix = f'.vol{slot:02}.bin' if volume else (
                    f'.rt{slot:02}.bin' if mip == 0 else f'.rt{slot:02}.mip{mip:02}.bin')
                write(suffix, bytes(8 if compressed else width * height * depth * 4))
            textures.append(texture)
        write('.targets.json', textures)
        write('.samplers.json', [dict(slot=i, filter=0, address=[3, 3, 3], mip_bias=0,
                                      max_anisotropy=1, comparison=1, border=[0, 0, 0, 0],
                                      min_lod=0, max_lod=3.40282347e38) for i in range(5)])
        receipt = prepare(draw, fixture, root / 'valid')
        assert not receipt['unbound_texture_slots'] and not receipt['unbound_sampler_slots']
        rejected = []

        def reject(label, path, changed):
            original = path.read_bytes()
            if changed is None:
                path.unlink()
            else:
                path.write_bytes(changed)
            destination = root / label
            try:
                prepare(draw, fixture, destination)
            except (ValueError, FileNotFoundError):
                assert not destination.exists(), 'failed preparation left a usable packet'
                rejected.append(label)
            else:
                raise AssertionError(label + ' was accepted')
            finally:
                path.write_bytes(original)

        reject('missing-grain-mip', Path(str(draw) + '.rt01.mip01.bin'), None)
        reject('missing-vertex-constants', Path(str(draw) + '.vs.cb0.bin'), None)
        reject('altered-shader', Path(str(draw) + '.dxbc'), b'not the original shader')
        reject('vertex-out-of-bounds', Path(str(draw) + '.indices.bin'), bytes(4) + struct.pack('<6H', *([65535] * 6)))
        reject('partial-viewport', Path(str(draw) + '.post-state.json'), json.dumps(dict(state, viewport=[0, 0, 2, 4, 0, 1])).encode())
        invalid = [dict(t) for t in textures]
        invalid[0]['bytes_per_pixel'] = 16
        reject('wrong-format-size', Path(str(draw) + '.targets.json'), json.dumps(invalid).encode())
        prepared = root / 'valid'
        rendered = root / 'synthetic-output.bin'
        rendered.write_bytes((prepared / 'reference.bin').read_bytes())
        assert compare(prepared, rendered)['byte_identical']
        command = [sys.executable, str(Path(__file__).with_name('replay_post.py')), 'compare', str(prepared), str(rendered)]
        assert subprocess.run(command, capture_output=True, check=False).returncode == 0
        altered = bytearray(rendered.read_bytes());altered[:2] = struct.pack('<e', 1)
        rendered.write_bytes(altered)
        result = compare(prepared, rendered)
        assert not result['byte_identical'] and result['max_absolute_component_error'] == 1
        assert subprocess.run(command, capture_output=True, check=False).returncode == 1
        (prepared / 'packet.bin').write_bytes(b'changed')
        try:
            compare(prepared, rendered)
        except ValueError:
            rejected.append('changed-replay-packet')
        else:
            raise AssertionError('changed replay inputs accepted')
        return dict(valid_packet_prepared=True, rejected=rejected,
                    scope='Synthetic capture data with real archived shaders; CPU preparation/comparator checks only.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('fixture', type=Path)
    print(json.dumps(check(parser.parse_args().fixture), indent=2))
