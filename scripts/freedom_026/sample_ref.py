# SPDX-License-Identifier: GPL-3.0-or-later
import sys, json, colorsys, numpy as np
from PIL import Image, ImageDraw
ref=sys.argv[1]; out=sys.argv[2]
im=Image.open(ref).convert('RGB'); a=np.asarray(im).astype(float)
regions={ 'helmet_top':(400,60,470,100),'helmet_side':(470,95,520,120),
 'chest_stripe_1':(320,365,540,380),'chest_stripe_2':(320,418,540,432),'chest_stripe_3':(330,455,540,470),
 'mask_canister_l':(392,225,415,265),'mask_canister_r':(505,225,525,265),
 'arm_band_r':(600,530,690,550),'arm_band_l':(180,485,240,505),'knee_plate':(440,880,490,905)}
def lin(c): c=c/255; return np.where(c<=0.04045,c/12.92,((c+0.055)/1.055)**2.4)
res={}; allpx=[]
for k,(x0,y0,x1,y1) in regions.items():
    px=a[y0:y1,x0:x1].reshape(-1,3)
    h=np.array([colorsys.rgb_to_hsv(*(p/255)) for p in px])
    keep=(h[:,1]>0.45)&(h[:,2]>0.35)&(h[:,0]>0.04)&(h[:,0]<0.17)  # painted pixels only
    px=px[keep]; allpx.append(px)
    med=np.median(px,0); hh=colorsys.rgb_to_hsv(*(med/255))
    res[k]=dict(n=int(len(px)),srgb=[round(v) for v in med],hsv=[round(hh[0]*360,1),round(hh[1],3),round(hh[2],3)])
meds=np.array([r['srgb'] for r in res.values()],float)
m=np.median(meds,0); hh=colorsys.rgb_to_hsv(*(m/255)); l=lin(m)
res['MEDIAN_OF_REGIONS']=dict(srgb=[int(round(v)) for v in m],hex='#%02X%02X%02X'%tuple(int(round(v)) for v in m),
  linear=[round(float(v),4) for v in l],hsv=[round(hh[0]*360,1),round(hh[1],3),round(hh[2],3)])
print(json.dumps(res,indent=1))
d=ImageDraw.Draw(im)
for k,(x0,y0,x1,y1) in regions.items(): d.rectangle((x0,y0,x1,y1),outline=(0,255,255))
im.save(out)
# OKLCh statistics of all painted pixels (pooled) and per-region medians
import sys as _s; _s.path.insert(0,str(__import__('pathlib').Path(__file__).parent)); import colorlib as cl
pool=np.concatenate(allpx); L,C,h=cl.srgb8_to_oklch(pool)
print('OKLCh pooled painted pixels: n=%d L med %.3f p25 %.3f p75 %.3f | C med %.3f | h med %.1f p10 %.1f p90 %.1f | C/L med %.3f'%(len(pool),np.median(L),np.percentile(L,25),np.percentile(L,75),np.median(C),np.median(h),np.percentile(h,10),np.percentile(h,90),np.median(C/L)))
Lm,Cm,hm=cl.srgb8_to_oklch(meds); print('per-region-median OKLCh: L %.3f C %.3f h %.1f C/L %.3f'%(np.median(Lm),np.median(Cm),np.median(hm),np.median(Cm/Lm)))
