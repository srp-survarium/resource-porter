# SPDX-License-Identifier: GPL-3.0-or-later
"""Build the Freedom texture set for the ported STK2 Duty exoskeleton.

usage: make_freedom.py SRC_TEXDIR SIGN_LAYERS_DIR FREEDOM_PATCH_PNG OUTDIR [--gain G] [--target-h H] [--target-cl R] [--hue-keep K]

SRC_TEXDIR       resource-porter .work/duty-026-inputs/.../character/human/dolg (STK2 TGAs)
SIGN_LAYERS_DIR  layers of STK2 dol_sign.psd (psdlayers.py); layers 03/04 hold the star shape
FREEDOM_PATCH    RGBA PNG of the Freedom patch
OUTDIR/textures  a drop-in --texture-dir: every source TGA, changed ones rewritten in place
                 (same TGA header, footer, size, channel layout and alpha)
"""
import sys, json, shutil, argparse, numpy as np
from pathlib import Path
from PIL import Image
from scipy import ndimage as ndi
from scipy.spatial import ConvexHull, Delaunay
sys.path.insert(0,str(Path(__file__).parent)); import tga, inpaint, colorlib as cl

ap=argparse.ArgumentParser(); ap.add_argument('src',type=Path); ap.add_argument('sign_layers',type=Path)
ap.add_argument('patch',type=Path); ap.add_argument('out',type=Path)
ap.add_argument('--gain',type=float,default=1.0,help='luminance gain for recoloured paint (1 = keep source luminance)')
ap.add_argument('--target-h',type=float,default=54.9,help='OKLCh hue of the reference yellow')
ap.add_argument('--target-cl',type=float,default=0.206,help='OKLCh C/L of the reference yellow')
ap.add_argument('--hue-keep',type=float,default=0.35,help='fraction of source hue variation kept')
a=ap.parse_args()
OUT=a.out; TEX=OUT/'textures'; MASKS=OUT/'masks'; TEX.mkdir(parents=True,exist_ok=True); MASKS.mkdir(exist_ok=True)
import os
for f in sorted(a.src.glob('*.tga')):   # unchanged textures: symlinks to the STK2 sources
    t=TEX/f.name
    if t.is_symlink() or t.exists(): t.unlink()
    os.symlink(f.resolve(),t)
_tga_write=tga.write
def _safe_write(path,px,meta):
    path=Path(path)
    if path.is_symlink(): path.unlink()   # never write through to the source file
    _tga_write(path,px,meta)
tga.write=_safe_write
stats={'params':vars(a)|{'src':str(a.src),'sign_layers':str(a.sign_layers),'patch':str(a.patch),'out':str(a.out)}}

def smooth(x,e0,e1):
    t=np.clip((x-e0)/(e1-e0),0,1); return t*t*(3-2*t)
def hue_weight(h):
    hd=np.where(h>180,h-360,h)   # red family in OKLCh: about -20..45 deg
    return smooth(hd,-28,-16)*(1-smooth(hd,44,52))
def rect(mask,x0,y0,x1,y1): mask[y0:y1,x0:x1]=True

# --- spatial exclusions (2048 texel coordinates; x0,y0,x1,y1) --------------------
EXCLUDE={
 'dolg_exo_torso':[(1100,440,1740,640),(1100,600,1225,720),(1630,595,1745,720),  # bare fingers (skin)
                   (650,232,786,346)],                                             # Duty patch (replaced)
 'dolg_exo_legs':[(0,1290,1000,2048)],     # crotch armour: brown dirt and rust, not paint
}
INCLUDE={   # paint inside an excluded area
 'dolg_exo_legs':[(400,1640,650,1745)],    # red tab on the crotch-armour belt
}
def paint_mask(name,L,C,h):
    w=hue_weight(h)*smooth(C,0.014,0.030)*smooth(L,0.04,0.10)
    ex=np.zeros(L.shape,bool)
    for r in EXCLUDE.get(name,[]): rect(ex,*r)
    for x0,y0,x1,y1 in INCLUDE.get(name,[]): ex[y0:y1,x0:x1]=False
    w=w*(1-ndi.gaussian_filter(ex.astype(float),2.0))
    return ndi.gaussian_filter(w,0.6)

def recolour(name, extra_exclude=None):
    px,meta=tga.read(TEX/f'{name}.tga'); rgb=px[...,:3].astype(np.float64)
    L,C,h=cl.srgb8_to_oklch(rgb)
    w=paint_mask(name,L,C,h)
    if extra_exclude is not None: w=w*(1-extra_exclude)
    sel=w>0.5
    hd=np.where(h>180,h-360,h)
    h_med=float(np.median(hd[sel])); cl_med=float(np.median((C/np.maximum(L,1e-4))[sel])); L_med=float(np.median(L[sel]))
    act=w>0.003   # only touch pixels the mask reaches
    Lq,Cq,hq,wq=L[act],C[act],hd[act],w[act]
    Ln=np.clip(Lq+(Lq*a.gain-Lq)*np.clip((0.75-Lq)/0.35,0,1),0,0.97)   # lift dark paint; already-light paint keeps its value
    hn=a.target_h+a.hue_keep*(hq-h_med)
    cln=np.minimum((Cq/np.maximum(Lq,1e-4))*(a.target_cl/cl_med),1.5*a.target_cl)
    Cn=cl.fit_gamut(Ln,cln*Ln,hn)
    new=cl.oklch_to_srgb8(Ln,Cn,hn)
    lin=cl.srgb_to_lin(rgb[act]/255)*(1-wq[:,None])+cl.srgb_to_lin(new/255)*wq[:,None]
    out=px.copy(); out[...,:3][act]=np.clip(cl.lin_to_srgb(lin)*255+0.5,0,255).astype(np.uint8)
    Image.fromarray((w*255).astype(np.uint8)).save(MASKS/f'{name}_paint_mask.png')
    Ls,Cs,hs=cl.srgb8_to_oklch(out[...,:3][sel].astype(float))
    stats[name]={'mask_px_w>0.5':int(sel.sum()),'source':{'h_med':round(h_med,1),'C/L_med':round(cl_med,3),'L_med':round(L_med,3)},
                 'result':{'h_med':round(float(np.median(hs)),1),'C/L_med':round(float(np.median(Cs/np.maximum(Ls,1e-4))),3),'L_med':round(float(np.median(Ls)),3),
                           'srgb_med':[int(v) for v in np.median(out[...,:3][sel],0)]}}
    return out,meta,w

# --- 1. recolour the paint and fabric accents ----------------------------------
for name in ['dolg_exo_legs','dolg_exo_helmet','dold_exo_attach']:
    out,meta,_=recolour(name); tga.write(TEX/f'{name}.tga',out,meta)

# --- 2. chest sign: recolour the red rim edge, paint out the star --------------
a3=np.asarray(Image.open(a.sign_layers/'03.png'))[...,3]; a4=np.asarray(Image.open(a.sign_layers/'04.png'))[...,3]
star=np.maximum(a3,a4)>8; cy,cx=ndi.center_of_mass(star)
yy,xx=np.mgrid[:512,:512]; r=np.hypot(yy-cy,xx-cx)
hole=ndi.binary_dilation(star,iterations=3)&(r<113)
src=(r<110)&~ndi.binary_dilation(star,iterations=6)
sd,smeta,_=recolour('dol_sign',extra_exclude=ndi.gaussian_filter(ndi.binary_dilation(star,iterations=6).astype(float),1.5))
df=sd[...,:3].astype(float)
low=inpaint.radial_low(df,src,cy,cx,113)
nd,_=inpaint.fill(df,hole,src,block=16,feather=1.5,low=low,seed=3)
sd[...,:3]=np.clip(nd+0.5,0,255).astype(np.uint8); tga.write(TEX/'dol_sign.tga',sd,smeta)
sn,snmeta=tga.read(TEX/'dol_sign _nmap.tga')
lat=inpaint.find_lattice(sn[...,0].astype(float),src,maxp=24)
nlow=inpaint.radial_low(sn[...,:2].astype(float),src,cy,cx,113,mode='normal')
nn,_=inpaint.fill(sn[...,:2].astype(float),hole,src,block=16,lattice=(lat[0],lat[1]),feather=1.5,low=nlow,seed=4)
v=np.clip(nn,0,255)/127.5-1; z=np.sqrt(np.clip(1-(v**2).sum(-1),0,1))
sn=sn.copy(); sn[...,:2]=np.clip(nn+0.5,0,255).astype(np.uint8); sn[...,2]=np.where(hole|(ndi.gaussian_filter(hole.astype(float),1.5)>0.01),np.clip(z*127.5+127.5+0.5,0,255).astype(np.uint8),sn[...,2])
tga.write(TEX/'dol_sign _nmap.tga',sn,snmeta)
Image.fromarray((hole*255).astype(np.uint8)).save(MASKS/'dol_sign_star_hole.png')
stats['dol_sign']['star']={'centre_px':[round(cx,1),round(cy,1)],'hole_px':int(hole.sum()),'normal_lattice':[lat[0].tolist(),lat[1].tolist()]}

# --- 3. torso: recolour, then replace the Duty patch with the Freedom patch -----
tdf,tmeta,_=recolour('dolg_exo_torso')
orig_full,_=tga.read(a.src/'dolg_exo_torso.tga')
tnf,tnmeta=tga.read(TEX/'dolg_exo_torso_nmap.tga')
WY0,WY1,WX0,WX1=130,470,550,900          # working window (texels)
td=tdf[WY0:WY1,WX0:WX1].copy(); tn=tnf[WY0:WY1,WX0:WX1].copy(); orig_t=orig_full[WY0:WY1,WX0:WX1]
HH,WW=td.shape[:2]
# Duty shield: convex hull of its red embroidery/border (stripe below excluded)
Y0,Y1,X0,X1=220-WY0,370-WY0,640-WX0,800-WX0
c=orig_t[Y0:Y1,X0:X1,:3].astype(float); L,C,h=cl.srgb8_to_oklch(c)
red=(C>0.022)&((h<45)|(h>340))&(L>0.12); red[Y1-Y0-27:,:]=False
core=ndi.binary_fill_holes(ndi.binary_closing(red,np.ones((5,5)),iterations=2)); core=ndi.binary_opening(core,np.ones((3,3)))
lab,nl=ndi.label(core); core=lab==(1+int(np.argmax(ndi.sum(core,lab,range(1,nl+1)))))
rr=ndi.binary_dilation(core,iterations=14)&red; py_,px_=np.nonzero(rr|core); pts=np.stack([px_,py_],1)
hull=Delaunay(pts[ConvexHull(pts).vertices]); YY,XX=np.mgrid[:Y1-Y0,:X1-X0]
shield_c=hull.find_simplex(np.stack([XX.ravel(),YY.ravel()],1)).reshape(YY.shape)>=0
shield=np.zeros((HH,WW),bool); shield[Y0:Y1,X0:X1]=shield_c
ys,xs=np.nonzero(shield); scy,scx=ys.mean(),xs.mean()
cols=np.arange(xs.min()+8,xs.max()-8); tops=[np.nonzero(shield[:,x])[0].min() for x in cols]
tilt=float(np.degrees(np.arctan(np.polyfit(cols,tops,1)[0])))     # top edge slope (image y down)
Lp,Cp,_=cl.srgb8_to_oklch(orig_t[shield][:, :3].astype(float))
duty_L=float(np.median(Lp)); duty_C=float(np.percentile(Cp,90))
# 3a. remove the Duty patch (and its drop shadow) from diffuse and normal map
phole=ndi.binary_dilation(shield,iterations=5)
ring=ndi.binary_dilation(phole,iterations=40)&~ndi.binary_dilation(phole,iterations=2)
Lt,Ct,ht=cl.srgb8_to_oklch(td[...,:3].astype(float))
fabric_src=ring&~((Ct>0.02)&(Lt>0.08))&((np.arange(HH)+WY0)[:,None]<343)
latd=inpaint.find_lattice(tn[...,0].astype(float),fabric_src,maxp=20)
LAT=(latd[0],latd[1]) if latd and latd[1] is not None else None
ff,_=inpaint.fill(td[...,:3].astype(float),phole,fabric_src,block=14,lattice=LAT,feather=1.2,seed=5)
fn,_=inpaint.fill(tn[...,:2].astype(float),phole,fabric_src,block=14,lattice=LAT,feather=1.2,seed=6)
removed=td.copy(); removed[...,:3]=np.clip(ff+0.5,0,255).astype(np.uint8)
# 3b. the Freedom patch: same area, centre and tilt as the Duty patch
P=Image.open(a.patch).convert('RGBA'); pa=np.asarray(P)[...,3]>127
scale=float(np.sqrt(shield.sum()/pa.sum()))
SS=4  # resample at 4x then down, for clean edges
big=P.resize((round(P.width*scale*SS),round(P.height*scale*SS)),Image.LANCZOS).rotate(-tilt,Image.BICUBIC,expand=True)
small=big.resize((round(big.width/SS),round(big.height/SS)),Image.LANCZOS)
canvas=Image.new('RGBA',(WW,HH)); ox=round(scx-small.width/2); oy=round(scy-small.height/2); canvas.paste(small,(ox,oy))
F=np.asarray(canvas).astype(np.float64)
# a stitched edge is not a cut line: soften and roughen the outline a little
rng=np.random.default_rng(7)
fa=ndi.gaussian_filter(F[...,3]/255,0.7)
edge=(fa>0.05)&(fa<0.95)
fa=np.clip(fa+edge*(ndi.gaussian_filter(rng.standard_normal(fa.shape),0.8)*0.35),0,1)
for c in range(3): F[...,c]=ndi.gaussian_filter(F[...,c],0.45)   # thread fuzz at texel scale
FL,FC,Fh=cl.srgb8_to_oklch(F[...,:3]); inside=fa>0.5
f_L=float(np.median(FL[inside])); f_C=float(np.percentile(FC[inside],90))
L_ratio=duty_L/f_L; C_ratio=min(0.85,duty_C/f_C)   # faded towards the Duty patch saturation
fl,_,_=cl.srgb8_to_oklch(ff)
shade=ndi.gaussian_filter(fl,6); shade=shade/np.median(shade[inside])          # cloth folds
dirt=np.clip(fl/np.maximum(ndi.gaussian_filter(fl,10),1e-3),0.6,1.35)          # dust and stains
grime=fl-ndi.gaussian_filter(fl,2.5)
nL=np.clip(FL*L_ratio*shade**0.6*dirt**0.6+0.5*grime,0,1); nC=FC*C_ratio*np.clip(shade,0.7,1.2)*np.clip(dirt,0.7,1.1)
nrgb=cl.oklch_to_srgb8(nL,cl.fit_gamut(nL,nC,Fh),Fh)
nrgb=nrgb*0.92+ff*0.08                                                          # dust settles in the weave
sh=ndi.shift(ndi.gaussian_filter(fa,1.3),(1.6,1.0),order=1)*0.42   # patch sits proud of the cloth
base=cl.srgb_to_lin(ff/255)*(1-sh[...,None])
lin=base*(1-fa[...,None])+cl.srgb_to_lin(nrgb/255)*fa[...,None]
reg=ndi.binary_dilation(phole,iterations=6)
td[...,:3]=np.where(reg[...,None],np.clip(cl.lin_to_srgb(lin)*255+0.5,0,255),td[...,:3]).astype(np.uint8)
tdf[WY0:WY1,WX0:WX1]=td; tga.write(TEX/'dolg_exo_torso.tga',tdf,tmeta)
# 3c. normal map: raised patch + embroidery relief over the patch-free fabric normal
emb=FL-ndi.gaussian_filter(FL,1.6)
height=ndi.gaussian_filter(fa,0.9)*1.0+emb*fa*2.2
gy,gx=np.gradient(height)
band=ndi.binary_dilation(shield,iterations=2)&~ndi.binary_erosion(shield,iterations=2)
on=tn[...,:2].astype(float)/127.5-1; fnv=fn/127.5-1
orig_relief=float(np.median(np.hypot(*(on-fnv)[band].T)))
top_band=band&(np.arange(HH)[:,None]<ys.min()+4); g_top=float(np.median((on-fnv)[top_band][:,1]))
gs=1 if g_top>=0 else -1     # green channel convention, read off the Duty patch's top edge
m=float(np.median(np.hypot(gx,gy)[band])); k=orig_relief/max(m,1e-6)
nxy=fnv+np.stack([-gx*k,gy*k*gs],-1)
nz=np.sqrt(np.clip(1-(nxy**2).sum(-1),0.02,1)); nrm=np.dstack([nxy,nz]); nrm/=np.linalg.norm(nrm,axis=-1,keepdims=True)
for ch in range(3): tn[...,ch]=np.where(reg,np.clip(nrm[...,ch]*127.5+127.5+0.5,0,255),tn[...,ch]).astype(np.uint8)
tnf[WY0:WY1,WX0:WX1]=tn; tga.write(TEX/'dolg_exo_torso_nmap.tga',tnf,tnmeta)
def full(m):
    z=np.zeros((2048,2048),np.float64); z[WY0:WY1,WX0:WX1]=m; return z
Image.fromarray((full(shield)*255).astype(np.uint8)).save(MASKS/'dolg_exo_torso_duty_patch.png')
Image.fromarray((np.clip(full(fa),0,1)*255).astype(np.uint8)).save(MASKS/'dolg_exo_torso_freedom_patch_alpha.png')
Image.fromarray(removed[...,:3]).save(MASKS/'dolg_exo_torso_patch_removed_window.png')
xs_,ys_=xs+WX0,ys+WY0
stats['dolg_exo_torso']['patch']={'duty_shield_bbox_xyxy':[int(xs_.min()),int(ys_.min()),int(xs_.max()),int(ys_.max())],
 'uv_bbox':[round(xs_.min()/2048,4),round(ys_.min()/2048,4),round((xs_.max()+1)/2048,4),round((ys_.max()+1)/2048,4)],
 'centre_px':[round(scx+WX0,1),round(scy+WY0,1)],'area_px':int(shield.sum()),'top_edge_slope_deg':round(tilt,2),
 'freedom_scale':round(scale,4),'freedom_size_px':[small.width,small.height],'freedom_origin_px':[ox+WX0,oy+WY0],
 'grade':{'duty_L_med':round(duty_L,3),'L_ratio':round(L_ratio,3),'C_ratio':round(C_ratio,3)},
 'fabric_lattice':[latd[0].tolist(),latd[1].tolist()] if LAT else None,'normal_green_sign':gs,'relief_gain':round(k,3),'orig_relief':round(orig_relief,4)}
(OUT/'stats.json').write_text(json.dumps(stats,indent=1))
print(json.dumps(stats,indent=1))
