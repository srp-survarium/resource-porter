#!/usr/bin/env python3
"""Audit settled native battery boxes and propose unobstructed gallery cameras.

Uses exact triangle/box transforms. Non-box primitives are conservatively boxed
for camera rays and reported as unverified for physical overlap. No DB writes.
"""
import argparse
import importlib.util
import itertools
import json
import math
from pathlib import Path
import re
import struct

HALF = [.27, .108, .144]
FACES = [(0,1,3,2),(4,6,7,5),(0,4,5,1),(2,3,7,6),(0,2,6,4),(1,5,7,3)]

def sub(a,b): return [x-y for x,y in zip(a,b)]
def dot(a,b): return sum(x*y for x,y in zip(a,b))
def cross(a,b): return [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
def unit(v):
    n=math.sqrt(dot(v,v)); return [x/n for x in v] if n else [0.,0.,0.]
def close(bounds,p,radius): return all(bounds[0][i]<=p[i]+radius and bounds[1][i]>=p[i]-radius for i in range(3))
def bounds(vs): return [[min(p[i] for p in vs) for i in range(3)],[max(p[i] for p in vs) for i in range(3)]]

def segment_triangle(start,end,vs):
    """Return segment fraction, including either face of the triangle."""
    a,b,c=vs; direction=sub(end,start); e1=sub(b,a);e2=sub(c,a);h=cross(direction,e2);d=dot(e1,h)
    if abs(d)<1e-9:return None
    q=sub(start,a);u=dot(q,h)/d
    if u<0 or u>1:return None
    r=cross(q,e1);v=dot(direction,r)/d
    if v<0 or u+v>1:return None
    t=dot(e2,r)/d
    return t if 1e-5<t<1-1e-5 else None

def triangle_obb_penetration(vs,center,basis,half=HALF):
    """SAT intersection; minimum translation depth along its separating axes."""
    local=[[dot(sub(v,center),axis) for axis in basis] for v in vs]
    edges=[sub(local[(i+1)%3],local[i]) for i in range(3)]
    axes=[[1,0,0],[0,1,0],[0,0,1],cross(edges[0],edges[1])]
    axes += [cross(e,a) for e in edges for a in [[1,0,0],[0,1,0],[0,0,1]]]
    depth=math.inf
    for axis in axes:
        if dot(axis,axis)<1e-14:continue
        axis=unit(axis);values=[dot(v,axis) for v in local];r=sum(abs(axis[i])*half[i] for i in range(3))
        overlap=min(r-min(values),max(values)+r)
        if overlap<0:return None
        depth=min(depth,overlap)
    return depth

class Geometry:
    def __init__(self,root,project,points,helper):
        self.triangles=[];self.boxes=[];self.missing=[];self.unsupported=[]
        for oi,obj in enumerate(project['collision_objects']):
            path,*suffix=obj['lib_name'].removeprefix('resources/').split('#')
            scale=[float(x) for x in re.findall(r'\[([^]]+)\]',suffix[0])] if suffix else [1,1,1]
            rotation=helper.rotation(obj['rotation']);world=lambda p:helper.transform(p,rotation,obj['position'],scale)
            directory=root/path;vp=directory/'vertices';ip=directory/'indices';pp=directory/'exported_primitives'
            desc={'object':oi,'resource':path,'cgroup':obj['cgroup'],'cmask':obj['cmask']}
            if not vp.exists() and not pp.exists():self.missing.append(path)
            if vp.exists() and ip.exists():
                vs=[world(p) for p in struct.iter_unpack('<3f',helper.chunks(vp.read_bytes())[25])]
                for ti,ix in enumerate(struct.iter_unpack('<3I',helper.chunks(ip.read_bytes())[26])):
                    self.add_triangle([vs[j] for j in ix],dict(desc,kind='triangle',index=ti),points)
            if not pp.exists():continue
            for pi,p in enumerate(helper.config(pp.read_bytes())['primitives']):
                typ=p['type'];dim=p['scale'];pr=helper.rotation(p['rotation'])
                if typ==0:half=[dim[0]]*3
                elif typ==1:half=dim
                elif typ==2:half=[dim[0],dim[1],dim[0]]
                elif typ==3:half=[dim[0],dim[0]+dim[1]/2,dim[0]]
                else:raise ValueError(f'unknown primitive {typ}')
                vs=[world(helper.transform(v,pr,p['position'])) for v in itertools.product(*[(-h,h) for h in half])]
                bound=bounds(vs)
                if not any(close(bound,p,4) for p in points):continue
                meta=dict(desc,kind='box' if typ==1 else 'conservative_primitive_box',index=pi,primitive_type=typ)
                center=[sum(v[i] for v in vs)/8 for i in range(3)]
                edges=[sub(vs[i],vs[0]) for i in [4,2,1]]
                axes=[unit(e) for e in edges];actual_half=[math.sqrt(dot(e,e))/2 for e in edges]
                self.boxes.append({'center':center,'basis':axes,'half':actual_half,'bounds':bound,'description':meta})
                if typ!=1:self.unsupported.append(self.boxes[-1])
                for a,b,c,d in FACES:
                    self.add_triangle([vs[a],vs[b],vs[c]],meta,points)
                    self.add_triangle([vs[a],vs[c],vs[d]],meta,points)
    def add_triangle(self,vs,desc,points):
        b=bounds(vs)
        if any(close(b,p,4) for p in points):self.triangles.append({'vertices':vs,'bounds':b,'description':desc})
    def local(self,p,radius=4):return [t for t in self.triangles if close(t['bounds'],p,radius)]
    def cameras(self,center):
        local=self.local(center);views=[]
        for number,choices in enumerate([[(1.7,1.),(1.7,.65),(1.5,.4)],[(3.,1.6),(3.,1.),(2.4,.65),(2.,.4)]]):
            candidates=[]
            for radius,height in choices:
              for i in range(16):
                angle=i*math.tau/16
                if views and abs((angle-views[0]['azimuth_radians']+math.pi)%math.tau-math.pi)<math.pi/3:continue
                eye=[center[0]+math.sin(angle)*radius,center[1]+height,center[2]+math.cos(angle)*radius]
                hits=[segment_triangle(eye,center,t['vertices']) for t in local]
                if any(t is not None for t in hits):continue
                # Reject an eye inside a collision box, even if its ray exits a hole.
                if any(all(abs(dot(sub(eye,b['center']),axis))<h for axis,h in zip(b['basis'],b['half'])) for b in self.boxes):continue
                candidates.append({'view':number+1,'camera':eye,'target':center,'forward':unit(sub(center,eye)),
                                   'fov_degrees':65,'azimuth_radians':angle,'collision_segment_clear':True})
              if candidates:break
            if candidates:views.append(candidates[0])
        return views
    def footprint(self,center,basis):
        intersections=[];contained=[];overhead=[];unsupported=[]
        for t in self.local(center,.5):
            depth=triangle_obb_penetration(t['vertices'],center,basis)
            if depth is not None:intersections.append(dict(t['description'],penetration_metres=depth))
            if t['description']['kind']=='triangle':
                # An upward-facing surface immediately over the body centre is
                # evidence of burial even if the body no longer cuts that face.
                v=t['vertices'];normal=unit(cross(sub(v[1],v[0]),sub(v[2],v[0])))
                end=[center[0],center[1]+.5,center[2]];f=segment_triangle(center,end,v)
                if f is not None and normal[1]>.25:overhead.append(dict(t['description'],height_above_body_center=f*.5))
        for b in self.boxes:
            if not close(b['bounds'],center,.5):continue
            if b['description']['kind']!='box':unsupported.append(b['description']);continue
            if all(abs(dot(sub(center,b['center']),axis))<h for axis,h in zip(b['basis'],b['half'])):contained.append(b['description'])
        deep=[h for h in intersections if h['penetration_metres']>.02]
        return {'body_half_extents':HALF,'contact_tolerance_metres':.02,
                'intersecting_surfaces':intersections,'deep_intersections':deep,
                'body_center_inside_boxes':contained,'upward_mesh_surfaces_above_body':overhead,
                'nearby_unverified_primitives':unsupported,
                'status':'needs_review' if deep or contained or overhead or unsupported else 'no_burial_detected'}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('geometry_root',type=Path);ap.add_argument('project',type=Path);ap.add_argument('manifest',type=Path)
    ap.add_argument('--map',default='chemical-011-day');ap.add_argument('--output',type=Path,required=True)
    ap.add_argument('--geometry-helper',type=Path,default=Path(__file__).resolve().parents[1]/'chemical-plant/audit_collision_region.py')
    a=ap.parse_args();spec=importlib.util.spec_from_file_location('collision_geometry',a.geometry_helper);helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
    manifest=json.loads(a.manifest.read_text());spawns=next(m for m in manifest['maps'] if m['key']==a.map)['spawns']
    selected=[s for s in spawns if s.get('runtime_validated') and s.get('runtime_body_basis')]
    assert selected,'No stable checkpoints with exact body bases'
    geometry=Geometry(a.geometry_root,helper.config(a.project.read_bytes()),[s['runtime_position'] for s in selected],helper)
    rows=[]
    for spawn in selected:
        center=spawn['runtime_position'];basis=spawn['runtime_body_basis'];assert all(abs(dot(r,r)-1)<.002 for r in basis),'Invalid native rotation'
        check=geometry.footprint(center,basis);poses=geometry.cameras(center)
        rows.append({'index':spawn['index'],'body_center':center,'body_basis':basis,
                     'model_origin':[center[j]-.12*basis[1][j] for j in range(3)],
                     'collision_check':check,'capture_poses':poses})
    result={'map':a.map,'missing_resources':geometry.missing,'local_triangles':len(geometry.triangles),'spawns':rows,
            'scope':'Actual native box against extracted triangles and oriented boxes; conservative 2cm contact tolerance. Unsupported primitives and upward surfaces over body require review. Camera segments are collision-clear, not guaranteed visually clear.'}
    a.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'checked':len(rows),'needs_review':[r['index'] for r in rows if r['collision_check']['status']!='no_burial_detected'],
                      'two_clear_views':[r['index'] for r in rows if len(r['capture_poses'])==2],'missing_resources':len(geometry.missing)}))

if __name__=='__main__':main()
