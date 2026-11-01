"""zones.py: a placed converted_model cut by the lobby's unbaked zones (no Blender).

The characters (the squad at the origin) and the upgrade window's weapon cast
shadows only on what the engine lights; a baked surface (black, emitting its
page) takes no light. So the faces inside an unbaked zone (a box in the lobby
frame) stay the model's own, drawn and lit by the engine, and only those
outside are baked. A triangle across a zone's side is cut on it: the pieces
keep the model's vertex format (28 bytes, or 32 with a colour), their new
vertices interpolated (position, uv and colour linearly, the normal, tangent
and binormal renormalised), so both sides of the seam meet exactly.
"""
import math
import struct



def rotation_rows(a):
    """0.26g's Euler rotation of a placed object, as rows (common.rotation's)."""
    sx, cx = math.sin(a[0]), math.cos(a[0])
    sy, cy = math.sin(a[1]), math.cos(a[1])
    sz, cz = math.sin(a[2]), math.cos(a[2])
    return ((cy * cz, -cy * sz, sy),
            (sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy),
            (-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy))

# The squad stands at the origin and one metre to either side, half a metre
# ahead (0.26g's lobby: (1, 0, 0.5) and (-1, 0, 0.5)); their shadows fall on
# the floor around them and the walls close by.
SQUAD_ZONE = [[-1.7, -0.5, -0.9], [1.7, 2.4, 1.3]]
# The upgrade window's weapon floats over its table: its shadow on the top
# and the floor under it.
WEAPON_HALF = [0.9, 0.9]  # horizontal half extent; below, above:
WEAPON_BELOW, WEAPON_ABOVE = 1.5, 0.6


def default_zones(project):
    """The squad's zone and, when the project has srp_lobby's weapon, the
    weapon's."""
    zones = [{"name": "squad", "box": SQUAD_ZONE}]
    w = project.get("srp_lobby.weapon")
    if w:
        zones.append({"name": "weapon", "box": [[w[0] - WEAPON_HALF[0], w[1] - WEAPON_BELOW, w[2] - WEAPON_HALF[1]],
                                                [w[0] + WEAPON_HALF[0], w[1] + WEAPON_ABOVE, w[2] + WEAPON_HALF[1]]]})
    return zones


def zone_centre(zone):
    lo, hi = zone["box"]
    return [(lo[k] + hi[k]) / 2 for k in range(3)]


class Model:
    """A converted_model's vertices (raw records) and triangles."""

    def __init__(self, data):
        tag, size, n = struct.unpack_from("<III", data, 0)
        assert tag == 3 and n and (size - 4) % n == 0, "no vertex block"
        self.stride = (size - 4) // n
        assert self.stride in (28, 32), f"{self.stride}-byte vertices"
        self.records = [data[12 + i * self.stride:12 + (i + 1) * self.stride] for i in range(n)]
        o = 8 + size
        tag, isize, m = struct.unpack_from("<III", data, o)
        assert tag == 4 and isize == 4 + 2 * m and len(data) == o + 12 + 2 * m, "bad index block"
        idx = struct.unpack_from(f"<{m}H", data, o + 12)
        self.triangles = [tuple(idx[i:i + 3]) for i in range(0, m, 3)]

    def encode(self, records, triangles):
        assert len(records) <= 35003, f"{len(records)} vertices: more than 0.26g loads"
        out = bytearray(struct.pack("<III", 3, 4 + self.stride * len(records), len(records)))
        for r in records:
            out += r
        out += struct.pack("<III", 4, 4 + 6 * len(triangles), 3 * len(triangles))
        for t in triangles:
            out += struct.pack("<3H", *t)
        return bytes(out)


def _unpack(v):
    return [((v >> 16) & 255) / 127.5 - 1.0, ((v >> 8) & 255) / 127.5 - 1.0, (v & 255) / 127.5 - 1.0]


def _pack(vec, alpha):
    length = math.sqrt(sum(c * c for c in vec)) or 1.0
    c = lambda t: max(0, min(255, int(math.floor((t / length + 1.0) * 127.5))))
    return (alpha << 24) | (c(vec[0]) << 16) | (c(vec[1]) << 8) | c(vec[2])


def _half(h):
    return struct.unpack("<e", struct.pack("<H", h))[0]


def _half_bits(x):
    return struct.unpack("<H", struct.pack("<e", x))[0]


def _lerp_record(a, b, t, stride):
    """The vertex a fraction t of the way from record a to b."""
    pa, pb = struct.unpack_from("<3f", a), struct.unpack_from("<3f", b)
    out = bytearray(struct.pack("<3f", *[pa[k] + (pb[k] - pa[k]) * t for k in range(3)]))
    for off in (12, 16, 20):
        va, vb = struct.unpack_from("<I", a, off)[0], struct.unpack_from("<I", b, off)[0]
        ua, ub = _unpack(va), _unpack(vb)
        out += struct.pack("<I", _pack([ua[k] + (ub[k] - ua[k]) * t for k in range(3)], va >> 24))
    ua, ub = struct.unpack_from("<2H", a, 24), struct.unpack_from("<2H", b, 24)
    out += struct.pack("<2H", *[_half_bits(_half(ua[k]) + (_half(ub[k]) - _half(ua[k])) * t) for k in range(2)])
    if stride == 32:
        out += bytes(round(a[28 + k] + (b[28 + k] - a[28 + k]) * t) for k in range(4))
    return bytes(out)


def split(data, place, zones):
    """The model's triangles inside any zone and outside every one, as two
    converted_models (None for an empty side), the model placed by `place`
    (0.26g: world = (v * scale) * R + position)."""
    m = Model(data)
    R = rotation_rows(place["rotation"])
    s, p = place["scale"], place["position"]

    def world(r):
        v = struct.unpack_from("<3f", r)
        v = [v[k] * s[k] for k in range(3)]
        return [sum(v[j] * R[j][k] for j in range(3)) + p[k] for k in range(3)]

    records = list(m.records)
    where = [world(r) for r in records]
    cut = {}

    def cut_vertex(i, j, plane):
        key = (min(i, j), max(i, j), plane)
        if key not in cut:
            a, b = key[0], key[1]
            da, db = distance(plane, where[a]), distance(plane, where[b])
            t = da / (da - db)
            records.append(_lerp_record(records[a], records[b], t, m.stride))
            where.append([where[a][k] + (where[b][k] - where[a][k]) * t for k in range(3)])
            cut[key] = len(records) - 1
        return cut[key]

    # plane (zone, axis, side): inside when distance <= 0
    def distance(plane, w):
        z, axis, side = plane
        lo, hi = zones[z]["box"]
        return (min(lo[axis], hi[axis]) - w[axis]) if side == 0 else (w[axis] - max(lo[axis], hi[axis]))

    def clip(poly, plane):
        """poly split by the plane: (the part inside, the part outside)."""
        inside, outside = [], []
        for n, i in enumerate(poly):
            j = poly[(n + 1) % len(poly)]
            di, dj = distance(plane, where[i]), distance(plane, where[j])
            if di <= 0:
                inside.append(i)
            if di >= 0:
                outside.append(i)
            if (di < 0 < dj) or (dj < 0 < di):
                c = cut_vertex(i, j, plane)
                inside.append(c)
                outside.append(c)
        return inside, outside

    ins, outs = [], []
    for tri in m.triangles:
        pieces = [list(tri)]  # still to sort: the parts not yet outside every zone
        for z in range(len(zones)):
            rest = []
            for poly in pieces:
                cur = poly
                for plane in [(z, axis, side) for axis in range(3) for side in range(2)]:
                    inside, outside = clip(cur, plane)
                    if len(outside) >= 3:
                        rest.append(outside)
                    cur = inside
                    if len(cur) < 3:
                        break
                if len(cur) >= 3:
                    ins.append(cur)
            pieces = rest
        outs.extend(pieces)

    def fan(polys):
        return [(p[0], p[k], p[k + 1]) for p in polys for k in range(1, len(p) - 1)]

    def model_of(tris):
        if not tris:
            return None
        used = sorted({i for t in tris for i in t})
        remap = {v: k for k, v in enumerate(used)}
        return m.encode([records[i] for i in used], [tuple(remap[i] for i in t) for t in tris])

    return model_of(fan(ins)), model_of(fan(outs))
