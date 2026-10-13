#!/usr/bin/env python3
"""Compare matched weather captures on a fixed image-wide grid, excluding shared HUD areas."""
import argparse
import json
import math
from pathlib import Path
import struct


def bmp(path):
    data=path.read_bytes();offset=struct.unpack_from('<I',data,10)[0]
    width,height=struct.unpack_from('<ii',data,18)
    assert width==1280 and abs(height)==720 and struct.unpack_from('<H',data,28)[0]==32
    def pixel(x,y):
        row=y if height<0 else height-y-1
        at=offset+4*(row*width+x)
        return tuple(data[at+c]/255 for c in (2,1,0))
    return pixel


def compare(original,current,poses):
    output=[]
    for pose in poses:
        old_path=original/(pose['name']+'.bmp')
        if not old_path.exists() and original.name=='day':old_path=original.with_name('day-coverage')/old_path.name
        old,new=bmp(old_path),bmp(current/(pose['name']+'.bmp'))
        sums=[0.,0.];error=0.;mae=0.;count=0;ssim=[];contrast=[]
        # Four-pixel grid, excluding the old minimap and player/HUD lower strip.
        # Block statistics are over 8x8 samples (32x32 full-resolution pixels).
        for by in range(0,544,32):
            for bx in range(0,1280,32):
                if bx<224 and by<224:continue
                a=[];b=[]
                for y in range(by,by+32,4):
                    for x in range(bx,bx+32,4):
                        ar,br=old(x,y),new(x,y)
                        al=sum(c*w for c,w in zip(ar,(.2126,.7152,.0722)))
                        bl=sum(c*w for c,w in zip(br,(.2126,.7152,.0722)))
                        a.append(al);b.append(bl);sums[0]+=al;sums[1]+=bl
                        error+=sum((c-d)**2 for c,d in zip(ar,br));mae+=abs(al-bl);count+=1
                am=sum(a)/64;bm=sum(b)/64
                av=sum((v-am)**2 for v in a)/64;bv=sum((v-bm)**2 for v in b)/64
                covariance=sum((c-am)*(d-bm) for c,d in zip(a,b))/64
                ssim.append(((2*am*bm+.01**2)*(2*covariance+.03**2))/((am*am+bm*bm+.01**2)*(av+bv+.03**2)))
                contrast.append((math.sqrt(av),math.sqrt(bv)))
        output.append({'pose':pose['name'],'split':pose.get('split'),'samples':count,
                       'original_luma_mean':sums[0]/count,'target_luma_mean':sums[1]/count,
                       'display_rgb_rmse':math.sqrt(error/count/3),'display_luma_mae':mae/count,
                       'block_ssim_mean':sum(ssim)/len(ssim),
                       'original_block_contrast':sum(p[0] for p in contrast)/len(contrast),
                       'target_block_contrast':sum(p[1] for p in contrast)/len(contrast)})
    return {'scope':'Fixed four-pixel grid across image; excludes x<224,y<224 and y>=544 for original HUD/player. Display-encoded RGB/luma, 32-pixel block SSIM, separate animated frames. These metrics are not a proof of visual parity.','views':output}

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('original',type=Path);p.add_argument('current',type=Path);p.add_argument('--poses',type=Path,default=Path(__file__).with_name('poses-day-evening-expanded.json'));a=p.parse_args()
    print(json.dumps(compare(a.original,a.current,json.loads(a.poses.read_text())),indent=2))
