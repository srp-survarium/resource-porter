"""audit_respawns.World with a uniform grid index over the parts' bounds: near() looks only at the cells a
box touches (the same answers, far fewer bounds tests); exported_primitives configs are read and parsed once."""
import collections, functools, math
import audit_respawns
from audit_respawns import World, inv
from audit_collision_region import intersects, rotation

if not hasattr(audit_respawns, '_raw_config'):
    audit_respawns._raw_config = audit_respawns.config
    _cfg = {}

    def _cached(data):
        k = hash(data)
        if k not in _cfg: _cfg[k] = audit_respawns._raw_config(data)
        return _cfg[k]
    audit_respawns.config = _cached


class FastWorld(World):
    CELL = 2.0

    def __init__(self, root, project=None, parts=None):
        if parts is None:
            super().__init__(root, project)
        else:
            self.parts = parts; self.root = root
        self.grid = collections.defaultdict(list)
        for k, part in enumerate(self.parts):
            (x0, _, z0), (x1, _, z1) = part[2]['bounds'][0], part[2]['bounds'][1]
            for i in range(math.floor(x0 / self.CELL), math.floor(x1 / self.CELL) + 1):
                for j in range(math.floor(z0 / self.CELL), math.floor(z1 / self.CELL) + 1):
                    self.grid[(i, j)].append(k)

    def primitive_top(self, obj, part, x, z, lo, hi, step=0.01):
        """World.primitive_top's answer (the highest h in [lo, hi] on a 0.01 m grid from hi down where the
        vertical line is inside the primitive), searched at 0.05 m first, then 0.01 m within that step."""
        h = hi
        while h >= lo:
            if self.primitive_distance(obj, part, x, h, z) <= 0.0:
                g = min(hi, h + 0.05)
                while g > h:
                    if self.primitive_distance(obj, part, x, g, z) <= 0.0: return g
                    g -= step
                return h
            h -= 0.05
        return None

    def near(self, lo, hi):
        seen, out = set(), []
        for i in range(math.floor(lo[0] / self.CELL), math.floor(hi[0] / self.CELL) + 1):
            for j in range(math.floor(lo[2] / self.CELL), math.floor(hi[2] / self.CELL) + 1):
                for k in self.grid.get((i, j), ()):
                    if k in seen: continue
                    seen.add(k)
                    n, o, p = self.parts[k]
                    if intersects(p['bounds'], [lo, hi]): out.append((n, o, p))
        return out

    def primitive_distance(self, obj, part, x, y, z):
        """World.primitive_distance with each model's primitives read once (World reads the file per query)."""
        q = _primitives(self.root / obj['lib_name'].removeprefix('resources/').split('#')[0] / 'exported_primitives')[part['index']]
        d = q['scale']
        o = inv([x - obj['position'][0], y - obj['position'][1], z - obj['position'][2]], _rotation(tuple(obj['rotation'])))
        l = inv([o[i] - q['position'][i] for i in range(3)], _rotation(tuple(q['rotation'])))
        if q['type'] == 0: return max(math.sqrt(sum(c*c for c in l)) - d[0], 0)
        if q['type'] == 2: return math.hypot(max(math.hypot(l[0], l[2]) - d[0], 0), max(abs(l[1]) - d[1], 0))
        if q['type'] == 3:
            cy = max(-d[1]/2, min(d[1]/2, l[1])); return max(math.sqrt(l[0]**2 + (l[1]-cy)**2 + l[2]**2) - d[0], 0)
        return math.sqrt(sum(max(abs(l[i]) - d[i], 0)**2 for i in range(3)))

    def primitive_hit(self, obj, part, x, f, z, r):
        """World.primitive_hit (capsule samples against the primitive) on the cached primitives."""
        return any(self.primitive_distance(obj, part, x, f + 0.25 + r + (1.8 - 0.25 - 2*r) * k / 30, z) < r for k in range(31))


@functools.lru_cache(maxsize=None)
def _primitives(path):
    return audit_respawns.config(path.read_bytes())['primitives']


@functools.lru_cache(maxsize=None)
def _rotation(angles):
    return rotation(list(angles))
