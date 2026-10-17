# SPDX-License-Identifier: GPL-3.0-or-later
import numpy as np
def srgb_to_lin(c):
    c=np.asarray(c,np.float64)
    return np.where(c<=0.04045,c/12.92,((c+0.055)/1.055)**2.4)
def lin_to_srgb(c):
    c=np.clip(c,0,1); return np.where(c<=0.0031308,c*12.92,1.055*np.power(c,1/2.4)-0.055)
M1=np.array([[0.4122214708,0.5363325363,0.0514459929],[0.2119034982,0.6806995451,0.1073969566],[0.0883024619,0.2817188376,0.6299787005]])
M2=np.array([[0.2104542553,0.7936177850,-0.0040720468],[1.9779984951,-2.4285922050,0.4505937099],[0.0259040371,0.7827717662,-0.8086757660]])
def lin_to_oklab(rgb):
    lms=rgb@M1.T; lms=np.cbrt(lms); return lms@M2.T
def oklab_to_lin(lab):
    lms=lab@np.linalg.inv(M2).T; lms=lms**3; return lms@np.linalg.inv(M1).T
def srgb8_to_oklch(a):
    lab=lin_to_oklab(srgb_to_lin(a/255.0))
    L=lab[...,0]; C=np.hypot(lab[...,1],lab[...,2]); h=np.degrees(np.arctan2(lab[...,2],lab[...,1]))%360
    return L,C,h
def oklch_to_srgb8(L,C,h):
    hr=np.radians(h); lab=np.stack([L,C*np.cos(hr),C*np.sin(hr)],-1)
    lin=oklab_to_lin(lab); return lin_to_srgb(lin)*255
def fit_gamut(L,C,h,iters=18):
    """Reduce chroma until the colour lies inside sRGB (binary search per pixel)."""
    lo=np.zeros_like(C); hi=C.copy()
    def ok(c):
        hr=np.radians(h); lin=oklab_to_lin(np.stack([L,c*np.cos(hr),c*np.sin(hr)],-1))
        return np.all((lin>=-1e-4)&(lin<=1+1e-4),-1)
    good=ok(C); res=np.where(good,C,0.0)
    lo=np.zeros_like(C); hi=C.copy()
    for _ in range(iters):
        mid=(lo+hi)/2; g=ok(mid); lo=np.where(g,mid,lo); hi=np.where(g,hi,mid)
    return np.where(good,C,lo)
