#!/usr/bin/env python3
"""Replay captured 010b skylight RGB and export its exact weather constants."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct


def clamp(value):
    return max(0., min(1., value))


def audit(base, stride):
    def read(suffix):
        return Path(str(base) + suffix).read_bytes()
    shader = read('.skylight-draw.dxbc')
    if len(shader)!=3908 or shader[:4]!=b'DXBC' or shader[4:20].hex() != '21c28f59ef738c2db31065650ff8249f':
        raise ValueError(f'{base}: unsupported original skylight identity')
    info = json.loads(read('.skylight-draw.targets.json'))
    if [(v['format'], v['width'], v['height']) for v in info] != [(54,1280,720),(24,1280,720),(49,1280,720)]:
        raise ValueError(f'{base}: input contract changed')
    if json.loads(read('.skylight-draw.blend.json'))['enabled']:
        raise ValueError(f'{base}: unexpected blending')
    cb0, cb1 = read('.skylight-draw.cb0.bin'), read('.skylight-draw.cb1.bin')
    constants = cb0[32:160]
    p = [struct.unpack_from('<4f', constants, i*16) for i in range(8)]
    eye = struct.unpack_from('<3f', cb0, 16)
    matrix = [struct.unpack_from('<4f', cb1, 176+i*16) for i in range(3)]
    depth, normals, ao = [read(f'.skylight-draw.rt{i:02d}.bin') for i in range(3)]
    observed = read('.skylight-output.rt00.bin')
    before = read('.skylight-before.rt00.bin')
    angle = math.acos(p[6][0])+math.pi if -p[6][2]>0 else math.acos(-p[6][0])
    cosine, sine = math.cos(angle), math.sin(angle)
    count = failures = unchanged = changed_failures = 0
    total = maximum = changed_total = changed_maximum = 0.
    for y in range(stride//2, 720, stride):
        for x in range(stride//2, 1280, stride):
            index = y*1280+x
            packed = struct.unpack_from('<I', normals, index*4)[0]
            f = [(packed & 1023)/1023*4-2, ((packed>>10)&1023)/1023*4-2]
            radius = sum(v*v for v in f)
            normal = [f[0]*math.sqrt(abs(1-radius*.25)), f[1]*math.sqrt(abs(1-radius*.25)), radius*.5-1]
            length = math.sqrt(sum(v*v for v in normal))
            normal = [v/length for v in normal]
            world = [sum(row[i]*normal[i] for i in range(3)) for row in matrix]
            basis = [world[0]*cosine-world[2]*sine, world[1], world[0]*sine+world[2]*cosine]
            cube = [p[2][:3] if basis[0]<0 else p[3][:3],
                    (p[5][3],p[6][3],p[7][3]) if basis[1]<0 else p[4][:3],
                    (p[2][3],p[3][3],p[4][3]) if basis[2]<0 else p[5][:3]]
            wrap = clamp(-sum(world[i]*p[6][i] for i in range(3)))
            color = [sum(basis[i]**2*cube[i][c] for i in range(3))+wrap*p[7][c] for c in range(3)]
            z = struct.unpack_from('<e', depth, index*2)[0]
            position = [(1-2*(x+.5)/1280)*eye[0]/eye[2]*z, (1-2*(y+.5)/720)*eye[1]/eye[2]*z,z]
            height = sum(matrix[1][i]*position[i] for i in range(3))+matrix[1][3]
            ramp = clamp((height-p[1][0])/p[1][1])**p[1][2]
            predicted = [math.sqrt(clamp(color[c]*ramp*ao[index*2]/255*2.5*p[0][c]*.125)) for c in range(3)]
            result = struct.unpack_from('<I', observed,index*4)[0]
            errors = [abs(predicted[c]-((result>>(10*c))&1023)/1023) for c in range(3)]
            error = max(errors)
            total += sum(errors); maximum = max(maximum,error); count += 1
            failures += error>1/1023+1e-6
            same = observed[index*4:index*4+4]==before[index*4:index*4+4]
            unchanged += same
            if not same:
                changed_total += sum(errors); changed_maximum = max(changed_maximum,error)
                changed_failures += error>1/1023+1e-6
    return {'pose':str(base),'samples':count,'mean_absolute_rgb_error':total/(3*count),
            'maximum_channel_error':maximum,'outside_one_10bit_step':failures,
            'unchanged_from_before':unchanged,
            'changed_pixels':{'samples':count-unchanged,
                'mean_absolute_rgb_error':changed_total/(3*(count-unchanged)) if count!=unchanged else None,
                'maximum_channel_error':changed_maximum,'outside_one_10bit_step':changed_failures},
            'constants_sha256':hashlib.sha256(constants).hexdigest()}, constants


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory',type=Path)
    parser.add_argument('--stride',type=int,default=8)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--constants',type=Path,help='Export one matching 128-byte source sky CB; refuses mixed weather')
    args=parser.parse_args()
    if args.stride<1: parser.error('stride must be positive')
    files=sorted(args.directory.glob('*.bmp.skylight-draw.dxbc'))
    if not files: parser.error('no original skylight captures found')
    rows=[]; constants=[]
    for path in files:
        row,cb=audit(Path(str(path).removesuffix('.skylight-draw.dxbc')),args.stride)
        rows.append(row); constants.append(cb)
    if args.constants:
        if any(cb!=constants[0] for cb in constants): raise ValueError('source weather constants differ')
        args.constants.write_bytes(constants[0])
    args.output.write_text(json.dumps({'scope':'010b source skylight replay; not target parity or stencil validation','views':rows},indent=2)+'\n')
    print(json.dumps(rows,indent=2))


if __name__=='__main__':
    main()
