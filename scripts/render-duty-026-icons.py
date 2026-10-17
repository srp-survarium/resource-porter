#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the actual Duty meshes into the native lobby's equipment thumbnail atlas.

Requires numpy and Pillow. Reads an unpacked Duty bundle and the native lobby
SWF. Preserves every existing atlas pixel and every other SWF tag. Orthographic
cameras match the native category poses; outputs 4x renders and a comparison.
"""
import argparse
from pathlib import Path
import struct
import zlib
import numpy as np
from PIL import Image, ImageDraw


def swf_tags(path):
    b = path.read_bytes()
    assert b[:3] in (b'CWS', b'FWS')
    if b[:3] == b'CWS':
        b = b[:8] + zlib.decompress(b[8:])
    at = 8 + (5 + 4 * (b[8] >> 3) + 7) // 8 + 4
    prefix = b[:at]
    tags = []
    while at < len(b):
        header = struct.unpack_from('<H', b, at)[0]; at += 2
        size = header & 63
        if size == 63:
            size = struct.unpack_from('<I', b, at)[0]; at += 4
        tags.append((header >> 6, b[at:at+size])); at += size
    return prefix, tags


def atlas_tag(tags):
    # Resolve the embedded bitmap's symbol rather than relying on a tag number.
    char_id = None
    for kind, payload in tags:
        if kind != 76: continue
        at = 2
        for _ in range(struct.unpack_from('<H', payload)[0]):
            ident = struct.unpack_from('<H', payload, at)[0]; at += 2
            end = payload.index(0, at)
            if payload[at:end] == b'LobbyMenu_EquipmentIcons': char_id = ident
            at = end + 1
    assert char_id is not None, 'equipment bitmap symbol missing'
    for index, (kind, payload) in enumerate(tags):
        if kind == 36 and struct.unpack_from('<H', payload)[0] == char_id:
            ident, fmt, w, h = struct.unpack_from('<HBHH', payload)
            assert fmt == 5
            argb = np.frombuffer(zlib.decompress(payload[7:]), np.uint8).reshape(h,w,4).copy()
            return index, ident, argb
    raise ValueError('equipment bitmap is not lossless ARGB')


def mesh(root, part, surface, texture, selection=None):
    b = (root/f'duty_03_{part}.skinned_model/render/{surface}/converted_model').read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    raw = np.frombuffer(b[12:12+48*n], np.uint8).reshape(n,48)
    floats = raw.copy().view('<f4').reshape(n,12)
    p = floats[:,:3].astype(float)
    uv = floats[:,10:12].astype(float)
    normals = raw[:,28:31][:,::-1].astype(float)/127.5-1
    faces = np.frombuffer(b,dtype='<u2',offset=24+48*n).reshape(-1,3)
    if selection: faces = faces[selection(p[faces])]
    used = np.unique(faces)
    tex = np.asarray(Image.open(texture).convert('RGB'), dtype=float)/255
    return p, uv, normals, faces, tex, used


def render(meshes, yaw, pitch, size=(684,332), roll=0):
    yaw,pitch = np.radians([yaw,pitch])
    toward = np.array([np.sin(yaw)*np.cos(pitch), np.sin(pitch), np.cos(yaw)*np.cos(pitch)])
    right = np.cross([0,1,0],toward); right /= np.linalg.norm(right)
    up = np.cross(toward,right)
    roll=np.radians(roll)
    right,up=right*np.cos(roll)+up*np.sin(roll),up*np.cos(roll)-right*np.sin(roll)
    basis = np.stack([right,up,toward],axis=1)
    allp = np.concatenate([m[0][m[5]] for m in meshes]) @ basis
    lo,hi = allp.min(0),allp.max(0)
    scale = min((size[0]-32)/(hi[0]-lo[0]), (size[1]-16)/(hi[1]-lo[1]))
    center = (lo+hi)/2
    out = np.zeros((size[1],size[0],4),np.uint8)
    depth = np.full((size[1],size[0]),-np.inf)
    light = right*-0.45+up*0.75+toward*0.8; light/=np.linalg.norm(light)
    for p,uv,normals,faces,tex,_ in meshes:
        pp = p @ basis
        screen = (pp[:,:2]-center[:2])*[scale,-scale]+[size[0]/2,size[1]/2]
        for face in faces:
            q=screen[face]; z=pp[face,2]
            x0,y0=np.maximum(np.floor(q.min(0)).astype(int),0)
            x1,y1=np.minimum(np.ceil(q.max(0)).astype(int),[size[0]-1,size[1]-1])
            if x0>x1 or y0>y1: continue
            den=(q[1,1]-q[2,1])*(q[0,0]-q[2,0])+(q[2,0]-q[1,0])*(q[0,1]-q[2,1])
            if abs(den)<1e-8:continue
            yy,xx=np.mgrid[y0:y1+1,x0:x1+1]; xx=xx+.5; yy=yy+.5
            a=((q[1,1]-q[2,1])*(xx-q[2,0])+(q[2,0]-q[1,0])*(yy-q[2,1]))/den
            b=((q[2,1]-q[0,1])*(xx-q[2,0])+(q[0,0]-q[2,0])*(yy-q[2,1]))/den
            c=1-a-b; dz=a*z[0]+b*z[1]+c*z[2]
            visible=(a>=0)&(b>=0)&(c>=0)&(dz>depth[y0:y1+1,x0:x1+1])
            if not visible.any():continue
            weights=np.stack([a[visible],b[visible],c[visible]],axis=1)
            tuv=weights@uv[face]
            tx=np.clip((tuv[:,0]%1)*(tex.shape[1]-1),0,tex.shape[1]-1).astype(int)
            ty=np.clip((tuv[:,1]%1)*(tex.shape[0]-1),0,tex.shape[0]-1).astype(int)
            normal=weights@normals[face]; normal/=np.maximum(np.linalg.norm(normal,axis=1)[:,None],1e-8)
            illumination=.38+.75*np.maximum(normal@light,0)
            rgb=np.clip(tex[ty,tx]*illumination[:,None]*[1.05,1.01,.94],0,1)
            patch=out[y0:y1+1,x0:x1+1]; patch[visible,:3]=(rgb*255).astype(np.uint8);patch[visible,3]=255
            depth[y0:y1+1,x0:x1+1][visible]=dz[visible]
    return Image.fromarray(out)


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bundle',type=Path,required=True)
    ap.add_argument('--textures',type=Path,required=True)
    ap.add_argument('--lobby-swf',type=Path,required=True)
    ap.add_argument('--output',type=Path,required=True)
    ap.add_argument('--icon',action='append',default=[],metavar='NAME=PNG',
                    help='Use an earlier 171x83 RGBA thumbnail for body or helmet instead of rendering it')
    ap.add_argument('--gallery',type=Path,help='Also render the body, the helmet and the set at full size here')
    args=ap.parse_args();args.output.mkdir(parents=True,exist_ok=True)
    given=dict(i.split('=',1) for i in args.icon)
    prefix,tags=swf_tags(args.lobby_swf); tag_index,char_id,argb=atlas_tag(tags)
    h,w=argb.shape[:2];assert w==855 and h==2048,(w,h)
    # New sixth column preserves row count, hence every existing icon index.
    expanded=np.zeros((h,w+171,4),np.uint8);expanded[:,:w]=argb
    root=args.bundle/'overlay/models/ported/duty_026'
    textures=args.textures
    def m(part,surf,tex,select=None):return mesh(root,part,surf,textures/(tex+'.tga'),select)
    body=[
        m('torso_s1','torso_s','dolg_exo_torso'),
        m('torso_s1','sign_s','dol_sign'),
        m('torso_s1','exoskeleton_s','dold_exo_attach'),
        m('torso_s1','back_s','dold_exo_attach'),
        m('legs_s1','legs_s','dolg_exo_legs'),
        m('hand','torso_s','dolg_exo_torso'),
        m('boot','legs_s','dolg_exo_legs'),
    ]
    helmet=[m('head','helmet_s','dolg_exo_helmet')]
    # (name, meshes, yaw, pitch, Duty cell, native Zubr UM-4 cell compared).
    # Row 2 of the sixth column is the Bandit balaclava's.
    specs=[('body',body,15,6,(1,6),(8,5)),('helmet',helmet,25,5,(3,6),(6,5))]
    assert set(given)<=set(s[0] for s in specs),given
    original=Image.fromarray(argb[:,:,[1,2,3,0]])
    comparison=Image.new('RGB',(760,len(specs)*180),(30,33,36));draw=ImageDraw.Draw(comparison)
    for i,(name,meshes,yaw,pitch,(row,col),(nrow,ncol)) in enumerate(specs):
        assert col==6
        if name in given:
            icon=Image.open(given[name]).convert('RGBA');assert icon.size==(171,83),icon.size
            shown=icon.resize((342,166),Image.Resampling.LANCZOS)
            print(name,'icon from',given[name],flush=True)
        else:
            rendered=render(meshes,yaw,pitch)
            rendered.save(args.output/(name+'-render.png'))
            icon=rendered.resize((171,83),Image.Resampling.LANCZOS)
            shown=rendered.resize((342,166),Image.Resampling.LANCZOS)
            print(name,'camera',yaw,pitch,'triangles',sum(len(m[3]) for m in meshes),flush=True)
        icon.save(args.output/(name+'.png'))
        y=(row-1)*91
        rgba=np.asarray(icon).copy();a=rgba[:,:,3:4].astype(np.uint16)
        premult=(rgba[:,:,:3].astype(np.uint16)*a+127)//255
        expanded[y:y+83,w:,0]=rgba[:,:,3]
        expanded[y:y+83,w:,1:]=premult.astype(np.uint8)
        og=original.crop(((ncol-1)*171,(nrow-1)*91,ncol*171,(nrow-1)*91+83)).resize((342,166))
        comparison.paste(og,(0,i*180),og)
        comparison.paste(shown,(370,i*180),shown)
        draw.text((5,i*180+160),'Zubr UM-4 / Duty '+name,fill='white')
    comparison.save(args.output/'comparison.png')
    if args.gallery:
        args.gallery.mkdir(parents=True,exist_ok=True)
        for name,meshes,size in [('body-without-helmet',body,(600,900)),('helmet',helmet,(600,600)),('full-set',body+helmet,(600,900))]:
            image=render(meshes,15 if name!='helmet' else 25,6,size=size)
            backdrop=Image.new('RGBA',image.size,(30,33,36,255));backdrop.alpha_composite(image)
            backdrop.convert('RGB').save(args.gallery/(name+'.png'))
            print('gallery',name,flush=True)
    tags[tag_index]=(36,struct.pack('<HBHH',char_id,5,w+171,h)+zlib.compress(expanded.tobytes(),9))
    chunks=[prefix[8:]]
    for kind,payload in tags:
        chunks.extend([struct.pack('<HI',(kind<<6)|63,len(payload)),payload])
    body_bytes=b''.join(chunks)
    swf=b'CWS'+prefix[3:4]+struct.pack('<I',8+len(body_bytes))+zlib.compress(body_bytes,9)
    (args.output/'lobby_menu.swf').write_bytes(swf)
    # Re-read and check the entire native atlas is byte-for-byte untouched.
    _,checktags=swf_tags(args.output/'lobby_menu.swf')
    _,_,check=atlas_tag(checktags)
    assert np.array_equal(check[:,:w],argb)
    assert all(a==b for j,(a,b) in enumerate(zip(tags,checktags)) if j!=tag_index)


if __name__=='__main__':main()
