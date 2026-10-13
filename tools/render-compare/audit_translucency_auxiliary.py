#!/usr/bin/env python3
"""Replay the private material-gated translucency addition and compare source stages."""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
import struct

from audit_surface_light import REGIONS


def saturate(value):
    return max(0.,min(1.,value))


def normalize(value):
    length=math.sqrt(sum(v*v for v in value))
    return [v/length for v in value] if length else [0.,0.,0.]


def rgb(data,index):
    value=struct.unpack_from('<I',data,index*4)[0]
    return [(value>>shift&1023)/1023 for shift in (0,10,20)]


def stats():
    return {'samples':0,'sum_original':[0.,0.,0.],'sum_native':[0.,0.,0.],
            'sum_absolute_rgb_error':0.,'sum_linear_absolute_rgb_error':0.}


def add(st,old,new):
    st['samples']+=1
    for c in range(3):
        st['sum_original'][c]+=old[c];st['sum_native'][c]+=new[c]
        st['sum_absolute_rgb_error']+=abs(old[c]-new[c])
        st['sum_linear_absolute_rgb_error']+=8*abs(old[c]**2-new[c]**2)


def finish(st):
    count=st['samples']
    return {'samples':count,'original_encoded_rgb_mean':[v/count for v in st['sum_original']] if count else None,
            'native_encoded_rgb_mean':[v/count for v in st['sum_native']] if count else None,
            'encoded_rgb_mae':st['sum_absolute_rgb_error']/(3*count) if count else None,
            'decoded_linear_rgb_mae':st['sum_linear_absolute_rgb_error']/(3*count) if count else None}


def compare_stages(original,stem,stride):
    old=original/Path(stem).name
    def read(base,suffix): return Path(str(base)+suffix).read_bytes()
    od=read(old,'.composition-draw.rt00.bin');nd=read(stem,'.composition-draw.rt03.bin')
    if read(old,'.translucency-output.rt00.bin')!=read(old,'.composition-draw.rt03.bin'):
        raise ValueError('original final accumulator continuity changed')
    stages={'ambient':(read(old,'.sun-before.rt00.bin'),read(stem,'.ambient-encoded.rt00.bin')),
            'ambient_sun_local':(read(old,'.translucency-before.rt00.bin'),read(stem,'.sun-encoded-1.rt00.bin')),
            'final':(read(old,'.composition-draw.rt03.bin'),read(stem,'.sun-encoded-2.rt00.bin'))}
    results={name:{mask:stats() for mask in ('fixed_mask_geometry','exact_depth','exact_depth_3x3')} for name in stages}
    regions={name:{stage:stats() for stage in stages} for name in REGIONS.get(Path(stem).stem,{})}
    matched=bytearray(1280*720)
    for i in range(len(matched)):
        if od[i*2:i*2+2]==nd[i*2:i*2+2] and 0<struct.unpack_from('<e',od,i*2)[0]<1000:
            matched[i]=1
    for y in range(1,719,stride):
        for x in range(1,1279,stride):
            i=y*1280+x
            if not (0<struct.unpack_from('<e',od,i*2)[0]<1000 and 0<struct.unpack_from('<e',nd,i*2)[0]<1000): continue
            # Same conservative HUD/weapon exclusion as broad weather metrics.
            if y>=544 or (x<224 and y<224) or (x>=640 and y>=384) or (608<=x<672 and 320<=y<384): continue
            masks=['fixed_mask_geometry']
            if matched[i]: masks.append('exact_depth')
            stable=all(matched[i+dy*1280+dx] for dy in (-1,0,1) for dx in (-1,0,1))
            if stable: masks.append('exact_depth_3x3')
            values={stage:(rgb(data[0],i),rgb(data[1],i)) for stage,data in stages.items()}
            for stage,(a,b) in values.items():
                for mask in masks: add(results[stage][mask],a,b)
            if matched[i]:
                for name,(x0,y0,x1,y1) in REGIONS.get(Path(stem).stem,{}).items():
                    if x0<=x<x1 and y0<=y<y1:
                        for stage,(a,b) in values.items(): add(regions[name][stage],a,b)
    return {'stages':{stage:{mask:finish(st) for mask,st in masks.items()} for stage,masks in results.items()},
            'predefined_regions_exact_depth':{name:{stage:finish(st) for stage,st in values.items()} for name,values in regions.items()}}


def audit(report,stride,original,comparison_stride):
    state=json.loads(report.read_text());stem=str(report).removesuffix('.sun-encoded.json')
    def read(suffix): return Path(stem+suffix).read_bytes()
    if not state['ready'] or state['draws']!=2 or len({state[k] for k in ('present','native_depth_present','projection_present')})!=1:
        raise ValueError(f'{report}: incomplete translucency diagnostic')
    helper=json.loads(read('.translucency-helper.json'))
    if helper['present']!=state['present'] or helper['helper_present']!=state['present']:
        raise ValueError('projection input is not from current Present')
    views={v['slot']:v for v in json.loads(read('.composition-draw.targets.json'))}
    for slot,fmt in ((0,34),(2,28),(3,54),(6,49),(13,41)):
        v=views[slot]
        if (v['format'],v['width'],v['height'],v['written'])!=(fmt,1280,720,True):
            raise ValueError(f'input contract changed: slot{slot}')
    ds_info=json.loads(read('.sun-encoded-0.targets.json'))[1]
    ds=read('.sun-encoded-0.rt01.bin')
    if not ds_info['written']: raise ValueError('no final native stencil readback')
    if ds_info['format'] in (44,45): stencil=ds[3::4]
    elif ds_info['format'] in (19,20): stencil=ds[4::8]
    else: raise ValueError('unsupported native stencil format')
    normal,parameters,depth,shadow,projection=[read(f'.composition-draw.rt{slot:02d}.bin') for slot in (0,2,3,6,13)]
    before,after=read('.sun-encoded-1.rt00.bin'),read('.sun-encoded-2.rt00.bin')
    if len(stencil)!=1280*720 or len(before)!=1280*720*4 or len(after)!=len(before): raise ValueError('incomplete output bytes')
    cb0=read('.composition-draw.cb0.bin');light=struct.unpack_from('<3f',cb0,0);ray=struct.unpack_from('<3f',cb0,400)
    color=state['source_constants'][4:7]
    groups={name:{'samples':0,'maximum_error':0.,'sum_error':0.,'outside_one_10bit_step':0,'examples':[]} for name in ('changed','unchanged')}
    full={'changed_pixels':0,'changed_alpha_pixels':0,'changed_without_stencil_pixels':0,'changed_without_parameter_pixels':0}
    for i in range(1280*720):
        a,b=struct.unpack_from('<I',before,i*4)[0],struct.unpack_from('<I',after,i*4)[0]
        if a!=b:
            full['changed_pixels']+=1;full['changed_alpha_pixels']+=(a>>30)!=(b>>30)
            full['changed_without_stencil_pixels']+=not stencil[i]
            full['changed_without_parameter_pixels']+=not parameters[i*4+2]
        if i%stride: continue
        encoded=[0.,0.,0.]
        amount=(math.floor((parameters[i*4+2]/255)**(1/2.2)*31)/31)**2.2
        if stencil[i] and amount>0:
            x,y=i%1280,i//1280;z=struct.unpack_from('<e',depth,i*2)[0]
            position=[(1-2*(x+.5)/1280)*ray[0]/ray[2]*z,(1-2*(y+.5)/720)*ray[1]/ray[2]*z,z]
            f=[v*4-2 for v in struct.unpack_from('<2e',normal,i*4)];radius=sum(v*v for v in f)
            n=normalize([f[0]*math.sqrt(abs(1-radius*.25)),f[1]*math.sqrt(abs(1-radius*.25)),radius*.5-1])
            projected=struct.unpack_from('<f',projection,i*4)[0];visibility=shadow[i*2]/255
            attenuation=(1-saturate((projected-visibility)/(amount*.035)))**10
            back=sum(v*w for v,w in zip(n,light));forward=saturate(-sum(v*w for v,w in zip(normalize(position),light)))
            result=[amount*.1+c*back*attenuation*amount*1.5+amount*back*.1*c+c*forward**3*amount*attenuation*2 for c in color]
            encoded=[math.sqrt(saturate(v*.125)) for v in result]
        errors=[abs(saturate((a>>shift&1023)/1023+value)-(b>>shift&1023)/1023) for shift,value in zip((0,10,20),encoded)]
        st=groups['changed' if a!=b else 'unchanged'];st['samples']+=1;st['sum_error']+=sum(errors)/3
        st['maximum_error']=max(st['maximum_error'],max(errors))
        if max(errors)>1/1023+1e-6:
            st['outside_one_10bit_step']+=1
            if len(st['examples'])<8: st['examples'].append({'xy':[i%1280,i//1280],'stencil':stencil[i],'amount':amount,'errors':errors})
    for st in groups.values(): st['mean_absolute_rgb_error']=st.pop('sum_error')/max(1,st['samples'])
    result={'pose':Path(stem).name,'present':state['present'],'stencil_histogram':dict(Counter(stencil)),
            'full_frame_gate_checks':full,'equation_groups':groups}
    if original: result['source_comparison']=compare_stages(original,stem,comparison_stride)
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('captures',type=Path)
    p.add_argument('--original',type=Path);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--stride',type=int,default=13);p.add_argument('--comparison-stride',type=int,default=4)
    a=p.parse_args()
    if min(a.stride,a.comparison_stride)<1: p.error('strides must be positive')
    files=sorted(a.captures.glob('*.bmp.sun-encoded.json'))
    if not files: p.error('no completed source sun/translucency captures')
    rows=[audit(path,a.stride,a.original,a.comparison_stride) for path in files]
    result={'scope':'Private source-equation diagnostic on native inputs and parameter-based material gate. Original geometry/stencil/material parity is not implied. Encoded-stage comparison uses a fixed HUD/weapon exclusion even though these buffers precede UI, and reports stricter exact-depth and 3x3 exact-depth masks separately.',
            'captures':str(a.captures.resolve()),'original':str(a.original.resolve()) if a.original else None,
            'equation_stride':a.stride,'comparison_grid_stride':a.comparison_stride,'views':rows}
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'report':str(a.output),'views':len(rows),
                      'samples':sum(st['samples'] for row in rows for st in row['equation_groups'].values()),
                      'maximum_error':max(st['maximum_error'] for row in rows for st in row['equation_groups'].values())}))
    if any(any(st['outside_one_10bit_step'] for st in row['equation_groups'].values()) or any(row['full_frame_gate_checks'][key] for key in ('changed_alpha_pixels','changed_without_stencil_pixels','changed_without_parameter_pixels')) for row in rows):
        raise SystemExit(1)


if __name__=='__main__':
    main()
