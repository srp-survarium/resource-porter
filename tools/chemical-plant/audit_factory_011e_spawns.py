#!/usr/bin/env python3
"""Check the explicit 0.11a Factory gameplay fallback against the 0.11e bundle."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys
from audit_collision_region import config, rotation, transform
import audit_collision_region as helper
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'day-map-objectives'))
from audit_battery_geometry import Geometry, segment_triangle


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('bundle', type=Path)
    p.add_argument('--source-client', type=Path, required=True)
    p.add_argument('--server-fallback', type=Path, required=True)
    p.add_argument('--native-battery-options', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    record = json.loads((a.bundle/'port-manifest.json').read_text())
    for f in record['files']:
        data=(a.bundle/'overlay'/f['path']).read_bytes()
        assert len(data)==f['size'] and hashlib.sha256(data).hexdigest()==f['sha256'],f['path']
    assert {'id':195,'level':'level_03','weather':'day'} in record['maps']
    assert record['server_project_fallback']['source_build']=='0.11a'
    fallback_bytes=a.server_fallback.read_bytes()
    assert hashlib.sha256(fallback_bytes).hexdigest()==record['server_project_fallback']['projects']['projects/level_03/server_project']
    source=config(a.source_client.read_bytes());fallback=config(fallback_bytes)['objects']
    assert fallback['respawn_points']==source['respawn_points']
    for group,kind in [('victory_items_containers','victory_items_container'),('collision_geometry','collision_geometry')]:
        assert fallback[group]==[o for o in source['objects'] if o.get('game_object_type')==kind]
    target=config((a.bundle/'overlay/projects/v011e_level_03/day_2/client_project').read_bytes())
    spawns=target['server_objects']['victory_item_spawners'];old=fallback['victory_item_spawners']
    assert len(spawns)==len(old)==21
    offset=config(a.native_battery_options.read_bytes())['collision_geometries'][0]['position']
    centers=[]
    for i,(before,after) in enumerate(zip(old,spawns)):
        assert all(before[k]==after[k] for k in ['position','rotation','point_id','priority','full_name']),i
        expected=transform(offset,rotation(before['rotation']),before['position'])
        assert math.dist(expected,after['spawned_item']['position'])<.00002,i
        centers.append(after['spawned_item']['position'])
    geometry=Geometry(a.bundle/'overlay',target,centers,helper)
    assert not geometry.missing,geometry.missing
    results=[]
    for i,(spawn,center) in enumerate(zip(spawns,centers)):
        basis=rotation(spawn['rotation']);rays=[]
        for x,z in [(0,0),(-.22,-.10),(-.22,.10),(.22,-.10),(.22,.10)]:
            point=transform([x,0,z],basis,center);start=[point[0],point[1]+.15,point[2]];end=[point[0],point[1]-2,point[2]];hits=[]
            for t in geometry.local(point,2.2):
                d=t['description']
                if not d['blocks_battery_physics'] or d['kind'] not in ('triangle','box'):continue
                f=segment_triangle(start,end,t['vertices'])
                if f is not None:
                    y=start[1]+f*(end[1]-start[1]);hits.append(dict(d,surface_y=y,distance_below_body_center=point[1]-y))
            hits.sort(key=lambda h:abs(h['distance_below_body_center']-.108))
            rays.append({'body_footprint_offset':[x,z],'nearest_support':hits[0] if hits else None})
        footprint=geometry.footprint(center,basis)
        results.append({'index':i,'point_id':spawn['point_id'],'name':spawn['full_name'],'authored_position':spawn['position'],'initial_body_position':center,'support_rays':rays,'initial_footprint':footprint})
    output={'scope':'Initial CPU support and box overlap only; no native settle, pickup or walking test. Battery candidates are explicitly from 0.11a, not claimed as original 0.11e server data.',
            'files_hashed':len(record['files']),'missing_collision_resources':geometry.missing,
            'player_respawns_identical':len(source['respawn_points']), 'collectors_identical':2,'zones_identical':47,
            'candidate_count':len(results),'center_support_found':sum(s['support_rays'][0]['nearest_support'] is not None for s in results),
            'center_support_within_20cm':sum(s['support_rays'][0]['nearest_support'] is not None and abs(s['support_rays'][0]['nearest_support']['distance_below_body_center'])<=.2 for s in results),
            'initial_footprint_needs_review':[s['index'] for s in results if s['initial_footprint']['status']=='needs_review'],
            'spawns':results}
    a.output.write_text(json.dumps(output,indent=2)+'\n')
    print(json.dumps({k:v for k,v in output.items() if k!='spawns'}))


if __name__=='__main__':main()
