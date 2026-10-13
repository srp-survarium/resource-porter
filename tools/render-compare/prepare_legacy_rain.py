#!/usr/bin/env python3
"""Prepare exact source rain assets; this does not activate a target renderer."""
import argparse
import hashlib
import json
import math
import shutil
import struct
from pathlib import Path
from audit_reference_rain import audit


def prepare(report, output):
    capture = audit(report)
    if len(capture['layers']) != 5:
        raise ValueError('expected original Factory five-layer recipe')
    prefix = str(report).removesuffix('.rain-counts.json') + '.rain-draw'
    vertex = Path(prefix + '.vs.dxbc').read_bytes()
    if len(vertex) != 1708 or vertex[4:20].hex() != 'f3cc7d8b745ee9cf1cdc476891bb1dae':
        raise ValueError('unexpected original rain vertex contract')
    metadata = json.loads(Path(prefix + '.targets.json').read_text())
    texture = next(item for item in metadata if item['slot'] == 1)
    if (texture['width'], texture['height'], texture['view_format']) != (512, 512, 78):
        raise ValueError('expected original 512-square sRGB BC3 rain texture')
    if Path(prefix + '.rt01.bin').stat().st_size != 262144:
        raise ValueError('rain texture mip-zero size differs')
    vertices = list(struct.iter_unpack('<6f', Path(prefix + '.vertices.bin').read_bytes()))
    indices = list(struct.unpack('<432H', Path(prefix + '.indices.bin').read_bytes()))
    if len(vertices) != 100:
        raise ValueError('unexpected source sphere vertex count')
    mesh_error = 0
    expected_indices = []
    for side in range(25):
        yaw = side * math.tau / 24
        for ring in range(4):
            angle = ring * math.pi / 3
            expected = [math.sin(angle)*math.cos(yaw), math.cos(angle), math.sin(angle)*math.sin(yaw),
                        1, side/24, ring/3]
            mesh_error = max(mesh_error, max(abs(a-b) for a,b in zip(vertices[side*4+ring],expected,strict=True)))
            if side < 24 and ring < 3:
                a, b = side*4+ring, (side+1)*4+ring
                expected_indices.extend([a,a+1,b,b,a+1,b+1])
    if mesh_error > 1e-6 or indices != expected_indices:
        raise ValueError('captured geometry does not reproduce sphere_geometry(24,3)')
    output.mkdir(parents=True, exist_ok=False)
    files = {'.dxbc':'rain.ps.dxbc', '.vs.dxbc':'rain.vs.dxbc', '.vertices.bin':'vertices.bin',
             '.indices.bin':'indices.bin', '.rt01.bin':'scroll-rain-bc3-srgb.bin'}
    for suffix, name in files.items():
        shutil.copy2(Path(prefix + suffix),output/name)
    recipe = {'scope':'Factory 0.10b layered rain; asset preparation only, target rendering not enabled',
              'source_report':str(report),'vertex_stride':24,'vertex_count':100,'index_count':432,
              'index_format':57,'texture':{'width':512,'height':512,'format':78,'mip_levels':1},
              'first_layer':5,'end_layer':10,'radius_scale':1.1,'density':.8,'speed':2,'uv_scale':[1,1],
              'angle_x':0,'angle_y':0,'rotation_radius':.1,'base_offset':.1,'rotation_speed':.2,
              'geometry_max_error':mesh_error,
              'required_dynamic_inputs':['camera-relative origin with original movement offsets','scene time',
                                         'view/projection matrices','frame depth','pre-rain color',
                                         'original encoded diffuse lighting','original rain shadow projection/map']}
    (output/'recipe.json').write_text(json.dumps(recipe,indent=2)+'\n')
    manifest=[{'path':p.name,'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
              for p in sorted(output.iterdir())]
    (output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return {'output':str(output),'files':len(manifest),'geometry_max_error':mesh_error}


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(prepare(args.report,args.output),indent=2))
