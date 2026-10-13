#!/usr/bin/env python3
"""Compare encoded lighting on common static pixels across two target runs."""
import argparse,json,pathlib,struct
from audit_translucency_auxiliary import rgb,stats,add,finish
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('original',type=pathlib.Path)
parser.add_argument('prior',type=pathlib.Path)
parser.add_argument('current',type=pathlib.Path)
parser.add_argument('--output',type=pathlib.Path,required=True)
args=parser.parse_args()
old,prior,new=args.original,args.prior,args.current
rows=[]
for f in sorted(new.glob('*.bmp.ambient-encoded.json')):
 pose=f.name.removesuffix('.ambient-encoded.json')
 def read(p,s):return (p/(pose+s)).read_bytes()
 od=read(old,'.composition-draw.rt00.bin');pd=read(prior,'.composition-draw.rt03.bin');nd=read(new,'.composition-draw.rt03.bin')
 ps=read(prior,'.sun-encoded-0.rt01.bin')[3::4];ns=read(new,'.sun-encoded-0.rt01.bin')[3::4]
 fields=[(read(prior,f'.composition-draw.rt{k:02d}.bin'),read(new,f'.composition-draw.rt{k:02d}.bin')) for k in (0,1,2)]
 stages={k:(read(old,o),read(prior,t),read(new,t)) for k,o,t in [('ambient','.sun-before.rt00.bin','.ambient-encoded.rt00.bin'),('final','.composition-draw.rt03.bin','.sun-encoded-2.rt00.bin')]}
 r={m:{k:{v:stats() for v in ['prior','last_mip']} for k in stages} for m in ['common_exact_depth','common_unchanged_gbuffer']}
 for y in range(1,719,4):
  for x in range(1,1279,4):
   i=y*1280+x
   if y>=544 or (x<224 and y<224) or (x>=640 and y>=384) or (608<=x<672 and 320<=y<384):continue
   if not ps[i] or not ns[i] or not 0<struct.unpack_from('<e',od,2*i)[0]<499.75 or not od[2*i:2*i+2]==pd[2*i:2*i+2]==nd[2*i:2*i+2]:continue
   masks=['common_exact_depth']
   if all(a[4*i:4*i+4]==b[4*i:4*i+4] for a,b in fields):masks.append('common_unchanged_gbuffer')
   for mask in masks:
    for k,(o,a,b) in stages.items():
     add(r[mask][k]['prior'],rgb(o,i),rgb(a,i));add(r[mask][k]['last_mip'],rgb(o,i),rgb(b,i))
 rows.append(dict(pose=pose,masks={m:{k:{v:finish(s) for v,s in vals.items()} for k,vals in stages.items()} for m,stages in r.items()}))
report=dict(scope='Intersection of source/prior/new exact geometry depth, nonzero native stencils, fixed UI/weapon exclusion. Stable subset additionally requires identical target normal/albedo/parameters bytes; it does not establish identical AO, draw constants or coverage.',views=rows)
args.output.write_text(json.dumps(report,indent=2)+'\n')
for r in rows:
 print(r['pose'],{m:{k:(v['prior']['samples'],v['prior']['encoded_rgb_mae'],v['last_mip']['encoded_rgb_mae']) for k,v in s.items()} for m,s in r['masks'].items()})
