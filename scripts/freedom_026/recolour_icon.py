# SPDX-License-Identifier: GPL-3.0-or-later
"""Apply the Freedom recolour to an existing Duty lobby icon (171x83 RGBA).
usage: recolour_icon.py IN.png OUT.png GAIN [x0,y0,x1,y1 ...exclusions]"""
import sys, numpy as np
from pathlib import Path
from PIL import Image
sys.path.insert(0,str(Path(__file__).parent)); import colorlib as cl
im=np.asarray(Image.open(sys.argv[1]).convert('RGBA')).astype(np.float64); gain=float(sys.argv[3])
rgb=im[...,:3]; L,C,h=cl.srgb8_to_oklch(rgb); hd=np.where(h>180,h-360,h)
def sm(x,a,b): t=np.clip((x-a)/(b-a),0,1); return t*t*(3-2*t)
w=sm(hd,-28,-16)*(1-sm(hd,44,52))*sm(C,0.014,0.030)*sm(L,0.04,0.10)*(im[...,3]>0)
for r in sys.argv[4:]:
    x0,y0,x1,y1=map(int,r.split(',')); w[y0:y1,x0:x1]=0
sel=w>0.5; h_med=np.median(hd[sel]); cl_med=np.median((C/np.maximum(L,1e-4))[sel])
Ln=np.clip(L+(L*gain-L)*np.clip((0.75-L)/0.35,0,1),0,0.97); hn=54.9+0.35*(hd-h_med)
Cn=cl.fit_gamut(Ln,np.minimum((C/np.maximum(L,1e-4))*(0.206/cl_med),1.5*0.206)*Ln,hn)
new=cl.oklch_to_srgb8(Ln,Cn,hn)
lin=cl.srgb_to_lin(rgb/255)*(1-w[...,None])+cl.srgb_to_lin(new/255)*w[...,None]
out=im.copy(); out[...,:3]=cl.lin_to_srgb(lin)*255
Image.fromarray(np.clip(out+0.5,0,255).astype(np.uint8)).save(sys.argv[2])
