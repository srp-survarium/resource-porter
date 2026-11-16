#!/usr/bin/env python3
"""The bushes of the STALKER maps as collision a foot step meets: every map of a
built pack's bundle gets them (build-pack.sh runs this after the import).

    bushes.py MAP... --bundle BUNDLES026/PACK [--maps ports/stalker-tdm/maps.json]

Each MAP is a key of maps.json; BUNDLE is build-survarium-maps-026's output for
them (overlay/ and port-manifest.json), changed in place: each map's model
models/srp/<level>_bushes is written into the overlay, placed in its Team
Deathmatch project, and both are entered in port-manifest.json (sha256 and size),
so `pack from-bundle` takes them as the importer's own files.

What a step is in 0.26g: step_manager::on_step (0x248AD0) casts a ray from 1 m
above the foot straight down, 2 m long, at the collision objects of group 8 whose
mask has 10h or 20h, and plays the step sound of the pair (the foot's material,
the material of the triangle met). The stock bushes that rustle (Tarakanovsky
Fort's and Vector's, models/flora/vine/bush_*) have a `collision` mesh of game
material foliage_deciduous, which the ray meets before the ground. The ported
levels have no collision for their leaves: build-soc-map drops X-Ray's passable
triangles.

A map's model is a `collision` mesh alone of game material foliage_deciduous (44):

- its triangles are the map's leaves as they are drawn: the triangles of the
  level's chunks whose material is foliage (base material `flora`) with a texture
  of LEAVES (branches and needles; bark and stumps are solid in the level's own
  collision), a leaf's two drawn sides once;
- only what a step can meet: a triangle is kept when a point of it (tried every
  SAMPLE) is above a floor of the level's collision by REACH at most, and is cut
  off at REACH above the highest such floor. A floor is a collision triangle that
  faces up, within FLOOR_SLOPE of level, and is not a tree's wood (NOT_FLOORS):
  the ground, a slab, a roof; never an underside, never a branch's top. The leaves
  over a walker's head and the crowns are not in it;
- inside the box of the level's collision (the physics world's, which stays as
  it is): a part of a leaf out of it is left out;
- in the map's frame (the level's chunks stand at the origin, unturned);
- no bigger than stock's biggest collision mesh (MOST_TRIANGLES): more
  triangles are several models, <level>_bushes, <level>_bushes_1, ...

Every drawn surface of a level is leaves (LEAVES), foliage that is not (SOLID), a
surface of another kind of material (NOT_FOLIAGE) or one drawn with no material
(not leaves). A foliage texture on neither list stops it with its name. A map
whose leaves no step meets gets no model.

The model is placed after the level's own chunks as a collision object of cgroup
8 and cmask 20h, which no object of the game has; the entry is the level's first
chunk's (at the origin, unturned) with the model's name and filter, added as an
edit adds one (its records and the list's new array appended to the file, the
other records as they are). The step's ray (group 30h, mask 8) meets it; bullets,
blasts and the lobby's and the warm-up's cameras (group 10h), grenades (818h),
batteries (0A18h), the use ray (114h), the walkers (mask 3) and the Skhid ragdoll
patch's level ray (414h) do not. A spot's ray (group 0FFFFh) does: it goes on
through it as through anything hittable that is not walkable.

This is skhid's stalker-bushes@2 (srp-026g/scripts/stalker-bushes/make_pack.py,
docs/notes/stalker-maps.md "Bushes rustle") made part of the port: the same rule
gives Experimental Pool, Military Warehouses and Training Camp the same bytes.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parents[2]

# The foliage textures of the levels (the file name of texture_diffuse; the material's base is
# `flora`): what a walker goes through (branches with leaves or needles, bare twigs of a bush,
# reeds), and what is not leaves (bark, stumps and planks, which stand in the level's own
# collision; the moss hanging in Sports Center's basin).
LEAVES = {'trees_vetkagreen1', 'trees_vetkagreen3', 'trees_vetkagreen5', 'trees_vetkabig', 'trees_vetkasux1_v',
          'trees_vetkasux2', 'trees_sosna', 'trees_sosna_sux', 'trees_sosna_low', 'trees_sosna_sux_low', 'trees_dub',
          'trees_elka', 'trees_elka_sux', 'trees_orex', 'trees_a_bush', 'trees_kamish_big_01', 'trees_kamish_big_02'}
SOLID = {'trees_bark_a_01', 'trees_bark_a_02', 'trees_bark_sosna_01', 'trees_bark_sosna_02', 'trees_bark_sux_mox',
         'trees_dub_bark', 'trees_byaka1', 'trees_spil', 'wood_collect', 'trees_tina'}
# The base materials of the levels' other surfaces: walls, floors and props, metal, the ground,
# lamps' glow, water.
NOT_FOLIAGE = {'dielectric', 'complex', 'terrain_material', 'forward_emissive', 'forward_simple_water'}
# The game material of stock's bushes (game_materials/game.materials; models/flora/vine/bush_big).
MATERIAL = ('foliage_deciduous', 44)
FACE = 'default'
# How far above a floor a step meets a leaf, metres: on_step's ray starts 1 m above the foot
# (0x248B25), the foot bone 0.2 m above the floor at most.
REACH = 1.2
# A triangle's points are tried this far apart at most, metres.
SAMPLE = 0.5
# A floor: a collision triangle that faces up, its normal within 60 degrees of straight up. The
# normal is (b - a) x (c - a) of the triangle's corners in the file's order: the levels' ground
# has it up.
FLOOR_SLOPE = 0.5
# Not a floor, whichever way it faces: the trees' own wood. The port gives X-Ray's tree trunks and
# branches (and its planks) one game material; a branch's top would keep the crown over it.
NOT_FLOORS = {'wood_silent'}
# The collision object: hittable (8), tried by a ray whose group has 20h (the docstring).
CGROUP, CMASK = 8, 0x20
# The floors' grid, metres.
CELL = 1.0
# Chunk tags: a drawn mesh's vertices and triangles, a collision's.
DRAWN_VERTICES, DRAWN_INDICES = 3, 4
VERTICES, INDICES, MATERIALS, FACES = 0x19, 0x1a, 0x1b, 0x1c
# The stock collision meshes go up to 64,801 triangles (models/level/04/wagons_crash).
MOST_TRIANGLES = 64801
SOURCE = 'srp-bushes'

T_BOOL, T_INT, T_FLOAT, T_NAMED, T_INDEXED, T_STR = 0, 1, 2, 3, 4, 5
REC = struct.Struct('<QQIHH')


class Unknown(Exception):
    """Something of the bundle this has no rule for: it makes nothing."""


def sha256(data):
    return hashlib.sha256(data).hexdigest()


# A binary config as a tree: a value is (type, data); a named table's data a list of (name, value),
# an indexed one's a list of values, a string's its text, a vector's its floats, a number's or a
# boolean's the record's raw u64.
def parse(buf, off=0):
    data, _, _, t, cnt = REC.unpack_from(buf, off)
    if t in (T_NAMED, T_INDEXED):
        kids = []
        for k in range(cnt):
            c = data + 24 * k
            v = parse(buf, c)
            if t == T_NAMED:
                i = REC.unpack_from(buf, c)[1]
                kids.append((bytes(buf[i:buf.index(b'\0', i)]).decode(), v))
            else:
                kids.append(v)
        return (t, kids)
    if t == T_STR:
        return (t, bytes(buf[data:data + cnt - 1]).decode())
    if t > T_STR:
        return (t, list(struct.unpack_from('<%df' % (t - 4), buf, data)))
    return (t, data)


def serialize(root):
    """The tree as a config: the records (a table's children side by side, the tables in the order
    they are reached), then the names, strings and vectors; a named table's children sorted by CRC,
    as the game's lookup needs."""
    heap, strings = bytearray(), {}

    def text(s):
        if s not in strings:
            strings[s] = len(heap)
            heap.extend(s.encode() + b'\0')
        return strings[s]

    slots = [[root, None]]
    queue = [0]
    first = {}
    while queue:
        at = queue.pop(0)
        t, v = slots[at][0]
        if t in (T_NAMED, T_INDEXED):
            kids = v
            if t == T_NAMED:
                kids = sorted(v, key=lambda kv: zlib.crc32(kv[0].encode()) & 0xffffffff)
            first[at] = len(slots)
            for kid in kids:
                slots.append([kid[1], kid[0]] if t == T_NAMED else [kid, None])
                queue.append(len(slots) - 1)
    heap_base = 24 * len(slots)
    out = bytearray()
    for at, (value, name) in enumerate(slots):
        t, v = value
        name_off = heap_base + text(name) if name is not None else 0
        crc = zlib.crc32(name.encode()) & 0xffffffff if name is not None else 0
        if t in (T_NAMED, T_INDEXED):
            data, cnt = 24 * first[at], len(v)
        elif t == T_STR:
            data, cnt = heap_base + text(v), len(v.encode()) + 1
        elif t > T_STR:
            data, cnt = heap_base + len(heap), 4 * len(v)
            heap.extend(struct.pack('<%df' % len(v), *v))
        else:
            data, cnt = v, 4
        out += REC.pack(data, name_off, crc, t, cnt)
    return bytes(out + heap)


def get(tree, path):
    for part in path.split('/'):
        tree = dict(tree[1])[part]
    return tree


def value(tree, path):
    """The value at a/b/c of a config's tree, None when it has no such key."""
    try:
        return get(tree, path)[1]
    except KeyError:
        return None


class Overlay:
    """The bundle's files, each read once."""

    def __init__(self, folder):
        self.folder = folder
        self.read = {}

    def file(self, path):
        if path not in self.read:
            self.read[path] = (self.folder / path).read_bytes()
        return self.read[path]

    def config(self, path):
        return parse(bytearray(self.file(path)))


def chunk(data, at, tag):
    """(payload, the offset after it) of the chunk at `at`, which must be `tag`."""
    kind, size = struct.unpack_from('<II', data, at)
    assert kind == tag and at + 8 + size <= len(data), f'chunk {tag:#x} expected'
    return data[at + 8:at + 8 + size], at + 8 + size


def whole(data, tag):
    payload, end = chunk(data, 0, tag)
    assert end == len(data), 'bytes after the chunk'
    return payload


def at_origin(entry):
    """A project entry that stands at the origin, unturned and unscaled."""
    return (value(entry, 'position') == [0.0, 0.0, 0.0] and value(entry, 'rotation') == [0.0, 0.0, 0.0]
            and value(entry, 'scale') == [1.0, 1.0, 1.0])


def collision_triangles(overlay, model):
    """The triangles of a level chunk's collision: (three points, its game material's name) each."""
    base = model + '/collision/'
    raw = whole(overlay.file(base + 'vertices'), VERTICES)
    points = [struct.unpack_from('<3f', raw, k) for k in range(0, len(raw), 12)]
    raw = whole(overlay.file(base + 'indices'), INDICES)
    indices = struct.unpack('<%dI' % (len(raw) // 4), raw)
    data = overlay.file(base + 'face_data')
    names, at = chunk(data, 0, MATERIALS)
    count = struct.unpack_from('<H', names, 0)[0]
    names = [name.decode() for name in names[2:].split(b'\0')[:-1]]
    assert len(names) == count, f'{base}face_data: {count} materials, {len(names)} names'
    raw, end = chunk(data, at, FACES)
    assert end == len(data) and len(raw) == 2 * len(indices) // 3, f'{base}face_data: not a face a triangle'
    faces = struct.unpack('<%dH' % (len(raw) // 2), raw)
    settings = overlay.config(model + '/settings')
    materials = [value(settings, f'game_material_settings/{name}/game_material_name') for name in names]
    assert all(materials), f'{model}: a face without a game material'
    return [((points[indices[3 * k]], points[indices[3 * k + 1]], points[indices[3 * k + 2]]), materials[face])
            for k, face in enumerate(faces)]


def drawn_triangles(data):
    """The triangles (three points each) of a drawn surface (converted_model)."""
    tag, size, count = struct.unpack_from('<III', data, 0)
    assert tag == DRAWN_VERTICES and count and (size - 4) % count == 0, 'not a drawn mesh'
    stride = (size - 4) // count
    points = [struct.unpack_from('<3f', data, 12 + stride * k) for k in range(count)]
    at = 8 + size
    tag, size, count = struct.unpack_from('<III', data, at)
    assert tag == DRAWN_INDICES and size == 4 + 2 * count and at + 8 + size == len(data), 'not a triangle list'
    indices = struct.unpack_from('<%dH' % count, data, at + 12)
    return [(points[indices[k]], points[indices[k + 1]], points[indices[k + 2]]) for k in range(0, count, 3)]


class Floors:
    """The level's floors: its collision triangles that face up within FLOOR_SLOPE of level and are
    not of NOT_FLOORS, by the cells of a grid over x and z."""

    def __init__(self):
        self.cells = {}
        self.low = [math.inf] * 3
        self.high = [-math.inf] * 3
        self.up = self.down = 0.0   # the area that faces up, and down, within FLOOR_SLOPE of level

    def add(self, triangle, material):
        a, b, c = triangle
        for point in triangle:              # the box of all the collision, floors or not
            for k in range(3):
                self.low[k] = min(self.low[k], point[k])
                self.high[k] = max(self.high[k], point[k])
        nx = (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1])
        ny = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2])
        nz = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        length = math.sqrt(nx * nx + ny * ny + nz * nz)
        # d: twice the triangle's area seen from above; a floor has some.
        d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2])
        if length == 0 or abs(ny) < FLOOR_SLOPE * length or d == 0:
            return
        if ny < 0:                          # an underside: a ceiling, a branch from below
            self.down += length / 2
            return
        self.up += length / 2
        if material in NOT_FLOORS:
            return
        floor = (a, b, c, d, min(a[1], b[1], c[1]), max(a[1], b[1], c[1]))
        for i in range(math.floor(min(a[0], b[0], c[0]) / CELL), math.floor(max(a[0], b[0], c[0]) / CELL) + 1):
            for j in range(math.floor(min(a[2], b[2], c[2]) / CELL), math.floor(max(a[2], b[2], c[2]) / CELL) + 1):
                self.cells.setdefault((i, j), []).append(floor)

    def under(self, point):
        """The highest floor under `point` within REACH of it, or None."""
        x, y, z = point
        best = None
        for a, b, c, d, low, high in self.cells.get((math.floor(x / CELL), math.floor(z / CELL)), ()):
            if low >= y or high < y - REACH:
                continue
            u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d
            v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d
            w = 1 - u - v
            if u < 0 or v < 0 or w < 0:
                continue
            height = u * a[1] + v * b[1] + w * c[1]
            if y - REACH <= height < y and (best is None or height > best):
                best = height
        return best


def samples(triangle):
    """Points of a triangle no further apart than SAMPLE, its corners among them."""
    a, b, c = triangle
    n = max(1, math.ceil(max(math.dist(a, b), math.dist(b, c), math.dist(c, a)) / SAMPLE))
    for i in range(n + 1):
        for j in range(n + 1 - i):
            k = n - i - j
            yield tuple((i * a[m] + j * b[m] + k * c[m]) / n for m in range(3))


def below(triangle, top):
    """The part of a triangle at or under the height `top`: 0, 1 or 2 triangles."""
    inside = [p for p in triangle if p[1] <= top]
    if len(inside) == 3:
        return [triangle]
    if not inside:
        return []
    polygon = []
    for k in range(3):
        p, q = triangle[k], triangle[(k + 1) % 3]
        if p[1] <= top:
            polygon.append(p)
        if (p[1] <= top) != (q[1] <= top):
            t = (top - p[1]) / (q[1] - p[1])
            polygon.append((p[0] + t * (q[0] - p[0]), top, p[2] + t * (q[2] - p[2])))
    return [(polygon[0], polygon[k], polygon[k + 1]) for k in range(1, len(polygon) - 1)]


def f32(point):
    """A point as the file holds it."""
    return struct.unpack('<3f', struct.pack('<3f', *point))


def area(triangle):
    a, b, c = triangle
    u = [b[k] - a[k] for k in range(3)]
    v = [c[k] - a[k] for k in range(3)]
    n = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
    return math.sqrt(sum(x * x for x in n)) / 2


def bounds(points):
    lo = [min(p[k] for p in points) for k in range(3)]
    hi = [max(p[k] for p in points) for k in range(3)]
    return lo, hi


def level_chunks(project, models, level, table):
    """The entries of a project's table (render_visuals, collision_objects) that are the level's own
    chunks (models/<models><level>_<n>), each with its model's path and its number; they stand at
    the origin."""
    solid = table == 'collision_objects'
    prefix, suffix = (('resources/models/', '.model/collision') if solid else ('', ''))
    prefix += f'{models}{level}_'
    out = []
    for entry in get(project, table)[1]:
        name = value(entry, 'lib_name')
        number = name[len(prefix):len(name) - len(suffix)]
        if name.startswith(prefix) and name.endswith(suffix) and number.isdigit():
            assert at_origin(entry), f'{name}: not at the origin'
            assert solid or value(entry, 'visual_name') == name, f'{name}: another visual'
            out.append((entry, 'models/' + name.removeprefix('resources/models/').removesuffix(suffix) + '.model',
                        int(number)))
    assert out, f'{table}: no chunk of {level}'
    return out


def leaves(overlay, project, models, level):
    """The level's leaves: the triangles of its drawn chunks whose material is foliage with a
    texture of LEAVES; the triangles drawn of each such texture; the surfaces drawn with no
    material. A foliage surface on neither LEAVES nor SOLID is refused by its name."""
    triangles, drawn, bare = [], {}, 0
    for _, model, number in level_chunks(project, models, level, 'render_visuals'):
        settings = overlay.config(model + '/settings')
        for surface in (s[1] for s in get(settings, 'lod_hierrarchy/LOD0/surfaces')[1]):
            name = value(settings, f'material_settings/{surface}/material_name')
            if not name:
                bare += 1
                continue
            material = overlay.config(f'material_instances/{name}.material')
            base = value(material, 'material/instance/base_material')
            if base in NOT_FOLIAGE:
                continue
            if base != 'flora':
                raise Unknown(f'{model} {surface}: material {name} of base {base}: foliage or not (NOT_FOLIAGE)?')
            texture = value(material, 'material/g_stage/effect/texture_diffuse/value').rsplit('/', 1)[-1]
            if texture in SOLID:
                continue
            if texture not in LEAVES:
                raise Unknown(f'{model} {surface}: foliage of texture {texture}: leaves (LEAVES) or not (SOLID)?')
            mesh = drawn_triangles(overlay.file(f'{model}/render/{surface}/converted_model'))
            triangles += mesh
            drawn[texture] = drawn.get(texture, 0) + len(mesh)
    return triangles, drawn, bare


def bushes(overlay, models, level, project_path):
    """A map's models: {model name: its files} (none when no step meets a leaf; more than one when
    its triangles are more than MOST_TRIANGLES), the lib_names of the level's first and last
    collision chunks (each entry is made like the first, after the last) and a report."""
    project = overlay.config(project_path)
    floors = Floors()
    solid = level_chunks(project, models, level, 'collision_objects')
    for _, model, _ in solid:
        for triangle, material in collision_triangles(overlay, model):
            floors.add(triangle, material)
    # The level's collision faces up more than down (its ground does): the floors are its floors.
    assert floors.up > floors.down > 0, f'{level}: {floors.up:.0f} m2 face up, {floors.down:.0f} m2 down'
    drawn, by_texture, bare = leaves(overlay, project, models, level)

    points, index, triangles, kept, seen, outside = [], {}, [], 0, set(), 0
    for triangle in drawn:
        # The level draws a leaf's two sides as two triangles; the ray meets either side of one.
        if frozenset(triangle) in seen:
            continue
        seen.add(frozenset(triangle))
        tops = [t for t in map(floors.under, samples(triangle)) if t is not None]
        if not tops:
            continue
        kept += 1
        for part in below(triangle, max(tops) + REACH):
            part = tuple(f32(p) for p in part)
            if area(part) == 0:
                continue
            # Inside the level's collision: the physics world's box, which the level's collision
            # makes (bullet_physics_world::add 0x758E1, groups 8 and 2), stays as it is.
            if not all(floors.low[k] <= c[k] <= floors.high[k] for c in part for k in range(3)):
                outside += 1
                continue
            for corner in part:
                if corner not in index:
                    index[corner] = len(points)
                    points.append(corner)
            triangles.append(tuple(index[corner] for corner in part))
    textures = ', '.join(f'{t} {n}' for t, n in sorted(by_texture.items()))
    report = (f'{level}: {len(drawn)} leaf triangles drawn ({textures or "none"}), {bare} surfaces drawn with no '
              f'material, {len(seen)} leaves, {kept} within a step of a floor'
              + (f', {outside} parts of them out of the level\'s collision box left out' if outside else ''))
    if not triangles:
        return {}, None, None, report + '; no model'
    low, high = bounds(points)
    spread = sum(area(tuple(points[i] for i in t)) for t in triangles)
    report += (f'; {len(points)} vertices, {len(triangles)} triangles, {spread:.0f} m2; '
               f'{[round(c, 2) for c in low]} .. {[round(c, 2) for c in high]}')
    # No bigger mesh than stock has: the triangles in that many runs as they come, each run a model
    # of its own (models/srp/<level>_bushes, then <level>_bushes_1 ...), its corners in the order
    # it meets them.
    count = math.ceil(len(triangles) / MOST_TRIANGLES)
    size = math.ceil(len(triangles) / count)
    out = {}
    for k in range(count):
        run = triangles[k * size:(k + 1) * size]
        renumber = {}
        for i in (i for t in run for i in t):
            renumber.setdefault(i, len(renumber))
        name = model_name(level, k)
        out[name] = mesh_files(name, [points[i] for i in renumber], [tuple(renumber[i] for i in t) for t in run])
    if count > 1:
        report += f'; {count} models of at most {size} triangles'
    return out, value(solid[0][0], 'lib_name'), value(solid[-1][0], 'lib_name'), report


def model_name(level, k):
    return f'models/srp/{level}_bushes' + (f'_{k}' if k else '')


def mesh_files(name, points, triangles):
    """A model's files: its collision mesh of the points and the triangles, its settings."""
    vertices = b''.join(struct.pack('<3f', *p) for p in points)
    indices = struct.pack('<%dI' % (3 * len(triangles)), *(i for t in triangles for i in t))
    dictionary = struct.pack('<H', 1) + FACE.encode() + b'\0'
    faces = bytes(2 * len(triangles))
    model = f'{name}.model/'
    return {
        model + 'collision/vertices': struct.pack('<II', VERTICES, len(vertices)) + vertices,
        model + 'collision/indices': struct.pack('<II', INDICES, len(indices)) + indices,
        model + 'collision/face_data': (struct.pack('<II', MATERIALS, len(dictionary)) + dictionary
                                        + struct.pack('<II', FACES, len(faces)) + faces),
        model + 'settings': settings(),
    }


def settings():
    """A model of collision alone, as stock's bushes with one are set (models/flora/vine/bush_big)
    without their surfaces: every triangle of game material MATERIAL."""
    tree = (T_NAMED, [
        ('collision_enabled', (T_BOOL, 1)),
        ('walker_collision_enabled', (T_BOOL, 0)),
        ('collision_use_raytrace_for_walker', (T_BOOL, 0)),
        ('calculate_ao', (T_BOOL, 0)),
        ('lod_hierrarchy', (T_NAMED, [(name, (T_NAMED, [])) for name in ('unassigned', 'LOD0', 'LOD1', 'LOD2')])),
        ('material_settings', (T_NAMED, [])),
        ('game_material_settings', (T_NAMED, [(FACE, (T_NAMED, [
            ('game_material_id', (T_INT, MATERIAL[1])),
            ('game_material_name', (T_STR, MATERIAL[0])),
        ]))])),
        ('lod_switching', (T_NAMED, [('default_params', (T_BOOL, 1)), ('type', (T_INT, 0))])),
    ])
    data = serialize(tree)
    assert serialize(parse(bytearray(data))) == data
    return data


# The entry's placing: an edit of the project's records, the file as it is but for them.
def children(buf, off):
    data, _, _, t, cnt = REC.unpack_from(buf, off)
    assert t in (T_NAMED, T_INDEXED), 'not a table'
    return [data + 24 * i for i in range(cnt)]


def child(buf, table, name):
    for c in children(buf, table):
        i = REC.unpack_from(buf, c)[1]
        if bytes(buf[i:buf.index(b'\0', i)]) == name.encode():
            return c
    raise KeyError(name)


def text_of(buf, off):
    data, _, _, t, cnt = REC.unpack_from(buf, off)
    assert t == T_STR, 'not a text'
    return bytes(buf[data:data + cnt - 1]).decode()


def append(buf, blob):
    """`blob` at the end of the file; its offset."""
    at = len(buf)
    buf.extend(blob)
    return at


def append_array(buf, records):
    """24-byte records at the end of the file, at an 8-byte boundary as the game's files keep
    their arrays; their offset."""
    buf.extend(bytes(-len(buf) % 8))
    return append(buf, b''.join(records))


def place(buf, after, like, values):
    """A collision object after the entry named `after`: the entry named `like` copied (its keys,
    types and order), the keys of `values` set (a text, an int). Its records and the list's new
    array are appended to the file; the old array stays, unreferenced, the other records as they
    are. Returns its index."""
    objects = child(buf, 0, 'collision_objects')
    entries = children(buf, objects)
    names = [text_of(buf, child(buf, e, 'lib_name')) for e in entries]
    assert names.count(after) == 1 and names.count(like) == 1, (after, like)
    model = entries[names.index(like)]
    records = []
    for c in children(buf, model):
        d, i, crc, t, cnt = REC.unpack_from(buf, c)
        assert t not in (T_NAMED, T_INDEXED), 'a collision object of tables'
        name = bytes(buf[i:buf.index(b'\0', i)]).decode()
        if name in values:
            v = values[name]
            if t == T_STR:
                text = v.encode()
                d, cnt = append(buf, text + b'\0'), len(text) + 1
            else:
                assert t == T_INT and isinstance(v, int), (name, v)
                d = (d & ~0xffffffff) | (v & 0xffffffff)
        records.append(REC.pack(d, i, crc, t, cnt))
    assert {bytes(buf[REC.unpack(r)[1]:buf.index(b'\0', REC.unpack(r)[1])]).decode() for r in records} >= set(values)
    d, i, crc, t, cnt = REC.unpack_from(buf, model)
    entry = REC.pack(append_array(buf, records), i, crc, t, cnt)
    old = [bytes(buf[c:c + 24]) for c in entries]
    at = names.index(after) + 1
    new = old[:at] + [entry] + old[at:]
    d, i, crc, t, cnt = REC.unpack_from(buf, objects)
    REC.pack_into(buf, objects, append_array(buf, new), i, crc, t, cnt + 1)
    return at


def placement(name):
    """A model as a collision object: at the origin, unturned, as the level's own chunks."""
    return {'lib_name': f'resources/{name}.model/collision', 'cgroup': CGROUP, 'cmask': CMASK,
            'position': [0.0, 0.0, 0.0], 'rotation': [0.0, 0.0, 0.0], 'scale': [1.0, 1.0, 1.0]}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('map', nargs='+')
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--maps', type=Path, default=ROOT / 'ports/stalker-tdm/maps.json')
    a = ap.parse_args()
    maps = json.loads(a.maps.read_text())['maps']
    overlay_dir = a.bundle / 'overlay'
    record_path = a.bundle / 'port-manifest.json'
    record = json.loads(record_path.read_text())
    listed = {f['path']: f for f in record['files']}
    written = {}
    for key in a.map:
        m = maps[key]
        namespace = json.loads((ROOT / 'ports' / f"{m['recipe']}.json").read_text())['namespace']
        level = namespace.rsplit('/', 1)[-1]
        project_path = f"projects/v010b_{m['project']}/day_4/client_project"
        overlay = Overlay(overlay_dir)
        try:
            models, first, last, report = bushes(overlay, f'ported/v010b/{namespace}/', level, project_path)
        except Unknown as e:
            sys.exit(str(e))
        print(report)
        if not models:
            continue
        # The project with the models placed after the level's last collision chunk, each made like
        # its first: read back, those objects more, the others as they were.
        buf = bytearray(overlay.file(project_path))
        for name, files in models.items():
            before = get(parse(bytearray(buf)), 'collision_objects')[1]
            values = {k: v for k, v in placement(name).items() if k in ('lib_name', 'cgroup', 'cmask')}
            at = place(buf, last, first, values)
            after = get(parse(buf), 'collision_objects')[1]
            assert after[:at] + after[at + 1:] == before, f'{project_path}: not placed'
            assert value(after[at - 1], 'lib_name') == last, f'{project_path}: not after {last}'
            assert {k: e[1] for k, e in after[at][1]} == placement(name), f'{project_path}: not as placed'
            last = values['lib_name']
            for path in files:
                assert path not in listed and not (overlay_dir / path).exists(), f'{path}: the bundle has it'
            written.update(files)
        written[project_path] = bytes(buf)
    for path, data in written.items():
        target = overlay_dir / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        entry = listed.get(path)
        if entry is None:
            entry = {'path': path, 'source': SOURCE}
            record['files'].append(entry)
        entry['size'] = len(data)
        entry['sha256'] = sha256(data)
    record_path.write_text(json.dumps(record, indent=2) + '\n')
    print(f'{a.bundle}: {sum(p.startswith("models/srp/") for p in written)} files of bushes, '
          f'{sum(p.startswith("projects/") for p in written)} projects placed in')


if __name__ == '__main__':
    main()
