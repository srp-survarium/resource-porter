# SPDX-License-Identifier: GPL-3.0-or-later
"""Minimal PSD layer extractor (8-bit RGB, raw/RLE). Usage: psdlayers.py file.psd outdir [--list]"""
import sys, struct, json, numpy as np
from pathlib import Path
from PIL import Image
def packbits(buf, n):
    out=bytearray(); i=0
    while len(out)<n:
        h=buf[i]; i+=1
        if h<128: out+=buf[i:i+h+1]; i+=h+1
        elif h>128: out+=bytes([buf[i]])*(257-h); i+=1
    return bytes(out[:n])
def main():
    path=Path(sys.argv[1]); out=Path(sys.argv[2]); listonly='--list' in sys.argv
    d=path.read_bytes(); o=0
    sig,ver,_,ch,h,w,depth,mode=struct.unpack('>4sH6sHIIHH',d[:26]); o=26
    assert sig==b'8BPS' and ver==1 and depth==8, (sig,ver,depth)
    n,=struct.unpack('>I',d[o:o+4]); o+=4+n
    n,=struct.unpack('>I',d[o:o+4]); o+=4+n
    lmlen,=struct.unpack('>I',d[o:o+4]); o+=4
    lilen,=struct.unpack('>I',d[o:o+4]); o+=4
    cnt,=struct.unpack('>h',d[o:o+2]); o+=2; cnt=abs(cnt)
    layers=[]
    for _ in range(cnt):
        t,l,b,r,nc=struct.unpack('>iiiiH',d[o:o+18]); o+=18
        chans=[struct.unpack('>hI',d[o+6*k:o+6*k+6]) for k in range(nc)]; o+=6*nc
        bs,bk,op,clip,flags,_,el=struct.unpack('>4s4sBBBBI',d[o:o+16]); o+=16
        e=o; o+=el
        ml,=struct.unpack('>I',d[e:e+4]); mask=None
        if ml>=16: mask=struct.unpack('>iiii',d[e+4:e+20])
        e+=4+ml
        bl,=struct.unpack('>I',d[e:e+4]); e+=4+bl
        nl=d[e]; name=d[e+1:e+1+nl].decode('latin1'); e+=((nl+1+3)//4)*4
        while e+12<=o:
            s,k,ln=struct.unpack('>4s4sI',d[e:e+12])
            if k==b'luni':
                cl,=struct.unpack('>I',d[e+12:e+16]); name=d[e+16:e+16+2*cl].decode('utf-16-be')
            e+=12+ln+(ln&1 and 0)
        layers.append(dict(name=name,rect=[t,l,b,r],chans=chans,blend=bk.decode(),opacity=op,clip=clip,hidden=bool(flags&2),mask=mask))
    out.mkdir(parents=True,exist_ok=True)
    for i,L in enumerate(layers):
        t,l,b,r=L['rect']; H,W=b-t,r-l; planes={}
        for cid,ln in L['chans']:
            comp,=struct.unpack('>H',d[o:o+2]); data=d[o+2:o+ln]; o+=ln
            if cid==-2 and L['mask']:
                mt,ml_,mb,mr=L['mask']; hh,ww=mb-mt,mr-ml_
            else: hh,ww=H,W
            if listonly or hh<=0 or ww<=0: continue
            if comp==0: px=np.frombuffer(data[:hh*ww],np.uint8).reshape(hh,ww)
            elif comp==1:
                counts=struct.unpack(f'>{hh}H',data[:2*hh]); p=2*hh; rows=[]
                for c in counts: rows.append(packbits(data[p:p+c],ww)); p+=c
                px=np.frombuffer(b''.join(rows),np.uint8).reshape(hh,ww)
            else: px=None
            planes[cid]=px
        L['index']=i
        if not listonly and H>0 and W>0 and all(k in planes for k in (0,1,2)):
            rgba=np.zeros((h,w,4),np.uint8)
            a=planes.get(-1, np.full((H,W),255,np.uint8))
            y0,x0=max(t,0),max(l,0); y1,x1=min(b,h),min(r,w)
            for c in range(3): rgba[y0:y1,x0:x1,c]=planes[c][y0-t:y1-t,x0-l:x1-l]
            rgba[y0:y1,x0:x1,3]=a[y0-t:y1-t,x0-l:x1-l]
            if -2 in planes and L['mask']:
                mt,ml_,mb,mr=L['mask']; m=np.zeros((h,w),np.uint8)
                # default colour outside mask rect ignored (assume 0/255 by rect)
                yy0,xx0=max(mt,0),max(ml_,0); yy1,xx1=min(mb,h),min(mr,w)
                m[yy0:yy1,xx0:xx1]=planes[-2][yy0-mt:yy1-mt,xx0-ml_:xx1-ml_]
                np.save(out/f'{i:02d}_mask.npy',m)
            Image.fromarray(rgba).save(out/f'{i:02d}.png')
    (out/'layers.json').write_text(json.dumps([{k:v for k,v in L.items() if k!='chans'} for L in layers],indent=1))
    for L in layers: print(L.get('index','?'),repr(L['name']),L['rect'],L['blend'],L['opacity'],'hidden' if L['hidden'] else '', 'clip' if L['clip'] else '', 'mask' if L['mask'] else '')
main()
