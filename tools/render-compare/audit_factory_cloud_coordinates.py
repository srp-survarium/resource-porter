#!/usr/bin/env python3
"""Replay fixed source/native Factory rain cloud-coordinate hypotheses."""
import argparse,pathlib,json,math,struct,hashlib
from audit_local_light_authored import constants
from audit_full_diffuse_composition import unpack
from compare_weather import bmp
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('original',type=pathlib.Path);parser.add_argument('target',type=pathlib.Path)
parser.add_argument('--archive',type=pathlib.Path,required=True);parser.add_argument('--index',type=pathlib.Path,required=True)
parser.add_argument('--pose',default='streetlamp-user-area.bmp');parser.add_argument('--output',type=pathlib.Path,required=True)
args=parser.parse_args();old,new,pose=args.original,args.target,args.pose
idx=json.loads(args.index.read_text());offset,length=idx['textures/sky/cgskies_cloud.dds']
with args.archive.open('rb') as f:f.seek(offset);dds=f.read(length)
if dds[84:88]!=b'DXT5':raise ValueError('source cloud texture format changed')
w,h=struct.unpack_from('<II',dds,12)[::-1]
for folder in [old,new]:
 for suffix in ['cloud-before','cloud-output']:
  view=json.loads((folder/(pose+'.'+suffix+'.targets.json')).read_text())[0]
  if (view['width'],view['height'],view['format'],view['written'])!=(1280,720,26,True):raise ValueError('cloud output contract changed')
tex=dds[128:];assert tex[:w*h]==(old/(pose+'.cloud-draw.rt00.bin')).read_bytes()==(new/(pose+'.cloud-draw.rt01.bin')).read_bytes()
cache={}
def pixel(m,x,y):
 W,H=w>>m,h>>m;x%=W;y%=H;k=(m,x//4,y//4)
 if k not in cache:
  off=sum((w>>j)*(h>>j) for j in range(m))+(y//4*(W//4)+x//4)*16;b=tex[off:off+16];a0,a1=b[:2];aa=[a0,a1]+([((7-i)*a0+i*a1)/7 for i in range(1,7)] if a0>a1 else [((5-i)*a0+i*a1)/5 for i in range(1,5)]+[0,255])
  def col(z):r,g,b=z>>11,(z>>5)&63,z&31;return [(r<<3)|(r>>2),(g<<2)|(g>>4),(b<<3)|(b>>2)]
  c0,c1=[col(v) for v in struct.unpack_from('<HH',b,8)];cc=[c0,c1,[(2*a+b)/3 for a,b in zip(c0,c1)],[(a+2*b)/3 for a,b in zip(c0,c1)]]
  def linear(v):v=v/255;return v/12.92 if v<=.04045 else ((v+.055)/1.055)**2.4
  cc=[[linear(v) for v in c] for c in cc];cache[k]=(cc,aa,int.from_bytes(b[2:8],'little'),int.from_bytes(b[12:16],'little'))
 cc,aa,ai,ci=cache[k];i=y%4*4+x%4;return cc[ci>>(2*i)&3]+[aa[ai>>(3*i)&7]/255]
def sample(m,u,v):
 W,H=w>>m,h>>m;u=u%1*W-.5;v=v%1*H-.5;x,y=math.floor(u),math.floor(v);fx,fy=u-x,v-y;r=[0.]*4
 for dx in (0,1):
  for dy in (0,1):
   weight=(fx if dx else 1-fx)*(fy if dy else 1-fy)
   for c,z in enumerate(pixel(m,x+dx,y+dy)):r[c]+=weight*z
 return r
_,cb=constants(old/(pose+'.cloud-draw.vs.dxbc'));M=cb['m_VP'];mat=[list(M[i*4:i*4+4])+[int(i==j) for j in range(4)] for i in range(4)]
for c in range(4):
 k=max(range(c,4),key=lambda i:abs(mat[i][c]));mat[c],mat[k]=mat[k],mat[c];q=mat[c][c];mat[c]=[x/q for x in mat[c]]
 for i in range(4):
  if i!=c:q=mat[i][c];mat[i]=[a-q*b for a,b in zip(mat[i],mat[c])]
inv=[r[4:] for r in mat]
def unproject(x,y,z):
 q=[sum(a*b for a,b in zip(r,[x,y,z,1])) for r in inv];return [v/q[3] for v in q[:3]]
depth=[(p/(pose+s)).read_bytes() for p,s in [(old,'.composition-draw.rt00.bin'),(new,'.composition-draw.rt03.bin')]]
points=[]
for y in range(8,544,16):
 for x in range(8,1272,16):
  if x<224 and y<224:continue
  i=y*1280+x
  if any(struct.unpack_from('<e',d,i*2)[0]!=499.75 for d in depth):continue
  xx,yy=2*(x+.5)/1280-1,1-2*(y+.5)/720;near,far=unproject(xx,yy,0),unproject(xx,yy,.9);v=[b-a for a,b in zip(near,far)];l=math.sqrt(sum(z*z for z in v));v=[z/l for z in v];points.append((x,y,math.atan2(v[2],v[0])/(2*math.pi),math.acos(v[1])/math.pi))
if not points:raise ValueError('pose has no common clear-sky samples')
source_output=(old/(pose+'.cloud-output.rt00.bin')).read_bytes()
report={'scope':'Fixed shader-derived coordinate hypotheses; original and native HDR before/output. Sphere-direction interpolation and BC3 filtering are approximate, not an exact raster replay. No empirical phase search or clock fit.', 'samples':len(points),'texture_sha256':hashlib.sha256(tex[:w*h]).hexdigest(),'source_rotation':cb['sky_clouds_parameters2'],'hypotheses':{}}
for label,folder in [('source',old),('native',new)]:
 before=(folder/(pose+'.cloud-before.rt00.bin')).read_bytes();after=(folder/(pose+'.cloud-output.rt00.bin')).read_bytes();_,ps=constants(folder/(pose+'.cloud-draw.dxbc'));tint=ps['sky_clouds_parameters0'][0]*(1 if label=='source' else 10);results=[]
 for sign in [1,-1]:
  for phase in [0,.25,.5,.75]:
   for scale in [1,.96]:
    error=0;source_error=0;n=0
    for x,y,u,v in points:
     v*=scale
     if v>=.5:continue
     samples=[sample(m,sign*u+phase,v*2) for m in range(3)];avg=[sum(s[c] for s in samples)/3 for c in range(4)];fade=(1-max(0,min(1,(1-v*2)/.5)))**2;alpha=avg[3]*(1-fade);factor=[(1-alpha)*(1-tint*z)+tint*z for z in avg[:3]]
     i=y*1280+x;b=unpack(struct.unpack_from('<I',before,i*4)[0]);a=unpack(struct.unpack_from('<I',after,i*4)[0]);error+=sum(abs(b[c]*factor[c]-a[c]) for c in range(3));ref=unpack(struct.unpack_from('<I',source_output,i*4)[0]);source_error+=sum(abs(b[c]*factor[c]-ref[c]) for c in range(3));n+=1
    results.append(dict(sign=sign,phase=phase,v_scale=scale,samples=n,rgb_mae=error/(3*n),predicted_source_output_mae=source_error/(3*n)))
 report['hypotheses'][label]=sorted(results,key=lambda v:v['rgb_mae'])
op,np=bmp(old/pose),bmp(new/pose);report['sky_bmp_rgb_mae']=sum(abs(a-b) for x,y,_,_ in points for a,b in zip(op(x,y),np(x,y)))/(len(points)*3)
args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'samples':len(points),'best':{k:v[0] for k,v in report['hypotheses'].items()},'sky_bmp_rgb_mae':report['sky_bmp_rgb_mae']}))
