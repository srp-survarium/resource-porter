# SPDX-License-Identifier: GPL-3.0-or-later
"""Flat before/after texture comparisons for the gallery. usage: gallery_images.py SRC BUILD OUT REF_PNG REF_SAMPLES_PNG PATCH_PNG"""
import sys, numpy as np
from pathlib import Path
from PIL import Image, ImageDraw
sys.path.insert(0,str(Path(__file__).parent)); import tga
src,build,out=Path(sys.argv[1]),Path(sys.argv[2]),Path(sys.argv[3]); out.mkdir(parents=True,exist_ok=True)
V=['freedom','freedom-bright']
def rd(d,n): return tga.read(d/f'{n}.tga')[0]
def jpg(im,name,q=90): im.convert('RGB').save(out/name,quality=q)
def label(im,text):
    d=ImageDraw.Draw(im); d.rectangle((0,0,8+7*len(text),18),fill=(0,0,0)); d.text((4,3),text,fill=(255,255,255)); return im
# full textures: before / freedom / bright (1024 each)
for n in ['dolg_exo_torso','dolg_exo_legs','dolg_exo_helmet','dold_exo_attach','dol_sign']:
    s=1024 if n!='dol_sign' else 512
    ims=[Image.fromarray(rd(src,n)[...,:3]).resize((s,s),Image.LANCZOS)]+[Image.fromarray(rd(build/v/'textures',n)[...,:3]).resize((s,s),Image.LANCZOS) for v in V]
    m=Image.open(build/'freedom'/'masks'/f'{n}_paint_mask.png').resize((s,s),Image.LANCZOS).convert('RGB')
    c=Image.new('RGB',(s*4,s))
    for i,(im,t) in enumerate(zip(ims+[m],['Duty (STK2 source)','Freedom','Freedom bright (L x1.5)','recolour mask'])): c.paste(label(im,t),(s*i,0))
    jpg(c,f'flat_{n}.jpg')
# zoomed crops (texel coordinates), nearest-neighbour x3
crops=[('patch','dolg_exo_torso',650,215,830,365),('patch_n','dolg_exo_torso_nmap',650,215,830,365),
       ('star','dol_sign',0,0,512,512),('star_n','dol_sign _nmap',0,0,512,512),
       ('plate','dolg_exo_torso',1620,0,2048,190),('round_plate','dolg_exo_torso',880,1320,1190,1730),
       ('stripes','dolg_exo_legs',40,280,1000,480),('glove','dolg_exo_torso',1100,430,1740,880),
       ('hose','dolg_exo_helmet',1800,980,2048,2048),('canisters','dold_exo_attach',340,1380,720,1660),
       ('crotch_dirt','dolg_exo_legs',0,1280,1000,2048)]
for name,n,x0,y0,x1,y1 in crops:
    w,h=x1-x0,y1-y0; sc=max(1,min(3,900//max(w,h))) if max(w,h)<900 else 900/max(w,h)
    ims=[]
    for d,t in [(src,'Duty')]+[(build/v/'textures',v.replace('freedom','Freedom')) for v in V]:
        a=rd(d,n)[y0:y1,x0:x1,:3]; im=Image.fromarray(a); im=im.resize((int(w*sc),int(h*sc)),Image.NEAREST if sc>=1 else Image.LANCZOS); ims.append(label(im,t))
    c=Image.new('RGB',(sum(i.width for i in ims)+20,ims[0].height),(20,22,24)); x=0
    for im in ims: c.paste(im,(x,0)); x+=im.width+10
    jpg(c,f'crop_{name}.jpg')
# reference + target swatches
ref=Image.open(sys.argv[5]).convert('RGB'); jpg(ref,'reference_samples.jpg')
sw=Image.new('RGB',(600,120),(30,33,36)); d=ImageDraw.Draw(sw)
for i,(c,t) in enumerate([((160,92,38),'median of 10 region medians #A05C26'),((222,135,46),'lit panels ~#DE872E'),((68,35,11),'Freedom result, torso paint median'),((164,93,49),'sign rim result')]):
    d.rectangle((10+i*148,10,148+i*148,80),fill=c); d.text((10+i*148,88),t[:24],fill=(230,230,230)); d.text((10+i*148,102),t[24:],fill=(230,230,230))
sw.save(out/'swatches.png')
P=Image.open(sys.argv[6]).convert('RGBA'); bg=Image.new('RGBA',P.size,(60,63,66,255)); bg.alpha_composite(P); jpg(bg,'emblem_source.jpg')
