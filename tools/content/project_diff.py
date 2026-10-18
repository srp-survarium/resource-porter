#!/usr/bin/env python3
"""project_diff.py OLD_PACK NEW_PACK PROJECT...: what changed in each decoded client_project, by object.

Array items are matched by full_name (client and server objects) or by their whole value (visuals,
collision objects); a changed item is listed with its changed fields. Prints JSON and exits 1 when a change
is neither an ammo crate object (logic/fill/teamN/resupply/*, the item/quest/container_closed instances) nor
a spawn object (server_objects.respawn_points, their respawn_colision zones)."""
import json, sys
from pathlib import Path
import os
sys.path.insert(0, os.environ.get('TOOLS', '/mnt/hdd/Projects/release-20/crates-021d/tools-a0b5933/tools') + '/chemical-plant')
from audit_collision_region import config

def key(o): return o.get('full_name') if isinstance(o, dict) and 'full_name' in o else json.dumps(o, sort_keys=True)
def crate(o): return isinstance(o, dict) and ('container_closed' in str(o.get('lib_name', '')) or str(o.get('full_name', '')).startswith('logic/fill/team'))
def spawn(path, o): return 'respawn_points' in path or (isinstance(o, dict) and 'respawn_colision' in str(o.get('full_name', '')))

def walk(path, a, b, out):
    if isinstance(a, list) and isinstance(b, list):
        ka = {}; kb = {}
        for o in a: ka.setdefault(key(o), []).append(o)
        for o in b: kb.setdefault(key(o), []).append(o)
        for k in sorted(set(ka) | set(kb), key=str):
            xa, xb = ka.get(k, []), kb.get(k, [])
            if len(xa) == len(xb) == 1 and xa[0] != xb[0] and isinstance(xa[0], dict):
                fields = sorted(f for f in set(xa[0]) | set(xb[0]) if xa[0].get(f) != xb[0].get(f))
                out.append(dict(path=path, change='changed', name=k[:120], fields={f: [xa[0].get(f), xb[0].get(f)] for f in fields}, ok=crate(xb[0]) or spawn(path, xb[0])))
            elif xa != xb:
                for o in xa[len(xb):] if len(xa) > len(xb) else []: out.append(dict(path=path, change='removed', name=k[:120], ok=crate(o) or spawn(path, o)))
                for o in xb[len(xa):] if len(xb) > len(xa) else []: out.append(dict(path=path, change='added', name=(o.get('full_name') or o.get('lib_name')) if isinstance(o, dict) else k[:120],
                                                                                     position=o.get('position') if isinstance(o, dict) else None, rotation=o.get('rotation') if isinstance(o, dict) else None,
                                                                                     ok=crate(o) or spawn(path, o)))
    elif isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            if a.get(k) != b.get(k): walk(f'{path}.{k}' if path else k, a.get(k), b.get(k), out)
    elif a != b:
        out.append(dict(path=path, change='value', old=a, new=b, ok=False))

bad = 0; report = {}
old, new = Path(sys.argv[1]), Path(sys.argv[2])
for p in sys.argv[3:]:
    out = []; walk('', config((old / 'files' / p).read_bytes()), config((new / 'files' / p).read_bytes()), out)
    report[p] = out; bad += sum(not c['ok'] for c in out)
print(json.dumps(report, indent=1, default=str))
sys.exit(1 if bad else 0)
