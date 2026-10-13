#!/usr/bin/env python3
"""Audit source packed specular amounts against native material parameters."""
import argparse
from collections import Counter,defaultdict
import json
from pathlib import Path
import struct
from audit_wall_composition import REGIONS


def audit(original,captures,pose,rect):
    def read(folder,suffix): return (folder/(pose+'.bmp'+suffix)).read_bytes()
    od,nd=read(original,'.composition-draw.rt00.bin'),read(captures,'.composition-draw.rt03.bin')
    normal,albedo=read(original,'.composition-draw.rt01.bin'),read(original,'.composition-draw.rt02.bin')
    params=read(captures,'.composition-draw.rt02.bin')
    bins=Counter();vectors=defaultdict(Counter);count=0
    for y in range(rect[1],rect[3]):
        for x in range(rect[0],rect[2]):
            i=y*1280+x
            if od[2*i:2*i+2]!=nd[2*i:2*i+2] or not 0<struct.unpack_from('<e',od,2*i)[0]<499.75: continue
            count+=1;bits=struct.unpack_from('<I',normal,4*i)[0]>>27
            flag=albedo[4*i+3]>>7
            vector=tuple(params[4*i:4*i+4]);bins[(flag,bits,vector[2],vector[3])]+=1
            if not flag:vectors[vector][bits]+=1
    ambiguous=[dict(native_parameters=list(k),source_amount_bits=dict(sorted(v.items())),pixels=sum(v.values())) for k,v in vectors.items() if len(v)>1]
    return dict(pose=pose,rectangle=rect,exact_depth_samples=count,
        material_bins=[dict(source_translucent_flag=k[0],source_amount_bits=k[1],source_linear_amount=(k[1]/31)**2.2 if not k[0] else 1.,native_translucency=k[2],native_specular_amount=k[3],pixels=n) for k,n in sorted(bins.items())],
        ambiguous_native_parameter_vectors=sorted(ambiguous,key=lambda v:-v['pixels']))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('original',type=Path);p.add_argument('captures',type=Path);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();rows=[audit(a.original,a.captures,pose,rect) for pose,rect in REGIONS.items()]
    result=dict(scope='Fixed static wall regions, exact depth. Source amount is packed bits27..31 with gamma2.2 decode; translucent source flag makes composition amount1. Native bytes reported directly. No material draw identity or texture feature attribution is inferred from these pixels.',views=rows)
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps([dict(pose=r['pose'],pixels=r['exact_depth_samples'],ambiguous_vectors=len(r['ambiguous_native_parameter_vectors'])) for r in rows]))
