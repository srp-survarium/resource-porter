#!/usr/bin/env python3
from pathlib import Path
import argparse,json,numpy as np
from PIL import Image,ImageDraw
parser=argparse.ArgumentParser(description="Compare matched source/target remains surfaces without fitting a gain.")
parser.add_argument('--source',type=Path,required=True)
parser.add_argument('--packaged',type=Path,required=True)
parser.add_argument('--supported-sun',type=Path,required=True)
parser.add_argument('--supported-sun-local',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args();root=args.output;root.mkdir(parents=True,exist_ok=True)
paths={'source':args.source,'packaged_bc3':args.packaged,'supported_sun':args.supported_sun,'supported_sun_local':args.supported_sun_local}
regions={
 'skeleton-central-path':{'remains_overall':[530,183,785,454],'ribs':[647,322,676,347],'skull':[726,402,747,424],'pelvis':[599,267,621,284],'cloth_sleeve':[690,299,708,312],'ground_control':[850,330,1000,470],'ground_near':[810,482,866,527],'barrel_control':[80,516,164,591]},
 'skeleton-lower-path':{'remains_overall':[505,195,800,425],'ribs':[691,334,718,352],'skull':[739,389,758,404],'cloth_sleeve':[677,300,701,311],'floor_control':[825,450,985,580],'floor_near':[762,424,813,465],'wall_control':[203,81,290,136],'crate_control':[94,457,137,513]}}
report={'scope':'All original and target cameras match on011e. Preselected broad controls and manually selected source surface patches; small patches require visual review, not exact material masks. Screenshots have projection/mip/exposure differences. No gain is fitted or applied.','views':{}}
for name,rois in regions.items():
 ims={k:Image.open(next(q for q in [p/(name+'.bmp'),p/(name+'.png')] if q.exists())).convert('RGB') for k,p in paths.items()};a={k:np.asarray(v,dtype=float)/255 for k,v in ims.items()}
 for p in paths.values():
  j=json.loads((p/(name+'.json')).read_text());assert j['status']['error']==''
  for field in ['position','forward','fov']:
   assert np.allclose(j['pose'][field],json.loads((paths['source']/(name+'.json')).read_text())['pose'][field],atol=1e-7)
 rows={}
 for label,(x0,y0,x1,y1) in rois.items():
  samples={k:v[y0:y1,x0:x1] for k,v in a.items()};out={'rect':[x0,y0,x1,y1],'pixels':(x1-x0)*(y1-y0),'arms':{}}
  for k,v in samples.items():
   l=v@np.array([.2126,.7152,.0722]);out['arms'][k]={'mean_rgb':v.mean((0,1)).tolist(),'mean_luma':float(l.mean()),'p95_luma':float(np.quantile(l,.95)),'source_rgb_mae':float(abs(v-samples['source']).mean())}
  rows[label]=out
 control='floor_control' if name=='skeleton-lower-path' else 'ground_control'
 for label in ['ribs','skull','cloth_sleeve']:
  for arm in paths:
   rows[label]['arms'][arm]['luma_ratio_to_control']=rows[label]['arms'][arm]['mean_luma']/rows[control]['arms'][arm]['mean_luma']
 report['views'][name]=rows
 canvas=Image.new('RGB',(2560,1480));draw=ImageDraw.Draw(canvas)
 for index,(label,im) in enumerate(ims.items()):
  x=(index%2)*1280;y=(index//2)*740;canvas.paste(im,(x,y+20));draw.text((x+8,y+3),label,fill='white')
 canvas.save(root/(name+'-four-arms.png'))
 annotated=ims['source'].copy();draw=ImageDraw.Draw(annotated)
 for label,(x0,y0,x1,y1) in rois.items():
  draw.rectangle((x0,y0,x1,y1),outline='red',width=1);draw.text((x0,y0-11),label,fill='red')
 annotated.save(root/(name+'-regions.png'))
(root/'comparison.json').write_text(json.dumps(report,indent=2)+'\n')
for name,rows in report['views'].items():
 print(name)
 for label,row in rows.items():print(label,[(k,round(v['mean_luma'],5)) for k,v in row['arms'].items()])
