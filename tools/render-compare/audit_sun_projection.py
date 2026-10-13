#!/usr/bin/env python3
"""Replay captured cascade projection on changed samples; check the final SRV binding.

Unchanged helper values cannot identify which cascade drew them. This verifies
projection arithmetic, not legacy cascade selection or shadow visibility.
"""
from pathlib import Path
import argparse,json,struct,math
p=argparse.ArgumentParser();p.add_argument('capture',type=Path);p.add_argument('--stride',type=int,default=13);a=p.parse_args();assert a.stride>0
report={}
for meta in sorted(a.capture.glob('*.bmp.translucency-helper.json')):
 base=str(meta).removesuffix('.translucency-helper.json');m=json.loads(meta.read_text());assert m['present']==m['helper_present'];assert m['cascade_draws']>0
 inputs={x['slot']:x for x in json.loads(Path(base+'.composition-draw.targets.json').read_text())};w,h=inputs[3]['width'],inputs[3]['height'];assert inputs[3]['format']==54;assert inputs[13]['format']==41
 depth=Path(base+'.composition-draw.rt03.bin').read_bytes();previous=bytes(w*h*4);cascades=[]
 for n in range(1,m['cascade_draws']+1):
  prefix=base+f'.sun-projection-{n}';raw=Path(prefix+'.rt00.bin').read_bytes();assert len(raw)==len(previous)
  cb=Path(prefix+'.cb0.bin').read_bytes();matrix=[struct.unpack_from('<4f',cb,256+i*16) for i in range(4)];ray=struct.unpack_from('<3f',cb,352)
  errors=[];nonfinite=0
  for i in range(0,w*h,a.stride):
   if raw[i*4:i*4+4]==previous[i*4:i*4+4]:continue
   actual=struct.unpack_from('<f',raw,i*4)[0];z=struct.unpack_from('<e',depth,i*2)[0];x,y=i%w,i//w
   pos=[(1-2*(x+.5)/w)*ray[0]/ray[2]*z,(1-2*(y+.5)/h)*ray[1]/ray[2]*z,z,1]
   projected=[sum(v*t for v,t in zip(row,pos)) for row in matrix];expected=projected[2]/projected[3]
   if not math.isfinite(actual) or not math.isfinite(expected):nonfinite+=1
   else:errors.append(abs(actual-expected))
  cascades.append({'cascade':n,'changed_samples':len(errors),'nonfinite':nonfinite,'max_absolute_error':max(errors,default=0),'mean_absolute_error':sum(errors)/max(len(errors),1),'over_4e_6':sum(v>4e-6 for v in errors)})
  previous=raw
 assert previous==Path(base+'.composition-draw.rt13.bin').read_bytes()
 report[Path(base).name]={'helper_metadata':m,'final_binding_byte_identical':True,'cascades':cascades}
assert report, 'no helper captures'
print(json.dumps({'stride':a.stride,'views':report},indent=2))
if any(c['nonfinite'] or c['over_4e_6'] for r in report.values() for c in r['cascades']):
 raise SystemExit('sun projection replay failed')
