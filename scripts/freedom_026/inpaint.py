# SPDX-License-Identifier: GPL-3.0-or-later
"""Texture-preserving hole filling: low frequencies from normalised convolution of the
surrounding pixels, high frequencies quilted from known pixels (optionally on the
texture's own repeat lattice so periodic weave/knurl patterns stay aligned)."""
import numpy as np
from scipy import ndimage as ndi
def normconv_fill(img, known, sigmas=(4,10,24,60)):
    """Smooth fill of unknown pixels; coarser sigmas take over where finer ones lack support."""
    out=np.zeros_like(img); have=np.zeros(img.shape[:2])
    for s in sigmas:
        k=ndi.gaussian_filter(known.astype(np.float64),s)
        v=np.stack([ndi.gaussian_filter(img[...,c]*known,s) for c in range(img.shape[2])],-1)/np.maximum(k,1e-12)[...,None]
        w=np.clip(k/0.15,0,1)*(1-have)
        out+=v*w[...,None]; have+=w
    return out/np.maximum(have,1e-9)[...,None]
def find_lattice(gray, region, maxp=40):
    """Two shortest independent repeat vectors of the pattern inside `region` (bool)."""
    ys,xs=np.nonzero(region); y0,y1,x0,x1=ys.min(),ys.max()+1,xs.min(),xs.max()+1
    g=gray[y0:y1,x0:x1]; m=region[y0:y1,x0:x1].astype(float)
    hp=(g-ndi.gaussian_filter(g,3))*m
    F=np.fft.rfft2(hp,s=(2*hp.shape[0],2*hp.shape[1])); ac=np.fft.irfft2(F*np.conj(F))
    M=np.fft.rfft2(m,s=ac.shape); norm=np.fft.irfft2(M*np.conj(M)); ac=ac/np.maximum(norm,1)
    H,W=ac.shape; cand=[]
    for dy in range(-maxp,maxp+1):
        for dx in range(-maxp,maxp+1):
            if (dy,dx)<=(0,0) or dy*dy+dx*dx<9: continue
            cand.append((ac[dy%H,dx%W],dy,dx))
    peak=ndi.maximum_filter(ac,size=5,mode='wrap')
    cand=[c for c in cand if ac[c[1]%H,c[2]%W]>=peak[c[1]%H,c[2]%W] and c[0]>0.25*ac[0,0]]
    cand.sort(key=lambda c:(c[1]**2+c[2]**2))
    if not cand: return None
    v1=np.array(cand[0][1:])
    for c in cand[1:]:
        v2=np.array(c[1:])
        if abs(v1[0]*v2[1]-v1[1]*v2[0])>4: return v1,v2,[round(float(cand[0][0]/ac[0,0]),2),round(float(c[0]/ac[0,0]),2)]
    return v1,None,None
def quilt_fill(img, hole, source, block=20, lattice=None, rng=None, tries=400):
    """High-pass detail for `hole` pixels, copied block-wise from `source` pixels."""
    rng=rng or np.random.default_rng(1); H,W=hole.shape
    ys,xs=np.nonzero(hole); y0,y1,x0,x1=ys.min(),ys.max()+1,xs.min(),xs.max()+1
    sy,sx=np.nonzero(source)
    acc=np.zeros(img.shape); wsum=np.zeros((H,W))
    step=block//2; r=block//2
    win=np.outer(np.hanning(block+2)[1:-1],np.hanning(block+2)[1:-1])
    src_ok=ndi.binary_erosion(source,np.ones((block,block)))
    oky,okx=np.nonzero(src_ok)
    for cy in range(y0-step,y1+step,step):
        for cx in range(x0-step,x1+step,step):
            if not hole[max(cy-r,0):cy+r,max(cx-r,0):cx+r].any(): continue
            for _ in range(tries):
                k=rng.integers(len(oky)); ty,tx=oky[k],okx[k]
                if lattice is not None:
                    v1,v2=lattice; A=np.array([v1,v2],float).T
                    a,b=np.linalg.solve(A,[ty-cy,tx-cx]); a,b=round(a),round(b)
                    ty,tx=cy+a*v1[0]+b*v2[0],cx+a*v1[1]+b*v2[1]
                    if not(0<=ty<H and 0<=tx<W and src_ok[ty,tx]): continue
                break
            else: continue
            dy,dx=ty-cy,tx-cx
            ya,yb,xa,xb=cy-r,cy-r+block,cx-r,cx-r+block
            if ya<0 or xa<0 or yb>H or xb>W or ya+dy<0 or xa+dx<0 or yb+dy>H or xb+dx>W: continue
            acc[ya:yb,xa:xb]+=img[ya+dy:yb+dy,xa+dx:xb+dx]*win[...,None]; wsum[ya:yb,xa:xb]+=win
    return acc/np.maximum(wsum,1e-9)[...,None], wsum>1e-6
def fill(img, hole, source, sigma_hp=4, block=20, lattice=None, feather=2.0, seed=1, low=None):
    """img float (H,W,C). Returns img with `hole` replaced: smooth fill + quilted detail.
    Works in a window around hole and source only."""
    ys,xs=np.nonzero(hole|source); pad=70; H,W=hole.shape
    y0,y1,x0,x1=max(ys.min()-pad,0),min(ys.max()+pad+1,H),max(xs.min()-pad,0),min(xs.max()+pad+1,W)
    if (y0,y1,x0,x1)!=(0,H,0,W):
        sub,w=fill(img[y0:y1,x0:x1],hole[y0:y1,x0:x1],source[y0:y1,x0:x1],sigma_hp,block,lattice,feather,seed,
                   None if low is None else low[y0:y1,x0:x1])
        out=img.copy(); out[y0:y1,x0:x1]=sub; ww=np.zeros(hole.shape); ww[y0:y1,x0:x1]=w; return out,ww
    known=~hole
    if low is None: low=normconv_fill(img,known)
    detail_src=img-np.stack([ndi.gaussian_filter(img[...,c],sigma_hp) for c in range(img.shape[2])],-1)
    q,ok=quilt_fill(detail_src,hole,source&known,block,lattice,np.random.default_rng(seed))
    new=low+q
    w=ndi.gaussian_filter(hole.astype(float),feather) if feather else hole.astype(float)
    w=np.maximum(w,hole)  # fully replace the hole itself, feather only outward
    return img*(1-w[...,None])+new*w[...,None], w

def radial_low(img, known, cy, cx, rmax, mode='scalar', bins=1.0, green_sign=-1):
    """Rotationally symmetric low-frequency model of a disc (scalar channels, or a
    2-channel normal XY field modelled as radial component f(r) plus a constant tilt)."""
    H,W=img.shape[:2]; yy,xx=np.mgrid[:H,:W]; r=np.hypot(yy-cy,xx-cx); ri=np.minimum((r/bins).astype(int),int(rmax/bins))
    sel=known&(r<rmax); nb=int(rmax/bins)+1
    if mode=='scalar':
        out=np.zeros_like(img)
        for c in range(img.shape[2]):
            s=np.bincount(ri[sel],img[...,c][sel],nb); n=np.bincount(ri[sel],None,nb)
            prof=ndi.gaussian_filter1d(np.where(n>0,s/np.maximum(n,1),np.nan),0.1)
            idx=np.arange(nb); good=n>0; prof=np.interp(idx,idx[good],(s/np.maximum(n,1))[good]); prof=ndi.gaussian_filter1d(prof,2)
            out[...,c]=prof[ri]
        return out
    v=img[...,:2]/127.5-1; v=v*[1,green_sign]; ux=(xx-cx)/np.maximum(r,1e-6); uy=(yy-cy)/np.maximum(r,1e-6)
    rad=v[...,0]*ux+v[...,1]*uy
    tilt=np.array([ (v[...,0]-rad*ux)[sel].mean(), (v[...,1]-rad*uy)[sel].mean()])
    s=np.bincount(ri[sel],rad[sel],nb); n=np.bincount(ri[sel],None,nb); idx=np.arange(nb); good=n>0
    prof=ndi.gaussian_filter1d(np.interp(idx,idx[good],(s/np.maximum(n,1))[good]),2)
    f=prof[ri]; out=np.stack([f*ux+tilt[0],f*uy+tilt[1]],-1)
    return (out*[1,green_sign]+1)*127.5
