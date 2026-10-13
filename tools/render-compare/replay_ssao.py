from pathlib import Path
import json
import math
import os
import argparse
import statistics
import struct
import subprocess
import sys

parser=argparse.ArgumentParser(description='Replay archived Factory-rain SSAO on fixed 1280x720 captures; temporal history is omitted.')
parser.add_argument('capture',type=Path)
parser.add_argument('assets',type=Path,help='Original SSAO shaders, noise and fullscreen-vs.dxbc')
parser.add_argument('output',type=Path)
parser.add_argument('--runtime',type=Path,required=True,help='replay_ssao.exe with matching d3d11.dll and dxgi.dll')
parser.add_argument('--live',action='store_true',help='Verify live archived-pass readbacks against an independent replay on the exact captured depth')
parser.add_argument('--wine',required=True)
parser.add_argument('--prefix',required=True)
args=parser.parse_args()
tmp=args.assets.resolve();source=Path(__file__).resolve().parent
capture=args.capture.resolve();out=args.output.resolve();runtime=args.runtime.resolve()
out.mkdir(parents=True,exist_ok=True)
(out/'flat-depth.r16f').write_bytes(struct.pack('<e',10)*1280*720)
def win(p):return 'Z:'+str(p).replace('/',chr(92))
env=dict(os.environ,WINEPREFIX=args.prefix,WINEDEBUG='-all',DISPLAY=':0',WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json',DXVK_LOG_LEVEL='warn')
env.pop('VK_DRIVER_FILES',None)
cmd=[args.wine,win(runtime/'replay_ssao.exe'),'1280','720',win(tmp/'fullscreen-vs.dxbc'),win(tmp/'old-ssao-3.dxbc'),win(tmp/'old-ssao-upsample.dxbc')]
sys.path.insert(0,str(source))
from audit_surface_light import REGIONS
from inspect_buffers import png

def stats(values):
    return {'minimum':min(values),'maximum':max(values),'mean':statistics.fmean(values),'median':statistics.median(values)}

depth_suffix='.ssao-live.rt00.bin' if args.live else '.probe-draw.rt02.bin'
jobs={'flat':out/'flat-depth.r16f',**{p:capture/(p+'.bmp'+depth_suffix) for p in REGIONS}}
result={}
for pose,depth in jobs.items():
    subprocess.run(cmd+[win(depth),win(tmp/'old-ssao-noise.bgra'),win(out/pose)],env=env,cwd=runtime,check=True)
    half=list(struct.iter_unpack('<ee',(out/(pose+'.half.rg16f')).read_bytes()))
    full=(out/(pose+'.full.rg8')).read_bytes()
    assert len(half)==640*360 and len(full)==1280*720*2
    assert all(math.isfinite(a) and math.isfinite(z) for a,z in half)
    depth_bytes=depth.read_bytes()
    for i,(a,z) in enumerate(half):
        # Half-size pixel centers land on full-size point-sampler boundaries.
        # Interpolation rounding may select either adjacent texel on each axis.
        x,y=2*(i%640),2*(i//640)
        adjacent=[struct.unpack_from('<e',depth_bytes,((y+dy)*1280+x+dx)*2)[0]
                  for dy in (0,1) for dx in (0,1)]
        assert z in adjacent, 'depth sampling footprint or UV orientation differs'
        assert 0<=a<=.5, 'archived SSAO output is out of range'
    raw=full[::2]; assert not any(full[1::2])
    if pose=='flat':
        a=1-2**(-.5*1.442652)
        expected=min(min((2.5*a)**(max(a,.4)*2*2.5),2.5)*.4,.5)
        assert len(set(half))==1 and abs(half[0][0]-expected)<.00025 and half[0][1]==10
        assert set(raw)=={round(half[0][0]*255)}
        result[pose]={'expected_unquantized':expected,'observed_half':half[0][0],'observed_full':raw[0]/255,'passed':True}
        continue
    old=[x*2.5/255 for x in raw]
    native_suffix='.ssao-native.rt00.bin' if args.live else '.hba-draw.rt00.bin'
    native=[x/255 for x in (capture/(pose+'.bmp'+native_suffix)).read_bytes()]
    assert len(native)==len(old)
    png(out/(pose+'.png'),1280,720,b''.join(bytes([min(round(x*255),255)])*3 for x in old))
    result[pose]={'depth_sampling_footprint_verified':True,'old_normalized':stats(old),'target':stats(native),'regions':{}}
    if args.live:
        assert (out/(pose+'.half.rg16f')).read_bytes()==(capture/(pose+'.bmp.ssao-live.rt01.bin')).read_bytes(), 'live accumulation differs from offline replay'
        assert full==(capture/(pose+'.bmp.ssao-live.rt02.bin')).read_bytes(), 'live upsample differs from offline replay'
        result[pose]['live_half_and_full_byte_identical']=True
    for name,(x0,y0,x1,y1) in REGIONS[pose].items():
        indices=[y*1280+x for y in range(y0,y1) for x in range(x0,x1)]
        o=[old[i] for i in indices];n=[native[i] for i in indices]
        result[pose]['regions'][name]={'old_normalized':stats(o),'target':stats(n),'mean_ratio':statistics.fmean(o)/statistics.fmean(n)}
(out/'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v.get('regions',v) for k,v in result.items()},indent=2))
