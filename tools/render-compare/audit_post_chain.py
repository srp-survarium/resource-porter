#!/usr/bin/env python3
"""Experimental CPU post-chain audit; strict one-step GPU agreement is unresolved."""
import argparse
import json
import math
from pathlib import Path
import struct

from audit_surface_light import REGIONS
from compare_composition import unsigned_float

L11=[unsigned_float(i,6) for i in range(2048)]
L10=[unsigned_float(i,5) for i in range(1024)]
def rgb(data,index,fmt=26):
    if fmt==26:
        v=struct.unpack_from('<I',data,index*4)[0]
        return L11[v&2047],L11[(v>>11)&2047],L10[v>>22]
    if fmt==10:return struct.unpack_from('<4e',data,index*8)[:3]
    raise ValueError(fmt)

def bilinear(data,w,h,u,v,fmt,wrap=False):
    x,y=u*w-.5,v*h-.5;ix,iy=math.floor(x),math.floor(y);fx,fy=x-ix,y-iy
    out=[0.,0.,0.]
    for dy,wy in [(0,1-fy),(1,fy)]:
        for dx,wx in [(0,1-fx),(1,fx)]:
            sx=(ix+dx)%w if wrap else max(0,min(w-1,ix+dx))
            sy=(iy+dy)%h if wrap else max(0,min(h-1,iy+dy))
            p=rgb(data,sy*w+sx,fmt)
            for c in range(3):out[c]+=wx*wy*p[c]
    return out

def grade(lut,color):
    # texture coordinates color*15/16+.5/16 map to texel coordinates color*15.
    position=[max(0,min(15,v*15)) for v in color]
    origin=[math.floor(v) for v in position];fraction=[v-i for v,i in zip(position,origin)]
    out=[0.,0.,0.]
    for z in range(2):
        for y in range(2):
            for x in range(2):
                delta=(x,y,z);weight=math.prod(fraction[a] if delta[a] else 1-fraction[a] for a in range(3))
                i=((min(origin[2]+z,15)*16+min(origin[1]+y,15))*16+min(origin[0]+x,15))*4
                for c in range(3):out[c]+=weight*lut[i+2-c]/255
    return out

def bc1_noise(dds):
    assert dds[:4]==b'DDS ' and dds[84:88]==b'DXT1' and struct.unpack_from('<II',dds,12)==(64,64)
    pixels=[None]*4096
    for by in range(16):
        for bx in range(16):
            a,b,indices=struct.unpack_from('<HHI',dds,128+(by*16+bx)*8)
            def color(v):return ((v>>11)/31,((v>>5)&63)/63,(v&31)/31)
            ca,cb=color(a),color(b)
            palette=[ca,cb,tuple((2*x+y)/3 for x,y in zip(ca,cb)),tuple((x+2*y)/3 for x,y in zip(ca,cb))] if a>b else [ca,cb,tuple((x+y)/2 for x,y in zip(ca,cb)),(0,0,0)]
            for y in range(4):
                for x in range(4):pixels[(by*4+y)*64+bx*4+x]=palette[(indices>>(2*(y*4+x)))&3]
    return pixels

def audit(folder,noise):
    results={}
    for pose,regions in REGIONS.items():
        base=folder/(pose+'.bmp')
        def read(s):return Path(str(base)+s).read_bytes()
        targets={v['slot']:v for v in json.loads(read('.post-draw.targets.json'))}
        assert all(targets[i]['written'] for i in range(5))
        assert targets[1]['format']==87 and (targets[1]['width'],targets[1]['height'],targets[1]['depth'])==(16,16,16)
        samplers=json.loads(read('.post-draw.samplers.json'))
        assert [(s['filter'],s['address']) for s in samplers]==[(21,[1,1,1]),(0,[3,3,3]),(21,[3,3,3])]
        frame,lut,lum,shafts,bloom=[read('.post-draw.'+('vol' if i==1 else 'rt')+f'{i:02d}.bin') for i in range(5)]
        output=read('.post-output.rt00.bin');copy_input=read('.present-draw.rt00.bin')
        cb=read('.post-draw.cb0.bin');grain=struct.unpack_from('<4f',cb,176);mix=struct.unpack_from('<f',cb,288)[0]
        average=struct.unpack_from('<f',lum)[0];exposure=(.25/max(.09,min(.4,average)))**.2
        gamma=struct.unpack_from('<f',read('.present-draw.cb0.bin'))[0];assert gamma==1
        assert output==copy_input,'another pass changes the post output before gamma copy'
        results[pose]={'average_luminance':average,'exposure':exposure,'post_to_copy_input_identical':True,'regions':{}}
        for name,(x0,y0,x1,y1) in regions.items():
            sums={k:0. for k in ['current','source_gamma_tail','source_grain_and_gamma_tail','tone_input','shaft_input']}
            outside=0;maximum=0.;count=0
            for y in range(y0,y1):
                for x in range(x0,x1):
                    i=y*1280+x;uv=((x+.5)/1280,(y+.5)/720)
                    f=rgb(frame,i);b=bilinear(bloom,targets[4]['width'],targets[4]['height'],*uv,10)
                    sh=bilinear(shafts,targets[3]['width'],targets[3]['height'],*uv,26,wrap=True)
                    hdr=[(1-mix)*a+mix*c+s for a,c,s in zip(f,b,sh)]
                    tone=[]
                    for v in hdr:
                        q=max(v*exposure-.0015,0);tone.append(((q*(6.2*q+.2))/(q*(5.9*q+1.5)+.04))**2.2)
                    vig=(1-((uv[0]-.5)**2+(uv[1]-.5)**2))**(grain[3]+1) if grain[3]>.1 else 1
                    current=[((abs(v)+.055)/1.055)**2.4*vig for v in grade(lut,[1.055*v**(1/2.4)-.055 for v in tone])]
                    bits=struct.unpack_from('<I',output,4*i)[0]
                    actual=rgb(output,i)
                    for c,(shift,mask,table) in enumerate([(0,2047,L11),(11,2047,L11),(22,1023,L10)]):
                        stored=(bits>>shift)&mask;step=table[stored+1]-table[stored];err=abs(current[c]-actual[c])
                        maximum=max(maximum,err);outside+=err>max(step,1e-8)
                    old=grade(lut,[v**(1/2.2) for v in tone])
                    # Counterfactual phase follows this frame's offsets; the frozen
                    # old frame's grain phase is unknown. Original material scale .08.
                    nx=math.floor((uv[0]+grain[0]*2-1)*1280)%64
                    ny=math.floor((uv[1]+grain[1]*2-1)*720)%64
                    noisy=[min(1,max(0,v+n*.08*v**.6)) for v,n in zip(tone,noise[ny*64+nx])]
                    old_noise=grade(lut,[v**(1/2.2) for v in noisy])
                    def luma(c):return sum(a*b for a,b in zip(c,(.2126,.7152,.0722)))
                    sums['current']+=luma([max(0,min(1,1.055*v**(1/2.4)-.055)) for v in actual])
                    sums['source_gamma_tail']+=luma([v*vig**(1/2.2) for v in old])
                    sums['source_grain_and_gamma_tail']+=luma([v*vig**(1/2.2) for v in old_noise])
                    sums['tone_input']+=luma(hdr);sums['shaft_input']+=luma(sh);count+=1
            results[pose]['regions'][name]={'pixels_verified':count,'channels_outside_one_step':outside,'maximum_absolute_error':maximum,'means':{k:v/count for k,v in sums.items()}}
    return results

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('capture',type=Path);parser.add_argument('--noise',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    result=audit(args.capture,bc1_noise(args.noise.read_bytes()))
    args.output.write_text(json.dumps({'verified':False,'scope':'This is not a validated post-process replay. Fixed-region CPU audit of the current post chain; tail counterfactuals hold bloom, geometry and tone input fixed. Original grain phase unknown; BC1 interpolation is approximate. Source desaturation, DOF and bloom producers are not reproduced.','poses':result},indent=2)+'\n')
    print(json.dumps(result,indent=2))
    if any(r['channels_outside_one_step'] for p in result.values() for r in p['regions'].values()):raise SystemExit('post-chain equation verification failed')
