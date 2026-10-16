# SPDX-License-Identifier: GPL-3.0-or-later
"""Paint the chest-sign star out of the Duty torso icon. usage: icon_star.py IN OUT cy cx r"""
import sys, numpy as np
from PIL import Image
a=np.asarray(Image.open(sys.argv[1]).convert('RGBA')).astype(float); cy,cx,r=map(float,sys.argv[3:6])
yy,xx=np.mgrid[:a.shape[0],:a.shape[1]]; d=np.hypot(yy-cy,xx-cx)
w=np.clip(r+0.5-d,0,1)[...,None]
a[...,:3]=a[...,:3]*(1-w)+np.array([36,34,31])*w
Image.fromarray(np.clip(a+0.5,0,255).astype(np.uint8)).save(sys.argv[2])
