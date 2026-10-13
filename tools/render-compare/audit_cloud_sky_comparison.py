#!/usr/bin/env python3
"""Compare final sky pixels and cloud-stage HDR on a common frozen geometry mask."""
import argparse
import json
import math
from pathlib import Path
import struct

from audit_full_diffuse_composition import unpack
from compare_weather import bmp


def metrics(a,b,points):
    if not points:return {'samples':0,'rgb_mae':None,'rgb_rmse':None}
    absolute=squared=0.;ma=[0.]*3;mb=[0.]*3
    for x,y in points:
        for c,(v,w) in enumerate(zip(a(x,y),b(x,y))):
            absolute+=abs(v-w);squared+=(v-w)**2;ma[c]+=v;mb[c]+=w
    n=len(points)
    return dict(samples=n,rgb_mae=absolute/(3*n),rgb_rmse=math.sqrt(squared/(3*n)),
                source_rgb_mean=[v/n for v in ma],target_rgb_mean=[v/n for v in mb])


def audit(original,baseline,current,pose,stride):
    def path(folder,suffix):return folder/(pose+suffix)
    od=path(original,'.composition-draw.rt00.bin').read_bytes()
    bd=path(baseline,'.composition-draw.rt03.bin').read_bytes()
    current_depth=path(current,'.composition-draw.rt03.bin')
    cd=current_depth.read_bytes() if current_depth.exists() else None
    if any(len(d)!=1280*720*2 for d in [od,bd]+([cd] if cd else [])):raise ValueError('depth dimensions changed')
    sky=[];geometry=[];full=[]
    for y in range(stride//2,544,stride):
        for x in range(stride//2,1280,stride):
            if (x<224 and y<224) or (x>=640 and y>=384) or (608<=x<672 and 320<=y<384):continue
            full.append((x,y));i=y*1280+x
            values=[struct.unpack_from('<e',d,2*i)[0] for d in [od,bd]+([cd] if cd else [])]
            if all(v==499.75 for v in values):sky.append((x,y))
            elif all(0<v<499.75 for v in values):geometry.append((x,y))
    source=bmp(original/pose);rows={}
    for label,folder in [('baseline',baseline),('current',current)]:
        target=bmp(folder/pose)
        row={'final_sky':metrics(source,target,sky),'final_geometry':metrics(source,target,geometry),'final_fixed_mask':metrics(source,target,full)}
        for stage in ['cloud-before','cloud-output']:
            a,b=path(original,'.'+stage+'.rt00.bin'),path(folder,'.'+stage+'.rt00.bin')
            if a.exists() and b.exists():
                for directory in [original,folder]:
                    v=json.loads(path(directory,'.'+stage+'.targets.json').read_text())[0]
                    if (v['width'],v['height'],v['format'],v['written'])!=(1280,720,26,True):raise ValueError('cloud-stage contract changed')
                oa,ta=a.read_bytes(),b.read_bytes()
                pixel=lambda raw:lambda x,y:unpack(struct.unpack_from('<I',raw,4*(y*1280+x))[0])
                row[stage+'_sky_hdr']=metrics(pixel(oa),pixel(ta),sky)
        rows[label]=row
    return dict(pose=pose,current_geometry_readback=cd is not None,recipes=rows)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('original',type=Path);p.add_argument('baseline',type=Path);p.add_argument('current',type=Path)
    p.add_argument('--stride',type=int,default=16);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    if a.stride<1:p.error('stride must be positive')
    poses=sorted(f.name for f in a.current.glob('*.bmp') if (a.original/f.name).exists() and (a.baseline/f.name).exists())
    if not poses:p.error('no common captured poses')
    rows=[audit(a.original,a.baseline,a.current,pose,a.stride) for pose in poses]
    result=dict(scope='Common far-clear sky, or common non-clear geometry, excluding fixed HUD/weapon areas. Final screenshots include all later passes and animation phase. Missing current depth explicitly reuses original/baseline mask; no current geometry equivalence then implied.',stride=a.stride,original=str(a.original),baseline=str(a.baseline),current=str(a.current),views=rows)
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps([dict(pose=r['pose'],sky={k:v['final_sky']['rgb_mae'] for k,v in r['recipes'].items()},samples=r['recipes']['current']['final_sky']['samples']) for r in rows]))
