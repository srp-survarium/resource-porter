#!/usr/bin/env python3
"""Compare source cube mip hypotheses on geometry covered only by the global probe."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

from audit_local_light_authored import constants
from audit_translucency_auxiliary import normalize, rgb, saturate

CONE = ((.99725014,-.029667195,.06791174),(.948293,-.029853212,-.31598923),
        (.89564174,-.095602594,.43438),(.8242554,.55417657,.11615242),
        (.7863557,-.6177587,.004354838),(.7716971,.4068773,-.48880917),
        (.75259656,-.34534776,-.56065434),(.7230467,.28388765,.6297708))


class Cube:
    def __init__(self, path):
        self.raw = path.read_bytes()
        self.width = struct.unpack_from('<I', self.raw, 16)[0]
        self.mips = struct.unpack_from('<I', self.raw, 28)[0]
        if self.raw[:4] != b'DDS ' or self.raw[84:88] != b'DXT5':
            raise ValueError('audit requires source BC3 DDS')
        self.face_size = sum(max(1, self.width >> m >> 2)**2*16 for m in range(self.mips))
        if len(self.raw) != 128+6*self.face_size+1:
            raise ValueError('source cube length changed')
        self.cache = {}

    def texel(self, face, mip, x, y):
        dim = self.width >> mip
        key = face, mip, x//4, y//4
        if key not in self.cache:
            offset = 128+face*self.face_size+sum(max(1, self.width >> m >> 2)**2*16 for m in range(mip))
            offset += ((y//4)*max(1, dim//4)+x//4)*16
            block = self.raw[offset:offset+16]
            a0, a1 = block[:2]
            alpha = [a0, a1]
            alpha += [((7-i)*a0+i*a1)/7 for i in range(1,7)] if a0 > a1 else [((5-i)*a0+i*a1)/5 for i in range(1,5)]+[0,255]
            def color(value):
                r,g,b = value >> 11, (value >> 5) & 63, value & 31
                return ((r << 3)|(r >> 2), (g << 2)|(g >> 4), (b << 3)|(b >> 2))
            c0,c1 = [color(v) for v in struct.unpack_from('<HH',block,8)]
            colors = [c0,c1,tuple((2*a+b)/3 for a,b in zip(c0,c1)),tuple((a+2*b)/3 for a,b in zip(c0,c1))]
            self.cache[key] = colors, alpha, int.from_bytes(block[2:8],'little'), int.from_bytes(block[12:16],'little')
        colors, alpha, ai, ci = self.cache[key]
        i=(y%4)*4+x%4
        return [v/255 for v in colors[(ci >> (2*i)) & 3]]+[alpha[(ai >> (3*i)) & 7]/255]

    def sample(self, direction, mip):
        mip = min(mip, self.mips-1)
        x,y,z = direction
        axis=max(range(3),key=lambda i:abs(direction[i]))
        if axis == 0: face,sc,tc,ma = (0,-z,-y,x) if x >= 0 else (1,z,-y,-x)
        elif axis == 1: face,sc,tc,ma = (2,x,z,y) if y >= 0 else (3,x,-z,-y)
        else: face,sc,tc,ma = (4,x,-y,z) if z >= 0 else (5,-x,-y,-z)
        dim = self.width >> mip
        u,v = ((sc/ma+1)*dim*.5-.5,(tc/ma+1)*dim*.5-.5)
        ix,iy = math.floor(u),math.floor(v)
        # Three-face cube corners remain excluded; ordinary edges map to
        # the corresponding texel in the adjacent face, never clamp.
        if (ix < 0 or ix+1 >= dim) and (iy < 0 or iy+1 >= dim):
            return None
        out=[0.]*4
        for dy in (0,1):
            for dx in (0,1):
                weight=(u-ix if dx else 1-u+ix)*(v-iy if dy else 1-v+iy)
                tx,ty=ix+dx,iy+dy;tf=face
                if not (0<=tx<dim and 0<=ty<dim):
                    u2,v2=2*(tx+.5)/dim-1,2*(ty+.5)/dim-1
                    direction=((1,-v2,-u2),(-1,-v2,u2),(u2,1,v2),(u2,-1,-v2),(u2,-v2,1),(-u2,-v2,-1))[face]
                    xx,yy,zz=direction;ax=max(range(3),key=lambda i:abs(direction[i]))
                    if ax==0:tf,ss,tt,mm=(0,-zz,-yy,xx) if xx>=0 else (1,zz,-yy,-xx)
                    elif ax==1:tf,ss,tt,mm=(2,xx,zz,yy) if yy>=0 else (3,xx,-zz,-yy)
                    else:tf,ss,tt,mm=(4,xx,-yy,zz) if zz>=0 else (5,-xx,-yy,-zz)
                    tx,ty=min(dim-1,max(0,int((ss/mm+1)*dim*.5))),min(dim-1,max(0,int((tt/mm+1)*dim*.5)))
                for c,value in enumerate(self.texel(tf,mip,tx,ty)):
                    out[c]+=weight*value
        return [abs(v/out[3])**2.2 for v in out[:3]]


def audit(path,cube,stride,probes):
    digest,cb=constants(path)
    if digest != 'cc45055d632b75e94160d8a1595b5393':
        raise ValueError('unverified source probe variant')
    stem=str(path).removesuffix('.probe-draw.dxbc')
    def read(suffix): return Path(stem+suffix).read_bytes()
    views=json.loads(read('.probe-draw.targets.json'))
    if (views[3]['width'],views[3]['mip_levels'],views[3]['view_most_detailed_mip'])!=(cube.width,cube.mips,0):
        raise ValueError('source cube SRV differs from DDS')
    sampler=json.loads(read('.probe-draw.samplers.json'))[6]
    if sampler['slot']!=6 or sampler['filter']!=21 or sampler['min_lod']!=0 or sampler['max_lod']<cube.mips:
        raise ValueError('source diffuse sampler contract changed')
    uploaded=struct.unpack('<I',struct.pack('<f',cb['num_mips'][0]))[0]
    requested=uploaded-2;actual=min(requested,cube.mips-1)
    if cube.width >> actual != 4: raise ValueError('source diffuse does not select 4x4')
    depth,normal,albedo,ao=[read(f'.probe-draw.rt{i:02d}.bin') for i in (0,1,2,4)]
    before,after=read('.probe-before.rt00.bin'),read('.sun-before.rt00.bin')
    if before != read('.skylight-output.rt00.bin'): raise ValueError('first-probe input is not initial sky')
    ray=cb['s_eye_ray_corner'];matrix=cb['m_V2W'];center=[cb['m_W'][i*4+3] for i in range(3)]
    groups={name:{'samples':0,'maximum_error':0.,'sum_error':0.,'outside_one_step':0} for name in ('source_4x4','incorrect_8x8')}
    excluded={'unchanged':0,'cube_corners':0,'other_probe_bounds':0,'sky_depth':0}
    for i in range(0,1280*720,stride):
        if before[i*4:i*4+4]==after[i*4:i*4+4]:excluded['unchanged']+=1;continue
        x,y=i%1280,i//1280;z=struct.unpack_from('<e',depth,i*2)[0]
        if not 0<z<499.75:excluded['sky_depth']+=1;continue
        position=[(1-2*(x+.5)/1280)*ray[0]/ray[2]*z,(1-2*(y+.5)/720)*ray[1]/ray[2]*z,z,1]
        world=[sum(matrix[r*4+c]*position[c] for c in range(4)) for r in range(3)]
        if any(math.dist(world,probe['position']) <= (probe['radius'] if not probe['geometry'] else math.sqrt(sum(v*v for v in probe['scale']))) for probe in probes if math.dist(probe['position'],center)>.001):
            excluded['other_probe_bounds']+=1;continue
        view=normalize([a-b for a,b in zip(cb['eye_position'],world)])
        packed=struct.unpack_from('<I',normal,i*4)[0]
        f=[(packed >> shift & 1023)/1023*4-2 for shift in (0,10)];radius=sum(v*v for v in f)
        n=normalize([f[0]*math.sqrt(abs(1-radius*.25)),f[1]*math.sqrt(abs(1-radius*.25)),radius*.5-1])
        n=normalize([sum(matrix[r*4+c]*n[c] for c in range(3)) for r in range(3)])
        gloss=(packed >> 20 & 127)/127;f0=(albedo[i*4+3]&127)/127
        dot=sum(a*b for a,b in zip(view,n));reflection=[2*dot*a-b for a,b in zip(n,view)]
        half=normalize([a+b for a,b in zip(n,reflection)])
        mirror=normalize([a*(1-(1-gloss)**.8)+b*(1-gloss)**.8 for a,b in zip(half,reflection)])
        bent=normalize([a+b for a,b in zip(n,mirror)])
        light=[a*(1-gloss)+b*gloss for a,b in zip(mirror,bent)]
        halfway=normalize([a+b for a,b in zip(light,view)])
        fresnel=f0+(1-f0)*(1-max(0,sum(a*b for a,b in zip(view,halfway))))**5
        attenuation=math.exp(-8*(math.dist(world,center)/(cb['light_range'][0]*.95))**8)
        directions=[normalize([sum(a*b for a,b in zip(row,n)) for row in (cone,(cone[2],cone[0],cone[1]),(cone[1],cone[2],cone[0]))]) for cone in CONE]
        samples=[[cube.sample(d,m) for d in directions] for m in (actual,actual-1)]
        if any(v is None for values in samples for v in values):excluded['cube_corners']+=1;continue
        for name,values in zip(groups,samples):
            diffuse=[sum(v[c] for v in values)/9 for c in range(3)]
            encoded=[math.sqrt(saturate(min(v*ao[i*2]/255*2.5*cb['color_parameters'][0],10)*(1-fresnel*(1-gloss))*.125)) for v in diffuse]
            predicted=[v*attenuation+b*(1-attenuation) for v,b in zip(encoded,rgb(before,i))]
            errors=[abs(a-b) for a,b in zip(predicted,rgb(after,i))]
            s=groups[name];s['samples']+=1;s['sum_error']+=sum(errors)/3;s['maximum_error']=max(s['maximum_error'],max(errors));s['outside_one_step']+=max(errors)>1/1023+1e-6
    for s in groups.values():s['rgb_mae']=s.pop('sum_error')/s['samples'] if s['samples'] else None
    return {'pose':Path(stem).name,'uploaded_num_mips':uploaded,'requested_mip':requested,'effective_mip':actual,'excluded':excluded,'equations':groups}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('original',type=Path)
    p.add_argument('--project',type=Path,required=True);p.add_argument('--cube',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--stride',type=int,default=31)
    a=p.parse_args()
    if a.stride<1:p.error('stride must be positive')
    cube=Cube(a.cube)
    def probes(v):
        if isinstance(v,dict):
            if v.get('game_object_type')=='environment_probe':yield v
            else:
                for child in v.values():yield from probes(child)
        elif isinstance(v,list):
            for child in v:yield from probes(child)
    authored=list(probes(json.loads(a.project.read_text())))
    rows=[audit(path,cube,a.stride,authored) for path in sorted(a.original.glob('*.probe-draw.dxbc'))]
    if not rows:p.error('no original probe captures')
    report={'scope':'Original final ambient versus initial sky on geometry outside conservative bounds of every other authored probe. Excludes cube corners and unchanged pixels; ordinary edges map adjacent-face texels. First captured probe pass has no changed pixels. CPU filtering/raster precision remains approximate: outside-one-step counts must not be presented as an exact equation pass.', 'cube_sha256':hashlib.sha256(cube.raw).hexdigest(),'stride':a.stride,'views':rows}
    a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'report':str(a.output),'views':len(rows),'samples':sum(row['equations']['source_4x4']['samples'] for row in rows)}))


if __name__=='__main__':main()
