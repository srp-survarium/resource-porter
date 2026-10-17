"""audit_respawns.World with a uniform grid index over the parts' bounds: near() looks only at the cells a
box touches (the same answers, far fewer bounds tests); exported_primitives configs are parsed once."""
import collections, math
import audit_respawns
from audit_respawns import World
from audit_collision_region import intersects

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
