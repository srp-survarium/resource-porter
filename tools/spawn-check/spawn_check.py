#!/usr/bin/env python3
"""Spawn points of 0.26g map variants against the collision the game actually builds (offline, no game).

The owner's two rules, and the zones:
  1. inside: the player's standing capsule at the point is free of static movement collision;
  2. ground: a ray down from the capsule's base meets collision within GROUND_REACH, the point sits on
     that surface, and rays round it inside the capsule's radius meet collision too (no hole under it);
  3. zones: every zone a point names exists, and its ally zone holds the point.

What the game builds (0.10b sources, vostok/sources/vostok, and the 0.26g data):
- the player: gameplay/players/default.player physics_controller.stand_capsule_width / _height (0.76 m,
  1.8 m in 0.26g), a Bullet capsule standing on the feet (shape offset half its height); its filter
  group is 1<<2 and mask 1<<1 (physics character_controller.cpp), so it meets exactly the static
  objects whose cgroup has 0x2 (walkable) and whose cmask has 0x4; cgroup 8 / cmask 16 ("hittable"
  only: bullets) never touch it;
- static collision: each project `collision_objects[]` entry (project_cooker_simple.cpp): the model's
  collision folder `exported_primitives` (a compound of boxes, spheres, cylinders, capsules) plus its
  triangle mesh (`vertices` chunk 0x19, `indices` chunk 0x1a), both if both exist
  (collision_shape_cook.cpp); the shape's scale is only the lib_name `#[sx][sy][sz]` suffix (the
  object's `scale` goes into a matrix that from_vostok() turns into a quaternion: dropped); placed by
  rotation (math::create_rotation, row vectors) and position;
- zones: client objects `collision_geometry` (collision_geometry.cpp): `meshes[]` primitives (box half
  extents = scale) under the object's rotation and position.
A model folder is read from the pack's files, else from the newest of the base archives (the patches
given, then the stock database) that has it, as the game mounts them. A folder in none: no collision,
as in the game (reported).

The tests (exact up to the sampling noted):
- inside: a capsule of the player's radius + MARGIN along the vertical from feet + STEP_BAND to
  feet + height + MARGIN; the lowest STEP_BAND (0.25 m) is left out: the controller steps over it
  (engine step height 0.6 m) and it is where the feet meet the floor (slopes up to ~50 deg pass).
  Point-to-triangle and point-to-primitive distances at 1 cm steps along the axis (1-Lipschitz, so a
  hit is taken at distance < radius + 0.5 cm);
- ground: vertical rays from feet + 0.5 m: the centre must meet collision within GROUND_REACH below
  the feet; the surface it meets must be within SINK below .. FLOAT above the feet; 16 more rays (8
  directions at 0.5 and 0.95 of the radius) must each meet collision within RING_REACH below the feet.
  SINK 0.10 m and FLOAT 0.25 m are what 0.26g's own maps ship and play with: native level_03's barracks
  points stand 0.05-0.09 m into their floor box, native level_04's team-2 points 0.19 m over theirs,
  native level_06 one 0.07 m under a mesh floor (audit of the stock projects with this tool);
- zones: each named ally/enemy zone exists; the point (at feet + 0.9 m) is inside one of its ally
  zone's primitives.

Usage:
  spawn_check.py audit CONFIG.json OUT_DIR [--fail]       audit; --fail: exit 1 if any point fails or a
                                                           team has fewer than MIN_GOOD passing points
  spawn_check.py propose CONFIG.json AUDIT_DIR PLAN.json  edit-built-pack ops for the failing points
  spawn_check.py render CONFIG.json AUDIT_DIR OUT_DIR [--plan PLAN.json]   overhead PNGs
CONFIG.json: {"porter": vostok-port, "cache": DIR, "base": [newest archive, ..., stock resources.db],
              "pack": PACK_DIR, "variants": [{"label": "206", "project": "projects/.../client_project",
              "name": "Chemical Plant v0.11 batteries"}, ...]}
"""
import argparse, collections, csv, hashlib, json, math, os, re, struct, subprocess, sys
from pathlib import Path

import numpy as np

STEP_BAND = 0.25
MARGIN = 0.02
GROUND_REACH = 0.5
RING_REACH = 0.6
SINK = 0.10
FLOAT = 0.25
MIN_GOOD = 10


# --- binary config (read only; writes go through edit-built-pack) -------------------------------
def config(b, off=0):
    data, _, _, kind, count = struct.unpack_from('<QQIHH', b, off)
    if kind == 0: return bool(data)
    if kind == 1: return struct.unpack_from('<i', b, off)[0]
    if kind == 2: return struct.unpack_from('<f', b, off)[0]
    if kind == 5: return b[data:data + count - 1].decode('cp1251')
    if kind in (6, 7, 8): return list(struct.unpack_from('<' + 'f' * (kind - 4), b, data))
    if kind == 4: return [config(b, data + i * 24) for i in range(count)]
    if kind != 3: raise ValueError(f'unknown config kind {kind}')
    out = {}
    for i in range(count):
        child = data + i * 24
        p = struct.unpack_from('<Q', b, child + 8)[0]
        out[b[p:b.index(b'\0', p)].decode('cp1251')] = config(b, child)
    return out


def rotation(a):
    """math::create_rotation(float3 angles): rows i, j, k; a row vector v maps to v @ R."""
    sx, sy, sz = map(math.sin, a); cx, cy, cz = map(math.cos, a)
    return np.array([[cy * cz, -cy * sz, sy],
                     [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy],
                     [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy]])


def yaw_of(rot):
    return rot[1] if abs(rot[0]) < 1 else math.pi - rot[1]


def chunks(b):
    out = {}; off = 0
    while off < len(b):
        kind, size = struct.unpack_from('<II', b, off); off += 8
        if off + size > len(b): raise ValueError('truncated geometry chunk')
        out[kind] = b[off:off + size]; off += size
    return out


# --- where the files come from --------------------------------------------------------------------
class Sources:
    """The pack's files over the base archives (newest first), as the game mounts them."""

    def __init__(self, cfg):
        self.porter = cfg['porter']; self.cache = Path(cfg['cache']); self.cache.mkdir(parents=True, exist_ok=True)
        self.files = Path(cfg['pack']) / 'files'
        self.layers = []
        for db in cfg['base']:
            st = os.stat(db)
            tag = hashlib.sha1(f'{os.path.realpath(db)}:{st.st_size}:{int(st.st_mtime)}'.encode()).hexdigest()[:12]
            listing = self.cache / f'{tag}.list'
            if not listing.exists():
                out = subprocess.run([self.porter, 'list', '--db', db], check=True, capture_output=True, text=True).stdout
                listing.write_text(out)
            names = set(listing.read_text().split('\n')) - {''}
            folders = {n.rsplit('/', 1)[0] for n in names}
            self.layers.append(dict(db=db, tag=tag, names=names, folders=folders))

    def where(self, folder):
        if (self.files / folder).is_dir(): return ('pack', self.files / folder)
        for layer in self.layers:
            if folder in layer['folders']: return (layer['tag'], self.cache / layer['tag'] / 'overlay' / folder)
        return (None, None)

    def fetch(self, folders):
        """Extract every base folder in `folders` not cached yet, one extract-bundle per archive."""
        want = collections.defaultdict(list)
        for f in folders:
            tag, path = self.where(f)
            if tag not in (None, 'pack') and not path.is_dir(): want[tag].append(f)
        for layer in self.layers:
            todo = want.get(layer['tag'])
            if not todo: continue
            for i in range(0, len(todo), 300):
                tmp = self.cache / f"tmp-{layer['tag']}-{os.getpid()}"
                args = [self.porter, 'extract-bundle', '--db', layer['db'], '--output', str(tmp)]
                for f in todo[i:i + 300]: args += ['--prefix', f + '/']
                subprocess.run(args, check=True, capture_output=True)
                for p in (tmp / 'overlay').rglob('*'):
                    if p.is_file():
                        t = self.cache / layer['tag'] / 'overlay' / p.relative_to(tmp / 'overlay')
                        t.parent.mkdir(parents=True, exist_ok=True); p.replace(t)
                subprocess.run(['rm', '-rf', str(tmp)], check=True)

    def project(self, path):
        f = self.files / path
        if f.exists(): return f.read_bytes(), 'pack'
        for layer in self.layers:
            if path in layer['names']:
                out = self.cache / layer['tag'] / 'projects' / path.replace('/', '__')
                if not out.exists():
                    out.parent.mkdir(parents=True, exist_ok=True)
                    subprocess.run([self.porter, 'extract', '--db', layer['db'], '--path', path, '--output', str(out)], check=True, capture_output=True)
                return out.read_bytes(), layer['db']
        raise FileNotFoundError(path)

    def player(self):
        for layer in self.layers:
            path = 'gameplay/players/default.player'
            if (self.files / path).exists(): return config((self.files / path).read_bytes()), 'pack'
            if path in layer['names']:
                out = self.cache / layer['tag'] / 'default.player'
                if not out.exists():
                    subprocess.run([self.porter, 'extract', '--db', layer['db'], '--path', path, '--output', str(out)], check=True, capture_output=True)
                return config(out.read_bytes()), layer['db']
        raise FileNotFoundError('gameplay/players/default.player')


# --- the collision world ----------------------------------------------------------------------------
PRIM_NAMES = {0: 'sphere', 1: 'box', 2: 'cylinder', 3: 'capsule'}


def prim_distance(prim, pts):
    """Distance from world points (N,3) to a primitive (0 inside)."""
    l = ((pts - prim['t']) @ prim['R'].T - prim['c']) @ prim['Rc'].T
    d = prim['dim']; k = prim['type']
    if k == 0: return np.maximum(np.linalg.norm(l, axis=1) - d[0], 0)
    if k == 1: return np.linalg.norm(np.maximum(np.abs(l) - d, 0), axis=1)
    if k == 2:
        dr = np.maximum(np.hypot(l[:, 0], l[:, 2]) - d[0], 0); dy = np.maximum(np.abs(l[:, 1]) - d[1], 0)
        return np.hypot(dr, dy)
    if k == 3:
        cy = np.clip(l[:, 1], -d[1] / 2, d[1] / 2)
        return np.maximum(np.sqrt(l[:, 0] ** 2 + (l[:, 1] - cy) ** 2 + l[:, 2] ** 2) - d[0], 0)
    raise ValueError(f'primitive type {k}')


def point_triangle_distance(p, a, b, c):
    """Distances from points p (P,3) to triangles a, b, c (T,3): (P,T). Ericson's closest point."""
    P = p[:, None, :]; A = a[None]; B = b[None]; C = c[None]
    ab = B - A; ac = C - A; ap = P - A
    d1 = (ab * ap).sum(-1); d2 = (ac * ap).sum(-1)
    bp = P - B; d3 = (ab * bp).sum(-1); d4 = (ac * bp).sum(-1)
    cp = P - C; d5 = (ab * cp).sum(-1); d6 = (ac * cp).sum(-1)
    va = d3 * d6 - d5 * d4; vb = d5 * d2 - d1 * d6; vc = d1 * d4 - d3 * d2
    denom = va + vb + vc
    with np.errstate(divide='ignore', invalid='ignore'):
        v = vb / denom; w = vc / denom
        q = A + ab * v[..., None] + ac * w[..., None]                                    # inside
        t_ab = d1 / (d1 - d3); t_ac = d2 / (d2 - d6); t_bc = (d4 - d3) / ((d4 - d3) + (d5 - d6))
    regions = [
        ((d1 <= 0) & (d2 <= 0), A + 0 * P),
        ((d3 >= 0) & (d4 <= d3), B + 0 * P),
        ((d6 >= 0) & (d5 <= d6), C + 0 * P),
        ((vc <= 0) & (d1 >= 0) & (d3 <= 0), A + ab * np.nan_to_num(t_ab)[..., None]),
        ((vb <= 0) & (d2 >= 0) & (d6 <= 0), A + ac * np.nan_to_num(t_ac)[..., None]),
        ((va <= 0) & ((d4 - d3) >= 0) & ((d5 - d6) >= 0), B + (C - B) * np.nan_to_num(t_bc)[..., None]),
    ]
    out = np.where(np.isfinite(q), q, 0)
    done = np.zeros(d1.shape, bool)
    for mask, qq in regions:
        m = mask & ~done
        out = np.where(m[..., None], qq, out); done |= m
    return np.linalg.norm(P - out, axis=-1)


class World:
    def __init__(self, project, sources):
        self.notes = []; self.owners = []; prims = []; tris = []; tri_owner = []
        objs = [o for o in project['collision_objects'] if o['cgroup'] & 2 and o['cmask'] & 4]
        self.skipped = len(project['collision_objects']) - len(objs)
        folders = {}
        for o in objs:
            path, *suffix = o['lib_name'].removeprefix('resources/').split('#')
            folders[id(o)] = (path, suffix)
        sources.fetch({p for p, _ in folders.values()})
        missing = collections.Counter()
        for o in objs:
            path, suffix = folders[id(o)]
            scale = np.array([float(x) for x in re.findall(r'\[([^]]+)\]', suffix[0])] if suffix else [1.0, 1.0, 1.0])
            if scale.shape != (3,): raise ValueError(f'scale suffix of {o["lib_name"]}')
            tag, d = sources.where(path)
            if tag is None: missing[path] += 1; continue
            R = rotation(o['rotation']); t = np.array(o['position'], float)
            k = len(self.owners); self.owners.append(dict(model=path.removeprefix('models/'), source=tag, position=o['position']))
            ep = d / 'exported_primitives'
            if ep.exists():
                for i, p in enumerate(config(ep.read_bytes())['primitives']):
                    kind = p['type']
                    if not np.allclose(scale, scale[0]) and any(abs(r) > 1e-4 for r in p['rotation']):
                        self.notes.append(f'{path}: primitive {i} turned in a non-uniformly scaled compound (scale taken on its own axes, as Bullet does)')
                    if kind not in PRIM_NAMES: raise ValueError(f'{path}: primitive type {kind}')
                    dim = np.array(p['scale'], float) * scale
                    half = {0: [dim[0]] * 3, 1: dim, 2: [dim[0], dim[1], dim[0]], 3: [dim[0], dim[0] + dim[1] / 2, dim[0]]}[kind]
                    Rc = rotation(p['rotation']); c = np.array(p['position'], float) * scale
                    corners = np.array([[sx, sy, sz] for sx in (-1, 1) for sy in (-1, 1) for sz in (-1, 1)]) * np.array(half)
                    w = ((corners @ Rc) + c) @ R + t
                    prims.append(dict(type=kind, dim=dim, Rc=Rc, c=c, R=R, t=t, owner=k, index=i, lo=w.min(0), hi=w.max(0)))
            vf, xf = d / 'vertices', d / 'indices'
            if vf.exists() and xf.exists():
                v = np.frombuffer(chunks(vf.read_bytes())[0x19], '<f4').reshape(-1, 3).astype(float)
                ix = np.frombuffer(chunks(xf.read_bytes())[0x1a], '<u4').reshape(-1, 3)
                w = (v * scale) @ R + t
                tris.append(w[ix]); tri_owner.append(np.full(len(ix), k))
        for path, n in missing.items(): self.notes.append(f'{path}: no collision folder in the pack or the base ({n} objects; the game has none either)')
        self.tris = np.concatenate(tris) if tris else np.zeros((0, 3, 3))
        self.tri_owner = np.concatenate(tri_owner) if tri_owner else np.zeros(0, int)
        self.tlo = self.tris.min(1); self.thi = self.tris.max(1)
        self.prims = prims
        self.plo = np.array([p['lo'] for p in prims]).reshape(-1, 3); self.phi = np.array([p['hi'] for p in prims]).reshape(-1, 3)
        self.objects = len(objs)

    def near(self, lo, hi):
        lo = np.asarray(lo); hi = np.asarray(hi)
        ti = np.nonzero(np.all(self.tlo <= hi, 1) & np.all(self.thi >= lo, 1))[0]
        pi = np.nonzero(np.all(self.plo <= hi, 1) & np.all(self.phi >= lo, 1))[0] if len(self.prims) else []
        return ti, [self.prims[i] for i in pi]

    def name(self, owner, prim=None):
        o = self.owners[owner]
        return o['model'] + (f" primitive {prim['index']} ({PRIM_NAMES[prim['type']]})" if prim else '')

    def capsule_hits(self, x, feet, z, radius, height):
        """[(name, depth m, at height above the feet)] the test capsule overlaps."""
        R = radius + MARGIN; y0 = feet + STEP_BAND + R; y1 = feet + height + MARGIN - R
        ys = np.arange(y0, y1 + 1e-9, 0.01)
        pts = np.stack([np.full_like(ys, x), ys, np.full_like(ys, z)], 1)
        ti, ps = self.near([x - R, y0 - R, z - R], [x + R, y1 + R, z + R])
        hits = {}
        if len(ti):
            for s in range(0, len(ti), 4000):
                sel = ti[s:s + 4000]; T = self.tris[sel]
                d = point_triangle_distance(pts, T[:, 0], T[:, 1], T[:, 2])
                for col in np.nonzero(d.min(0) < R + 0.005)[0]:
                    n = self.name(self.tri_owner[sel[col]]); row = d[:, col].argmin(); depth = R - d[row, col]
                    if n not in hits or depth > hits[n][0]: hits[n] = (depth, ys[row] - feet)
        for p in ps:
            d = prim_distance(p, pts)
            if d.min() < R + 0.005:
                n = self.name(p['owner'], p); row = d.argmin(); depth = R - d[row]
                if n not in hits or depth > hits[n][0]: hits[n] = (depth, ys[row] - feet)
        return sorted(((n, round(float(a), 3), round(float(b), 2)) for n, (a, b) in hits.items()), key=lambda h: -h[1])

    def ray_down(self, x, z, top, bottom):
        """The first collision a vertical ray from `top` down to `bottom` at (x, z) meets: (height, name)."""
        best = None
        ti, ps = self.near([x - 1e-4, bottom, z - 1e-4], [x + 1e-4, top, z + 1e-4])
        if len(ti):
            T = self.tris[ti]
            x1, y1, z1 = T[:, 0].T; x2, y2, z2 = T[:, 1].T; x3, y3, z3 = T[:, 2].T
            dd = (z2 - z3) * (x1 - x3) + (x3 - x2) * (z1 - z3)
            ok = np.abs(dd) > 1e-12
            with np.errstate(divide='ignore', invalid='ignore'):
                a = ((z2 - z3) * (x - x3) + (x3 - x2) * (z - z3)) / dd
                b = ((z3 - z1) * (x - x3) + (x1 - x3) * (z - z3)) / dd
                c = 1 - a - b
                ok &= (a >= -1e-7) & (b >= -1e-7) & (c >= -1e-7)
                h = a * y1 + b * y2 + c * y3
                ok &= (h <= top) & (h >= bottom)
            if ok.any():
                j = np.nonzero(ok)[0][np.argmax(h[ok])]
                best = (float(h[j]), self.name(self.tri_owner[ti[j]]))
        for p in ps:
            ys = np.arange(top, bottom - 1e-9, -0.005)
            pts = np.stack([np.full_like(ys, x), ys, np.full_like(ys, z)], 1)
            inside = np.nonzero(prim_distance(p, pts) <= 0)[0]
            if len(inside) and (best is None or ys[inside[0]] > best[0]):
                best = (float(ys[inside[0]]), self.name(p['owner'], p))
        return best


# --- the checks -----------------------------------------------------------------------------------------
def zone_index(project):
    out = {}
    for o in project.get('client_objects', []):
        if o.get('game_object_type') == 'collision_geometry': out[o['full_name']] = o
    return out


def in_zone(zone, p):
    R = rotation(zone['rotation']); t = np.array(zone['position'], float)
    for m in zone.get('meshes', []):
        l = ((np.array(p) - t) @ R.T - np.array(m['position'])) @ rotation(m['rotation']).T
        d = np.array(m['scale'])
        k = m['type']
        if k == 1 and np.all(np.abs(l) <= d): return True
        if k == 0 and np.linalg.norm(l) <= d[0]: return True
        if k == 2 and math.hypot(l[0], l[2]) <= d[0] and abs(l[1]) <= d[1]: return True
        if k == 3 and math.sqrt(l[0] ** 2 + (l[1] - max(-d[1] / 2, min(d[1] / 2, l[1]))) ** 2 + l[2] ** 2) <= d[0]: return True
    return False


def zones_of(sp, kind):
    return [z['name'] for z in (sp.get(kind) or {}).get('collision_geometries', [])]


def check_point(world, cap, x, y, z, zones=None, sp=None):
    """{'inside': [...], 'ground': [...], 'zones': [...], 'floor': h} problems per check (empty: pass)."""
    radius, height = cap
    out = dict(inside=[], ground=[], zones=[], floor=None, floor_by=None)
    c = world.ray_down(x, z, y + 0.5, y - GROUND_REACH)
    if c is None:
        deep = world.ray_down(x, z, y + 0.5, y - 60)
        out['ground'].append('no collision within %.1f m below%s' % (GROUND_REACH, f' (next {y - deep[0]:.2f} m down, {deep[1]})' if deep else ' (none at all: out of the map)'))
    else:
        h, by = c; out['floor'] = round(h, 3); out['floor_by'] = by
        if h - y > SINK: out['ground'].append(f'{h - y:.2f} m below its surface ({by})')
        elif y - h > FLOAT: out['ground'].append(f'floats {y - h:.2f} m over its surface ({by})')
    feet = y if c is None else max(y, c[0]) if c[0] - y > SINK else y
    holes = []
    for frac in (0.5, 0.95):
        for k in range(8):
            a = 2 * math.pi * k / 8; dx, dz = math.cos(a) * radius * frac, math.sin(a) * radius * frac
            if world.ray_down(x + dx, z + dz, y + 0.5, y - RING_REACH) is None: holes.append(f'({dx:+.2f},{dz:+.2f})')
    if holes: out['ground'].append(f'no collision within {RING_REACH} m under the capsule at ' + ' '.join(holes))
    for n, depth, at in world.capsule_hits(x, feet, z, radius, height):
        out['inside'].append(f'{n}: {depth:.2f} m deep at {at:.2f} m')
    if zones is not None and sp is not None:
        for kind in ('ally_zones', 'enemy_zones'):
            for n in zones_of(sp, kind):
                if n not in zones: out['zones'].append(f'{kind} {n} does not exist')
        ally = [zones[n] for n in zones_of(sp, 'ally_zones') if n in zones]
        if ally and not any(in_zone(zn, [x, y + 0.9, z]) for zn in ally):
            out['zones'].append('outside its ally zone ' + ', '.join(zn['full_name'] for zn in ally))
    return out


def side_of(points, team, p):
    cen = {}
    for q in points: cen.setdefault(q['team'], []).append(q['position'])
    cen = {t: np.mean(v, 0) for t, v in cen.items()}
    return min(cen, key=lambda t: np.linalg.norm(np.array(p) - cen[t])) == team


def load(cfg, sources, v):
    data, src = sources.project(v['project'])
    project = config(data)
    return project, src


def audit_variant(cfg, sources, cap, v):
    project, src = load(cfg, sources, v)
    world = World(project, sources)
    zones = zone_index(project)
    rows = []
    for i, sp in enumerate(project['server_objects']['respawn_points']):
        x, y, z = sp['position']
        r = check_point(world, cap, x, y, z, zones, sp)
        status = 'pass' if not (r['inside'] or r['ground'] or r['zones']) else 'FAIL'
        rows.append(dict(index=i, team=sp['team'], name=sp['full_name'], point_id=sp.get('point_id'), priority=sp.get('priority'),
                         position=[round(c, 3) for c in sp['position']], rotation=sp['rotation'], status=status, **r))
    teams = {}
    for t in sorted({r['team'] for r in rows}):
        mine = [r for r in rows if r['team'] == t]
        teams[t] = dict(points=len(mine), good=sum(r['status'] == 'pass' for r in mine))
    return dict(label=v['label'], name=v.get('name', ''), project=v['project'], project_from=src, collision_objects=world.objects,
                ignored_not_walkable=world.skipped, triangles=int(len(world.tris)), primitives=len(world.prims), notes=world.notes,
                capsule=dict(radius=cap[0], height=cap[1]), teams=teams, points=rows)


def capsule(sources):
    pl, src = sources.player()
    pc = pl['player']['physics_controller']
    return (pc['stand_capsule_width'] / 2, pc['stand_capsule_height']), src


def write_tables(out, results):
    with open(out / 'audit.csv', 'w', newline='') as f:
        w = csv.writer(f)
        w.writerow(['variant', 'name', 'team', 'point', 'x', 'y', 'z', 'status', 'inside', 'ground', 'zones', 'floor'])
        for r in results:
            for p in r['points']:
                w.writerow([r['label'], r['name'], p['team'], p['name'], *p['position'], p['status'], ' | '.join(p['inside']), ' | '.join(p['ground']),
                            ' | '.join(p['zones']), p['floor']])
    md = ['| Variant | Team 0 pass / points | Team 1 pass / points | Failing points |', '|---|---|---|---|']
    for r in results:
        bad = [f"team {p['team']} `{p['name']}` {p['position']}: " + '; '.join(p['inside'] + p['ground'] + p['zones']) for p in r['points'] if p['status'] != 'pass']
        md.append(f"| {r['label']} {r['name']} | {r['teams'].get(0, {}).get('good')} / {r['teams'].get(0, {}).get('points')} | "
                  f"{r['teams'].get(1, {}).get('good')} / {r['teams'].get(1, {}).get('points')} | {'<br>'.join(bad) or 'none'} |")
    (out / 'summary.md').write_text('\n'.join(md) + '\n')
    lines = []
    for r in results:
        lines.append(f"\n## {r['label']} {r['name']}\n\n`{r['project']}` ({r['collision_objects']} movement collision objects: {r['triangles']} triangles, "
                     f"{r['primitives']} primitives; {r['ignored_not_walkable']} not walkable, ignored)\n")
        for n in r['notes']: lines.append(f'- note: {n}')
        lines.append('\n| Team | Point | Position | Result | Inside collision | Ground | Zones |\n|---|---|---|---|---|---|---|')
        for p in sorted(r['points'], key=lambda p: (p['team'], p['name'])):
            lines.append(f"| {p['team']} | `{p['name']}` | {p['position']} | {p['status']} | {'; '.join(p['inside']) or 'clear'} | "
                         f"{'; '.join(p['ground']) or 'on ' + str(p['floor_by']) + ' at ' + str(p['floor'])} | {'; '.join(p['zones']) or 'ok'} |")
    (out / 'points.md').write_text('\n'.join(lines) + '\n')


def cmd_audit(a):
    cfg = json.loads(Path(a.config).read_text()); out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    sources = Sources(cfg); cap, cap_src = capsule(sources)
    print(f'player capsule: radius {cap[0]:.3f} m, height {cap[1]:.3f} m (default.player from {cap_src})', flush=True)
    results = []
    for v in cfg['variants']:
        if a.only and v['label'] not in a.only: continue
        r = audit_variant(cfg, sources, cap, v); results.append(r)
        (out / f"{v['label']}.json").write_text(json.dumps(r, indent=1) + '\n')
        print(v['label'], v.get('name', ''), json.dumps(r['teams']), flush=True)
        for n in r['notes']: print('   note:', n)
        for p in r['points']:
            if p['status'] != 'pass': print('   FAIL', p['team'], p['name'], p['position'], '; '.join(p['inside'] + p['ground'] + p['zones']), flush=True)
    write_tables(out, results)
    bad = [(r['label'], p['name']) for r in results for p in r['points'] if p['status'] != 'pass']
    short = [(r['label'], t, c['good']) for r in results for t, c in r['teams'].items() if c['good'] < MIN_GOOD]
    print(json.dumps(dict(variants=len(results), points=sum(len(r['points']) for r in results), failing=len(bad), teams_short=short)))
    if a.fail and (bad or short):
        print(f'SPAWN CHECK FAILED: {len(bad)} failing points, teams under {MIN_GOOD}: {short}', file=sys.stderr); sys.exit(1)


# --- proposals ---------------------------------------------------------------------------------------------------
def zone_centre_offset(zones, points):
    """The usual height of an ally zone's origin over its point's feet, from the zones that hold theirs."""
    offs = []
    for sp in points:
        for n in zones_of(sp, 'ally_zones'):
            z = zones.get(n)
            if z and in_zone(z, [sp['position'][0], sp['position'][1] + 0.9, sp['position'][2]]): offs.append(z['position'][1] - sp['position'][1])
    return float(np.median(offs)) if offs else 0.0


def cmd_propose(a):
    """For each failing point: on its own surface if that passes, else the nearest passing spot (5 cm rings
    out to --search) on its team's side, 0.9 m clear of every other point; its own ally zone follows it (or is
    centred on it when it did not hold it). A variant with `same_as` first takes that variant's ops (the TDM
    projects are their battery projects with less collision, so the same points move the same way), then gets
    its own for any failing point those ops do not cover (e.g. a crate only the TDM project has); the audit
    after checks every point of every variant again."""
    cfg = json.loads(Path(a.config).read_text()); sources = Sources(cfg); cap, _ = capsule(sources)
    plan = dict(projects={}); report = []
    for v in cfg['variants']:
        inherited = []
        if v.get('same_as'):
            src = next(w for w in cfg['variants'] if w['label'] == v['same_as'])
            inherited = list(plan['projects'].get(src['project'], []))
            report.append(dict(variant=v['label'], same_as=v['same_as'], ops=len(inherited)))
        covered = {o['full_name'] for o in inherited if o['op'] in ('respawn_move', 'respawn_zone')}
        r = json.loads((Path(a.audit) / f"{v['label']}.json").read_text())
        covered |= {p['name'] for p in r['points'] for o in inherited if o['op'] == 'zone_move' and (p['name'] + ' ') in (o['why'] + ' ')}
        failing = [p for p in r['points'] if p['status'] != 'pass' and p['name'] not in covered]
        if not failing:
            if inherited: plan['projects'][v['project']] = inherited
            continue
        project, _ = load(cfg, sources, v); world = World(project, sources); zones = zone_index(project)
        points = project['server_objects']['respawn_points']
        zoff = zone_centre_offset(zones, points)
        ops = list(inherited)
        use = collections.Counter(n for sp in points for k in ('ally_zones', 'enemy_zones') for n in zones_of(sp, k))
        placed = {sp['full_name']: list(sp['position']) for sp in points}
        for o in inherited:
            if o['op'] == 'respawn_move': placed[o['full_name']] = list(o['position'])
        for fp in failing:
            sp = next(q for q in points if q['full_name'] == fp['name'])
            x, y, z = sp['position']
            own = [n for n in zones_of(sp, 'ally_zones') if n in zones and use[n] == 1]
            shared = [zones[n] for n in zones_of(sp, 'ally_zones') if n in zones and use[n] > 1]
            moved_needed = bool(fp['inside'] or fp['ground'])

            def ok(px, py, pz, snap=True):
                if not side_of(points, sp['team'], [px, py, pz]): return None
                if any(math.dist([px, pz], [q[0], q[2]]) < 0.9 for n, q in placed.items() if n != sp['full_name']): return None
                if snap:
                    c = world.ray_down(px, pz, py + 0.6, py - 0.8)
                    if c is None: return None
                    ny = round(c[0] + 0.02, 3)
                else: ny = py
                res = check_point(world, cap, px, ny, pz)
                if res['inside'] or res['ground']: return None
                if shared and not any(in_zone(zn, [px, ny + 0.9, pz]) for zn in shared): return None
                return ny
            best = None
            if not moved_needed: best = (0.0, [x, y, z])
            else:
                ny = ok(x, y, z)
                if ny is not None: best = (0.0, [round(x, 3), ny, round(z, 3)])
                else:
                    for ring in range(1, int(a.search / 0.05) + 1):
                        rr = ring * 0.05; n = max(8, int(2 * math.pi * rr / 0.05))
                        cands = []
                        for k in range(n):
                            ang = 2 * math.pi * k / n; px, pz = x + rr * math.cos(ang), z + rr * math.sin(ang)
                            ny = ok(px, y, pz)
                            if ny is not None: cands.append((abs(ny - y), [round(px, 3), ny, round(pz, 3)]))
                        if cands: best = (rr, min(cands)[1]); break
            if not best:
                report.append(dict(variant=v['label'], point=sp['full_name'], result=f'no passing spot within {a.search} m')); continue
            dist, pos = best
            if moved_needed:
                placed[sp['full_name']] = pos
                ops.append(dict(op='respawn_move', full_name=sp['full_name'], **{'from': [round(c, 3) for c in sp['position']]}, position=pos,
                                why='; '.join(fp['inside'] + fp['ground']) + (f' -> moved {dist:.2f} m' if dist else f' -> on its surface ({pos[1] - y:+.2f} m)')))
            delta = [pos[i] - sp['position'][i] for i in range(3)]
            # A named ally zone that holds another place, while an unnamed zone holds this point (a
            # reference slip in the source data): name that one instead.
            for n in list(zones_of(sp, 'ally_zones')):
                if n not in zones or in_zone(zones[n], [pos[0], pos[1] + 0.9, pos[2]]): continue
                folder = sp['full_name'].rsplit('/', 1)[0]
                spare = [m for m, zn in zones.items() if use[m] == 0 and m.rsplit('/', 1)[0] == folder and 'respawn_col' in m
                         and in_zone(zn, [pos[0], pos[1] + 0.9, pos[2]])]
                if spare:
                    ops.append(dict(op='respawn_zone', full_name=sp['full_name'], kind='ally_zones', **{'from': n}, to=spare[0],
                                    why=f'{n} holds another place; {spare[0]}, named by no point, holds this one'))
                    use[n] -= 1; use[spare[0]] += 1
                    if n in own: own.remove(n)
                    if any(z['full_name'] == n for z in shared): shared = [z for z in shared if z['full_name'] != n]
            for n in own:
                zn = zones[n]; zp = zn['position']
                held = in_zone(zn, [x, y + 0.9, z])
                if held:
                    if not any(abs(d) > 1e-4 for d in delta): continue
                    to = [round(zp[i] + delta[i], 3) for i in range(3)]; why = f"the ally zone of {sp['full_name']} moves with it"
                else:
                    m = zn['meshes'][0]['position']; R = rotation(zn['rotation'])
                    off = np.array(m) @ R
                    to = [round(pos[0] - off[0], 3), round(pos[1] + zoff, 3), round(pos[2] - off[2], 3)]
                    why = f"centred on {sp['full_name']} (it held {'another place' if math.dist([x, z], [zp[0] + off[0], zp[2] + off[2]]) > 0.1 else 'no point'})"
                ops.append(dict(op='zone_move', full_name=n, **{'from': [round(c, 3) for c in zp]}, position=to, why=why))
            for zn in shared:
                if not in_zone(zn, [pos[0], pos[1] + 0.9, pos[2]]):
                    report.append(dict(variant=v['label'], point=sp['full_name'], result=f"shared ally zone {zn['full_name']} does not hold it; left as is"))
            report.append(dict(variant=v['label'], point=sp['full_name'], moved=round(dist, 3) if moved_needed else None, to=pos if moved_needed else None,
                               zones=[o.get('to', o['full_name']) for o in ops if o['op'] in ('zone_move', 'respawn_zone') and sp['full_name'] in (o['why'] + o['full_name'])]))
        if ops: plan['projects'][v['project']] = ops
    Path(a.plan).write_text(json.dumps(plan, indent=1) + '\n')
    print(json.dumps(report, indent=1))


# --- overhead renders -------------------------------------------------------------------------------------------
def sub_world(world, lo, hi):
    ti, ps = world.near(lo, hi)
    w = World.__new__(World); w.__dict__.update(world.__dict__)
    w.tris = world.tris[ti]; w.tri_owner = world.tri_owner[ti]
    w.tlo = w.tris.min(1) if len(ti) else np.zeros((0, 3)); w.thi = w.tris.max(1) if len(ti) else np.zeros((0, 3))
    w.prims = ps; w.plo = np.array([p['lo'] for p in ps]).reshape(-1, 3); w.phi = np.array([p['hi'] for p in ps]).reshape(-1, 3)
    return w


def blocked_grid(w, xs, zs, feet):
    """Cells (len(zs), len(xs)) where collision is within 2.5 cm of the capsule's tested height band."""
    hs = np.arange(feet + STEP_BAND, feet + 1.82, 0.15)
    X, Z = np.meshgrid(xs, zs)
    out = np.zeros(X.shape, bool)
    for h in hs:
        pts = np.stack([X.ravel(), np.full(X.size, h), Z.ravel()], 1)
        hit = np.zeros(len(pts), bool)
        for s in range(0, len(w.tris), 200):
            T = w.tris[s:s + 200]
            for c in range(0, len(pts), 2000):
                d = point_triangle_distance(pts[c:c + 2000], T[:, 0], T[:, 1], T[:, 2])
                hit[c:c + 2000] |= d.min(1) < 0.025
        for p in w.prims: hit |= prim_distance(p, pts) < 0.025
        out |= hit.reshape(X.shape)
    return out


def cmd_render(a):
    """Per variant one sheet: a panel per team and floor (every point, pass green / fail red, its facing,
    the floor shaded by height, collision 1 m over the floor brown), then a close-up per moved point (the
    collision the capsule's height band meets, brown; the capsule's footprint where it was, thin, and where
    it goes, magenta)."""
    from PIL import Image, ImageDraw
    cfg = json.loads(Path(a.config).read_text()); sources = Sources(cfg); out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    plan = json.loads(Path(a.plan).read_text()) if a.plan else dict(projects={})
    for v in cfg['variants']:
        r = json.loads((Path(a.audit) / f"{v['label']}.json").read_text())
        project, _ = load(cfg, sources, v); world = World(project, sources)
        moves = {o['full_name']: o for o in plan['projects'].get(v['project'], []) if o['op'] == 'respawn_move'}
        pts = r['points']; cap = r['capsule']['radius']
        zops = [o for o in plan['projects'].get(v['project'], []) if o['op'] in ('zone_move', 'respawn_zone')]
        zone_fixed = {p['name'] for p in pts for o in zops if o.get('full_name') == p['name'] or (p['name'] + ' ') in (o['why'] + ' ')}
        panels = []
        for t in sorted({p['team'] for p in pts}):
            mine = sorted((p for p in pts if p['team'] == t), key=lambda p: p['position'][1])
            groups = [[mine[0]]]
            for p in mine[1:]:
                if p['position'][1] - groups[-1][-1]['position'][1] > 2.0: groups.append([p])
                else: groups[-1].append(p)
            for g in groups:
                xs = [p['position'][0] for p in g]; zs = [p['position'][2] for p in g]
                panels.append((t, g, min(xs) - 5, max(xs) + 5, min(zs) - 5, max(zs) + 5))
        imgs = []
        for t, g, x0, x1, z0, z1 in panels:
            px = max(8, min(24, int(700 / max(x1 - x0, z1 - z0))))
            W, H = int((x1 - x0) * px), int((z1 - z0) * px)
            img = Image.new('RGB', (max(W, 520), H + 40), (255, 255, 255)); d = ImageDraw.Draw(img)
            sx = lambda x: (x - x0) * px; sz = lambda z: (z1 - z) * px
            fy = float(np.median([p['position'][1] for p in g]))
            w = sub_world(world, [x0, fy - 8, z0], [x1, fy + 8, z1])
            step = 0.25
            for j in range(int((z1 - z0) / step)):
                for i in range(int((x1 - x0) / step)):
                    x, z = x0 + (i + .5) * step, z0 + (j + .5) * step
                    c = w.ray_down(x, z, fy + 2.2, fy - 4)
                    if c is None: col = (60, 60, 60)
                    elif c[0] > fy + 1.0: col = (110, 84, 70)
                    else:
                        gr = int(235 - max(0, min(1, (c[0] - fy + 2) / 3)) * 80); col = (gr, gr, gr - 6)
                    d.rectangle([sx(x - step / 2), sz(z + step / 2), sx(x + step / 2), sz(z - step / 2)], fill=col)
            for gx in range(math.ceil(x0), int(x1) + 1):
                if gx % 5 == 0: d.line([sx(gx), 0, sx(gx), H], fill=(170, 165, 150)); d.text((sx(gx) + 2, 2), f'x{gx}', fill=(30, 30, 30))
            for gz in range(math.ceil(z0), int(z1) + 1):
                if gz % 5 == 0: d.line([0, sz(gz), W, sz(gz)], fill=(170, 165, 150)); d.text((2, sz(gz) + 2), f'z{gz}', fill=(30, 30, 30))
            for p in g:
                x, _, z = p['position']
                col = (20, 150, 60) if p['status'] == 'pass' else (220, 30, 30)
                rr = cap * px
                d.ellipse([sx(x) - rr, sz(z) - rr, sx(x) + rr, sz(z) + rr], outline=col, width=3)
                yaw = yaw_of(p['rotation'])
                d.line([sx(x), sz(z), sx(x - math.sin(yaw) * 1.0), sz(z + math.cos(yaw) * 1.0)], fill=(30, 90, 210) if p['team'] == 0 else (230, 120, 20), width=2)
                label = re.sub(r'.*\((\d+)\)$', r'\1', p['name']) if p['name'].endswith(')') else '0'
                if p['name'] in moves: label += ' moved'
                elif p['name'] in zone_fixed: label += ' zone fixed'
                d.text((sx(x) + rr + 2, sz(z) - 6), label, fill=(150, 0, 150) if p['name'] in moves or p['name'] in zone_fixed else (0, 0, 0))
            good = sum(p['status'] == 'pass' for p in g)
            d.text((4, H + 3), f"{v['label']} {v.get('name', '')}: team {t}, floor ~y {fy:.1f}: {good}/{len(g)} pass (green pass, red fail; label: point number)", fill=(0, 0, 0))
            d.text((4, H + 20), f'floor shaded by height; brown: collision over y {fy + 1:.1f}; dark grey: no floor; 5 m grid, +z up', fill=(60, 60, 60))
            imgs.append(img)
        for name, m in moves.items():
            p = next(q for q in pts if q['name'] == name)
            fx, fyy, fz = m.get('from', p['position']); mx, my, mz = m['position']
            cx, cz = (fx + mx) / 2, (fz + mz) / 2; half = 1.6; px = 90; step = 0.04
            xs = np.arange(cx - half + step / 2, cx + half, step); zs = np.arange(cz - half + step / 2, cz + half, step)
            w = sub_world(world, [cx - half - 0.5, min(fyy, my) - 1, cz - half - 0.5], [cx + half + 0.5, max(fyy, my) + 2.5, cz + half + 0.5])
            bl = blocked_grid(w, xs, zs, min(fyy, my))
            S = int(2 * half * px)
            img = Image.new('RGB', (max(S, 520), S + 56), (255, 255, 255)); d = ImageDraw.Draw(img)
            sx = lambda x: (x - (cx - half)) * px; sz = lambda z: ((cz + half) - z) * px
            for j, z in enumerate(zs):
                for i, x in enumerate(xs):
                    col = (110, 84, 70) if bl[j, i] else (238, 236, 230)
                    d.rectangle([sx(x - step / 2), sz(z + step / 2), sx(x + step / 2), sz(z - step / 2)], fill=col)
            rr = cap * px
            d.ellipse([sx(fx) - rr, sz(fz) - rr, sx(fx) + rr, sz(fz) + rr], outline=(220, 30, 30), width=2)
            d.ellipse([sx(mx) - rr, sz(mz) - rr, sx(mx) + rr, sz(mz) + rr], outline=(200, 0, 200), width=3)
            d.line([sx(fx), sz(fz), sx(mx), sz(mz)], fill=(200, 0, 200), width=2)
            d.text((4, S + 3), f"{v['label']} team {p['team']} {name}", fill=(0, 0, 0))
            d.text((4, S + 18), f"{[round(c, 3) for c in (fx, fyy, fz)]} -> {[round(c, 3) for c in (mx, my, mz)]}", fill=(0, 0, 0))
            d.text((4, S + 33), f"red: capsule (r {cap:.2f}) before, magenta: after; brown: collision at {STEP_BAND}-1.82 m over the feet; {2 * half:.1f} m wide", fill=(60, 60, 60))
            imgs.append(img)
        # Rows of at most 3 images.
        rows = [imgs[i:i + 3] for i in range(0, len(imgs), 3)]
        Wt = max(sum(i.width for i in row) + 10 * (len(row) - 1) for row in rows); Ht = sum(max(i.height for i in row) for row in rows) + 10 * (len(rows) - 1)
        sheet = Image.new('RGB', (Wt, Ht), (255, 255, 255)); yo = 0
        for row in rows:
            xo = 0
            for i in row: sheet.paste(i, (xo, yo)); xo += i.width + 10
            yo += max(i.height for i in row) + 10
        sheet.save(out / f"{v['label']}.png"); print(out / f"{v['label']}.png", sheet.size, flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('audit'); p.add_argument('config'); p.add_argument('out'); p.add_argument('--fail', action='store_true'); p.add_argument('--only', nargs='*')
    p = sub.add_parser('propose'); p.add_argument('config'); p.add_argument('audit'); p.add_argument('plan'); p.add_argument('--search', type=float, default=4.0)
    p = sub.add_parser('render'); p.add_argument('config'); p.add_argument('audit'); p.add_argument('out'); p.add_argument('--plan')
    a = ap.parse_args()
    dict(audit=cmd_audit, propose=cmd_propose, render=cmd_render)[a.cmd](a)


if __name__ == '__main__':
    main()
