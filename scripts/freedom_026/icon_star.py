# SPDX-License-Identifier: GPL-3.0-or-later
"""Edit the chest disc out of the Duty torso icon (171x83 RGBA).
usage: icon_star.py IN OUT cy cx r [star|disc]
  star: paint the star dark (the disc's empty centre), r = star radius
  disc: remove the whole disc, filling from the surrounding plate, r = disc radius"""
import sys, numpy as np
from pathlib import Path
from PIL import Image
sys.path.insert(0,str(Path(__file__).parent)); import inpaint
a=np.asarray(Image.open(sys.argv[1]).convert('RGBA')).astype(float); cy,cx,r=map(float,sys.argv[3:6])
mode=sys.argv[6] if len(sys.argv)>6 else 'star'
yy,xx=np.mgrid[:a.shape[0],:a.shape[1]]; d=np.hypot(yy-cy,xx-cx)
w=np.clip(r+0.5-d,0,1)[...,None]
if mode=='star':
    fillc=np.array([36,34,31])
else:
    hole=d<=r+0.5; known=(~hole)&(a[...,3]>200)
    fillc=inpaint.normconv_fill(a[...,:3],known,sigmas=(1.5,3,6))
a[...,:3]=a[...,:3]*(1-w)+fillc*w
Image.fromarray(np.clip(a+0.5,0,255).astype(np.uint8)).save(sys.argv[2])
