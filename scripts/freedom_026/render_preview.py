# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline preview of the ported Duty/Freedom exo meshes with a given texture set.
Mesh loading and the rasteriser follow resource-porter scripts/render-duty-026-icons.py.
usage: render_preview.py BUNDLE TEXDIR OUTDIR [--views name:yaw:pitch[:zoomx,zoomy,zoomw,zoomh] ...]
"""
import sys, struct, numpy as np
from pathlib import Path
from PIL import Image
def mesh(root, part, surface, texture):
    b=(root/f'duty_03_{part}.skinned_model/render/{surface}/converted_model').read_bytes()
    n=struct.unpack_from('<I',b,8)[0]
    raw=np.frombuffer(b[12:12+48*n],np.uint8).reshape(n,48)
    f=raw.copy().view('<f4').reshape(n,12)
    p=f[:,:3].astype(float); uv=f[:,10:12].astype(float)
    nr=raw[:,28:31][:,::-1].astype(float)/127.5-1
    faces=np.frombuffer(b,dtype='<u2',offset=24+48*n).reshape(-1,3)
    tex=np.asarray(Image.open(texture).convert('RGB'),dtype=np.float32)/255
    return p,uv,nr,faces,tex
def render(meshes,yaw,pitch,size,crop=None,ss=2):
    W,H=size[0]*ss,size[1]*ss
    yaw,pitch=np.radians([yaw,pitch])
    toward=np.array([np.sin(yaw)*np.cos(pitch),np.sin(pitch),np.cos(yaw)*np.cos(pitch)])
    right=np.cross([0,1,0],toward); right/=np.linalg.norm(right); up=np.cross(toward,right)
    basis=np.stack([right,up,toward],axis=1)
    allp=np.concatenate([m[0] for m in meshes])@basis
    lo,hi=allp.min(0),allp.max(0)
    if crop: # crop in normalised [0,1] of the full-body frame: cx,cy,w,h
        cx,cy,cw,chh=crop; lo2=lo.copy(); hi2=hi.copy()
        lo2[0]=lo[0]+(hi[0]-lo[0])*(cx-cw/2); hi2[0]=lo[0]+(hi[0]-lo[0])*(cx+cw/2)
        hi2[1]=hi[1]-(hi[1]-lo[1])*(cy-chh/2); lo2[1]=hi[1]-(hi[1]-lo[1])*(cy+chh/2); lo,hi=lo2,hi2
    scale=min((W-32*ss)/(hi[0]-lo[0]),(H-16*ss)/(hi[1]-lo[1])); center=(lo+hi)/2
    out=np.zeros((H,W,3),np.float32); out[:]=(30/255,33/255,36/255); depth=np.full((H,W),-np.inf)
    light=right*-0.45+up*0.75+toward*0.8; light/=np.linalg.norm(light)
    for p,uv,nr,faces,tex in meshes:
        pp=p@basis; sc=(pp[:,:2]-center[:2])*[scale,-scale]+[W/2,H/2]
        for face in faces:
            q=sc[face]; z=pp[face,2]
            x0,y0=np.maximum(np.floor(q.min(0)).astype(int),0); x1,y1=np.minimum(np.ceil(q.max(0)).astype(int),[W-1,H-1])
            if x0>x1 or y0>y1: continue
            den=(q[1,1]-q[2,1])*(q[0,0]-q[2,0])+(q[2,0]-q[1,0])*(q[0,1]-q[2,1])
            if abs(den)<1e-8: continue
            yy,xx=np.mgrid[y0:y1+1,x0:x1+1]; xx=xx+.5; yy=yy+.5
            a=((q[1,1]-q[2,1])*(xx-q[2,0])+(q[2,0]-q[1,0])*(yy-q[2,1]))/den
            b=((q[2,1]-q[0,1])*(xx-q[2,0])+(q[0,0]-q[2,0])*(yy-q[2,1]))/den
            c=1-a-b; dz=a*z[0]+b*z[1]+c*z[2]
            vis=(a>=0)&(b>=0)&(c>=0)&(dz>depth[y0:y1+1,x0:x1+1])
            if not vis.any(): continue
            w=np.stack([a[vis],b[vis],c[vis]],1); tuv=w@uv[face]
            th,tw=tex.shape[:2]
            fx=(tuv[:,0]%1)*tw-.5; fy=(tuv[:,1]%1)*th-.5
            ix=np.floor(fx).astype(int); iy=np.floor(fy).astype(int); dx=(fx-ix)[:,None]; dy=(fy-iy)[:,None]
            ix0=ix%tw; iy0=iy%th; ix1=(ix+1)%tw; iy1=(iy+1)%th
            col=(tex[iy0,ix0]*(1-dx)*(1-dy)+tex[iy0,ix1]*dx*(1-dy)+tex[iy1,ix0]*(1-dx)*dy+tex[iy1,ix1]*dx*dy)
            nrm=w@nr[face]; nrm/=np.maximum(np.linalg.norm(nrm,axis=1)[:,None],1e-8)
            ill=.38+.75*np.maximum(nrm@light,0)
            patch=out[y0:y1+1,x0:x1+1]; patch[vis]=np.clip(col*ill[:,None]*[1.05,1.01,.94],0,1)
            depth[y0:y1+1,x0:x1+1][vis]=dz[vis]
    # the porter's icon rasteriser is mirrored relative to the game (chest sign and
    # mask canister swap sides); flip so previews match the lobby
    im=Image.fromarray((out*255+.5).astype(np.uint8)).transpose(Image.FLIP_LEFT_RIGHT)
    return im.resize(size,Image.LANCZOS)
def main():
    bundle=Path(sys.argv[1]); T=Path(sys.argv[2]); out=Path(sys.argv[3]); out.mkdir(parents=True,exist_ok=True)
    root=bundle/'overlay/models/ported/duty_026'
    def m(part,surf,tex): return mesh(root,part,surf,T/(tex+'.tga'))
    full=[m('torso_s1','torso_s','dolg_exo_torso'),m('torso_s1','sign_s','dol_sign'),m('torso_s1','exoskeleton_s','dold_exo_attach'),
          m('torso_s1','back_s','dold_exo_attach'),m('legs_s1','legs_s','dolg_exo_legs'),m('hand','torso_s','dolg_exo_torso'),
          m('boot','legs_s','dolg_exo_legs'),m('head','helmet_s','dolg_exo_helmet')]
    views=[a for a in sys.argv[4:]] or ['front:15:6','back:195:6']
    for v in views:
        parts=v.split(':'); name,yaw,pitch=parts[0],float(parts[1]),float(parts[2])
        crop=tuple(map(float,parts[3].split(','))) if len(parts)>3 else None
        size=(700,1000) if not crop else (700,700)
        render(full,yaw,pitch,size,crop).save(out/f'{name}.png'); print('rendered',name,flush=True)
main()
