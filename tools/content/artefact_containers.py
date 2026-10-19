#!/usr/bin/env python3
"""artefact_containers.py ROOT PROJECT NATIVE_PROJECT > report.json

Every artefact container of every generic_anomaly_core of PROJECT (a 0.21d port's client_project): the container
object's position, its collision geometry's position and box centres (meshes, world), the movement-collision
floor under the box centre (ROOT: the merged collision root) within 3 m up / 6 m down, and the core's own zones
(its states' damage zones) for reference; then NATIVE_PROJECT's cores (stock level of the same location) with
their containers, for comparison. Positions in the projects' frame."""
import json, math, os, sys
from pathlib import Path
T = os.environ.get('TOOLS', '/mnt/hdd/Projects/release-20/crates-021d/tools-a0b5933/tools') + '/day-map-objectives'
sys.path.insert(0, T); sys.path.insert(0, T + '/../chemical-plant')
from fast_world import FastWorld
from audit_respawns import config
from audit_collision_region import rotation, transform

def cores(project):
    objs = {o.get('full_name'): o for o in project['client_objects']}
    out = []
    for o in project['client_objects']:
        if o.get('game_object_type') != 'generic_anomaly_core': continue
        conts = []
        for name in o.get('artefact_containers') or []:
            c = objs.get(name)
            if c is None: conts.append(dict(name=name, missing=True)); continue
            geoms = []
            for g in c.get('collision_geometries') or []:
                cg = objs.get(g['name'])
                if cg is None: geoms.append(dict(name=g['name'], missing=True)); continue
                R = rotation(cg.get('rotation', [0, 0, 0]))
                boxes = [transform(m.get('position', [0, 0, 0]), R, cg['position'], cg.get('scale', [1, 1, 1])) for m in cg.get('meshes') or []]
                geoms.append(dict(name=g['name'], position=cg['position'], boxes=boxes,
                                  sizes=[m.get('scale') for m in cg.get('meshes') or []]))
            conts.append(dict(name=name, position=c['position'], geometries=geoms))
        zones = sorted({z for s in o.get('states') or [] for g in s.get('groups') or [] for z in g.get('zones') or []})
        zpos = [objs[z]['position'] for z in zones if z in objs and 'position' in objs[z]]
        out.append(dict(core=o['full_name'], position=o.get('position'), artefacts=o.get('artefacts'),
                        chance=o.get('artefacts_respawn_chance'), respawn=o.get('artefacts_respawn_time_sec'),
                        max=o.get('max_artefacts_count'), enabled=o.get('artefacts_enabled'),
                        zone_centre=[sum(p[i] for p in zpos) / len(zpos) for i in range(3)] if zpos else None, containers=conts))
    return out

root, proj, native = Path(sys.argv[1]), config(Path(sys.argv[2]).read_bytes()), config(Path(sys.argv[3]).read_bytes())
ours = cores(proj)
world = FastWorld(root, proj)
for core in ours:
    for c in core['containers']:
        for g in c.get('geometries', []):
            for b in g.get('boxes', []):
                f = world.floor(b[0], b[1], b[2], below=6.0, above=3.0)
                g.setdefault('floor_under_box', []).append(None if f is None else [round(f[0], 3), f[1]])
print(json.dumps(dict(ours=ours, native=cores(native)), indent=1, default=float))
