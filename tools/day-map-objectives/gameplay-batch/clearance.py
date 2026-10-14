"""Conservative floor/clearance check for diagnostic standing and battery points."""
import sys, json, math
sys.path.insert(0, '/tmp/resource-porter-render-compare/tools/chemical-plant')
from audit_collision_region import config, collision_parts, intersects
from pathlib import Path
root = Path(sys.argv[1]); project = config(Path(sys.argv[2]).read_bytes())
centre = json.loads(sys.argv[3]); floor_y = float(sys.argv[4]); radius = float(sys.argv[5])
region = [[centre[0]-radius-1, floor_y-1, centre[2]-radius-1], [centre[0]+radius+1, floor_y+2.5, centre[2]+radius+1]]
parts = []
for obj in project['collision_objects']:
    if not (obj['cgroup'] & 2 and obj['cmask'] & 4): continue   # walker collision only
    try:
        for p in collision_parts(root, obj):
            if intersects(p['bounds'], region): parts.append((obj['lib_name'].split('/models/')[-1], p))
    except FileNotFoundError: pass
def blocked(x, z, half=0.35):
    box = [[x-half, floor_y+0.25, z-half], [x+half, floor_y+1.8, z+half]]
    return [n for n, p in parts if intersects(p['bounds'], box)]
def floor(x, z):
    hs = [p['bounds'][1][1] for n, p in parts if p['bounds'][0][0] <= x <= p['bounds'][1][0] and p['bounds'][0][2] <= z <= p['bounds'][1][2] and abs(p['bounds'][1][1]-floor_y) < 0.3]
    return max(hs) if hs else None
for k in range(16):
    a = 2*math.pi*k/16
    for d in (1.6, 2.4, 3.2):
        x, z = centre[0]+d*math.sin(a), centre[2]+d*math.cos(a)
        b = blocked(x, z)
        print(f'dir {k:2d} d {d}: [{x:.3f},{z:.3f}] floor {floor(x, z)} blockers {sorted(set(b))[:3]}')
if len(sys.argv) > 6:
    for x, z in json.loads(sys.argv[6]):
        print('POINT', [x, z], 'floor', floor(x, z), 'blockers', sorted(set(blocked(x, z, 0.3))))
