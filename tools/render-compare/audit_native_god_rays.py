#!/usr/bin/env python3
"""Replay native additive god-ray application from frozen diagnostic buffers."""
import sys,json,math,struct
from pathlib import Path
from inspect_buffers import pixels
root=Path(sys.argv[1]); rows=[]
assert root.is_dir(), root
for f in sorted(root.glob('*.god-rays-apply-before.targets.json')):
 base=str(f).removesuffix('.god-rays-apply-before.targets.json')
 def load(label):
  p=Path(base+'.'+label);m=json.loads(Path(str(p)+'.targets.json').read_text())[0]
  return m,list(pixels(Path(str(p)+'.rt00.bin').read_bytes(),m['format']))
 bm,b=load('god-rays-apply-before');am,a=load('god-rays-apply-output');sm,s=load('god-rays-apply-draw')
 w,h=bm['width'],bm['height'];sw,sh=sm['width'],sm['height'];n=bad=changed=0;total=maximum=0.;region=[0.,0.,0.];rn=0
 for y in range(0,h,2):
  sy=(y+.5)*sh/h-.5;iy=math.floor(sy);fy=sy-iy
  for x in range(0,w,2):
   sx=(x+.5)*sw/w-.5;ix=math.floor(sx);fx=sx-ix;k=y*w+x
   for c in range(3):
    v=sum(s[((iy+dy)%sh)*sw+(ix+dx)%sw][c]*(fy if dy else 1-fy)*(fx if dx else 1-fx) for dy in range(2) for dx in range(2))
    pred=b[k][c]+v;err=abs(pred-a[k][c]);ulp=math.ldexp(1,max(-14,math.floor(math.log2(pred)) if pred else -14)-(6 if c<2 else 5))
    n+=1;bad+=err>ulp+1e-8;changed+=a[k][c]!=b[k][c];total+=err;maximum=max(maximum,err)
    if 900<=x<1060 and 60<=y<260:region[c]+=a[k][c]-b[k][c]
   if 900<=x<1060 and 60<=y<260:rn+=1
 rows.append(dict(pose=Path(base).name,n=n,outside_one_ulp=bad,changed=changed,mae=total/n,max_error=maximum,door_region_added_rgb=[v/rn for v in region]))
assert rows, 'no native god-ray captures found'
print(json.dumps(rows,indent=2))
assert all(row['outside_one_ulp']==0 for row in rows), 'additive replay exceeded packed-float precision'
