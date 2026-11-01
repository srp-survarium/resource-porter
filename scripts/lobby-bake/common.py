"""What bake.py and show.py share: 0.26g models in Blender, the materials,
the lights in 0.26g's units, the views."""
import json
import math
import os
import struct
import time

import bpy
import mathutils

T0 = time.time()


def log(*a):
    print(f"[{time.time() - T0:7.1f}s]", *a, flush=True)


# --- 0.26g <-> Blender ---------------------------------------------------------
# 0.26g is left-handed, y up; Blender right-handed, z up: y and z swap (a
# mirror). Its triangles then turn the other way from Blender's (seen: every
# surface's turn disagrees with its normals unless turned over), and its uv's v
# runs down where Blender's runs up.
def to_b(p):
    return (p[0], p[2], p[1])


def half(h):
    return struct.unpack("<e", struct.pack("<H", h))[0]


def half_bits(x):
    return struct.unpack("<H", struct.pack("<e", x))[0]


def unpack_basis(v):
    return (((v >> 16) & 255) / 127.5 - 1.0, ((v >> 8) & 255) / 127.5 - 1.0, (v & 255) / 127.5 - 1.0)


def pack_basis(vec, alpha):
    x, y, z = vec
    length = math.sqrt(x * x + y * y + z * z) or 1.0
    c = lambda t: max(0, min(255, int(math.floor((t / length + 1.0) * 127.5))))
    return (alpha << 24) | (c(x) << 16) | (c(y) << 8) | c(z)


def read_model(path):
    """A converted_model in Blender's frame: positions, normals, uvs,
    triangles and blend weights. Its vertices are 28 bytes, or 32 with a
    colour after them (the 0.26 vertex-colour meshes: its alpha the weight
    of the material's base layer over its vertex-blended one; else None)."""
    d = open(path, "rb").read()
    tag, size, n = struct.unpack_from("<III", d, 0)
    assert tag == 3 and size - 4 in (28 * n, 32 * n), path
    stride = (size - 4) // n if n else 28
    pos, nrm, uv, weights = [], [], [], [] if stride == 32 else None
    for i in range(n):
        x, y, z, pn, _, _, u, v = struct.unpack_from("<3f3I2H", d, 12 + stride * i)
        pos.append(to_b((x, y, z)))
        nrm.append(to_b(unpack_basis(pn)))
        uv.append((half(u), 1.0 - half(v)))
        if weights is not None:
            weights.append(d[12 + stride * i + 31] / 255.0)
    o = 12 + stride * n
    tag, _, m = struct.unpack_from("<III", d, o)
    assert tag == 4, path
    idx = struct.unpack_from(f"<{m}H", d, o + 12)
    return pos, nrm, uv, [(idx[i], idx[i + 2], idx[i + 1]) for i in range(0, m, 3)], weights


_SWAP = mathutils.Matrix(((1, 0, 0), (0, 0, 1), (0, 1, 0)))


def rotation(a):
    """0.26g's Euler rotation of a placed object (row vectors: world = (v *
    scale) * R + position; resource-porter's instance_rotation)."""
    sx, cx = math.sin(a[0]), math.cos(a[0])
    sy, cy = math.sin(a[1]), math.cos(a[1])
    sz, cz = math.sin(a[2]), math.cos(a[2])
    return mathutils.Matrix(((cy * cz, -cy * sz, sy),
                             (sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy),
                             (-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy)))


def placement(place):
    """A placement (position, rotation, scale) as Blender's matrix."""
    m = _SWAP @ rotation(place["rotation"]).transposed() @ mathutils.Matrix.Diagonal(place["scale"]) @ _SWAP
    out = m.to_4x4()
    out.translation = to_b(place["position"])
    return out


def direction(angles, local):
    """A placed object's local direction (0.26g frame) in Blender's."""
    return (_SWAP @ rotation(angles).transposed() @ mathutils.Vector(local)).normalized()


def read_placed(surface):
    """A scene surface's model where it is placed (scene.json's `place`)."""
    pos, nrm, uv, tris, weights = read_model(surface["file"])
    if "place" not in surface:
        return pos, nrm, uv, tris, weights
    m = placement(surface["place"])
    n3 = m.to_3x3().inverted().transposed()
    pos = [tuple(m @ mathutils.Vector(p)) for p in pos]
    nrm = [tuple((n3 @ mathutils.Vector(n)).normalized()) for n in nrm]
    if m.to_3x3().determinant() < 0:
        tris = [(a, c, b) for a, b, c in tris]
    return pos, nrm, uv, tris, weights


def write_model(path, pos, nrm, uv, tris):
    """A converted_model from Blender's frame: 28-byte vertices (position,
    packed normal, tangent, binormal, half uv) and u16 indices."""
    out = bytearray(struct.pack("<III", 3, 4 + 28 * len(pos), len(pos)))
    for p, n, t in zip(pos, nrm, uv):
        n0 = mathutils.Vector(to_b(n)).normalized()
        tangent = n0.orthogonal().normalized()
        binormal = n0.cross(tangent)
        out += struct.pack("<3f", *to_b(p))
        out += struct.pack("<3I", pack_basis(n0, 127), pack_basis(tangent, 255), pack_basis(binormal, 255))
        out += struct.pack("<2H", half_bits(t[0]), half_bits(1.0 - t[1]))
    out += struct.pack("<III", 4, 4 + 6 * len(tris), 3 * len(tris))
    for a, b, c in tris:
        out += struct.pack("<3H", a, c, b)
    open(path, "wb").write(out)


# --- the scene -----------------------------------------------------------------
def new_scene(threads, gpu=False):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    if gpu:  # OptiX, else CUDA; a Blender without them (nixpkgs') stops here
        prefs = bpy.context.preferences.addons["cycles"].preferences
        for kind in ("OPTIX", "CUDA"):
            try:
                prefs.compute_device_type = kind
            except TypeError:
                continue
            prefs.get_devices()
            devices = [d for d in prefs.devices if d.type == kind]
            if devices:
                for d in prefs.devices:
                    d.use = d.type == kind
                sc.cycles.device = "GPU"
                log(f"Cycles on {kind}: {', '.join(d.name for d in devices)}")
                break
        else:
            raise SystemExit("--gpu: this Blender has no OptiX or CUDA device")
    sc.render.threads_mode = "FIXED"
    sc.render.threads = threads
    sc.cycles.use_denoising = True
    sc.cycles.max_bounces = 8
    sc.cycles.diffuse_bounces = 6
    sc.cycles.glossy_bounces = 2
    sc.cycles.transparent_max_bounces = 16
    sc.world = bpy.data.worlds.new("black")
    sc.world.color = (0, 0, 0)
    # the colour above is the viewport's only: a new world renders its
    # Background node, grey 0.05, which showed through every opening onto
    # nothing (the classic lobby's window) and lit the rooms through it
    if sc.world.node_tree is None:
        sc.world.use_nodes = True
    for n in sc.world.node_tree.nodes:
        if n.type == "BACKGROUND":
            n.inputs["Color"].default_value = (0, 0, 0, 1)
    sc.view_settings.view_transform = "Standard"
    sc.render.resolution_x, sc.render.resolution_y = 1280, 720
    return sc


_images = {}


def image(path, colour=True):
    if path not in _images:
        im = bpy.data.images.load(path)
        im.colorspace_settings.name = "sRGB" if colour else "Non-Color"
        _images[path] = im
    return _images[path]


def _nodes(name):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    for n in list(nt.nodes):
        nt.nodes.remove(n)
    return m, nt


def _multiply(nt, colour_socket, colour):
    mul = nt.nodes.new("ShaderNodeMix")
    mul.data_type = "RGBA"
    mul.blend_type = "MULTIPLY"
    mul.inputs[0].default_value = 1.0
    nt.links.new(colour_socket, mul.inputs[6])
    mul.inputs[7].default_value = list(colour) + [1.0]
    return mul.outputs[2]


def _cut(nt, shader, alpha_socket, ref):
    cut = nt.nodes.new("ShaderNodeMath")
    cut.operation = "GREATER_THAN"
    cut.inputs[1].default_value = ref
    nt.links.new(alpha_socket, cut.inputs[0])
    tr = nt.nodes.new("ShaderNodeBsdfTransparent")
    mix = nt.nodes.new("ShaderNodeMixShader")
    nt.links.new(cut.outputs[0], mix.inputs[0])
    nt.links.new(tr.outputs[0], mix.inputs[1])
    nt.links.new(shader, mix.inputs[2])
    return mix.outputs[0]


def lit_material(spec):
    """The 0.26g material as Cycles sees it: a matte surface of its diffuse
    texture times its colour, cut by its alpha test, and its emission."""
    m, nt = _nodes(spec["name"])
    m["srp_roughness"] = json.dumps(spec.get("roughness"))  # bake.py's gloss page
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    if spec.get("transparent"):  # glass and fx: drawn over the lit frame, seen through here
        nt.links.new(nt.nodes.new("ShaderNodeBsdfTransparent").outputs[0], out.inputs[0])
        return m
    uvn = nt.nodes.new("ShaderNodeUVMap")
    uvn.uv_map = "tex"
    bsdf = nt.nodes.new("ShaderNodeBsdfDiffuse")
    tex = None
    if spec["diffuse"]:
        tex = nt.nodes.new("ShaderNodeTexImage")
        tex.image = image(spec["diffuse"])
        nt.links.new(uvn.outputs[0], tex.inputs[0])
        colour = tex.outputs["Color"]
        if spec.get("blended"):  # the base layer over the blended one by the vertex's weight
            second = nt.nodes.new("ShaderNodeTexImage")
            second.image = image(spec["blended"])
            nt.links.new(uvn.outputs[0], second.inputs[0])
            weight = nt.nodes.new("ShaderNodeAttribute")
            weight.attribute_name = "blend"
            mix = nt.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            nt.links.new(weight.outputs["Fac"], mix.inputs[0])
            nt.links.new(second.outputs["Color"], mix.inputs[6])
            nt.links.new(colour, mix.inputs[7])
            colour = mix.outputs[2]
        nt.links.new(_multiply(nt, colour, spec["colour"]), bsdf.inputs["Color"])
    else:
        bsdf.inputs["Color"].default_value = list(spec["colour"]) + [1.0]
    shader = bsdf.outputs[0]
    e = spec["emission"]
    if e:
        em = nt.nodes.new("ShaderNodeEmission")
        em.inputs["Strength"].default_value = e["multiplier"]
        if e["texture"]:
            et = nt.nodes.new("ShaderNodeTexImage")
            et.image = image(e["texture"])
            nt.links.new(uvn.outputs[0], et.inputs[0])
            nt.links.new(_multiply(nt, et.outputs["Color"], e["colour"]), em.inputs["Color"])
        else:
            em.inputs["Color"].default_value = list(e["colour"]) + [1.0]
        add = nt.nodes.new("ShaderNodeAddShader")
        nt.links.new(shader, add.inputs[0])
        nt.links.new(em.outputs[0], add.inputs[1])
        shader = add.outputs[0]
    if spec["alpha_test"] and tex is not None:
        shader = _cut(nt, shader, tex.outputs["Alpha"], spec["alpha_ref"])
    if not spec.get("two_sided"):  # the game culls a one-sided face's back: light passes it
        geo = nt.nodes.new("ShaderNodeNewGeometry")
        back = nt.nodes.new("ShaderNodeMixShader")
        nt.links.new(geo.outputs["Backfacing"], back.inputs[0])
        nt.links.new(shader, back.inputs[1])
        nt.links.new(nt.nodes.new("ShaderNodeBsdfTransparent").outputs[0], back.inputs[2])
        shader = back.outputs[0]
    nt.links.new(shader, out.inputs[0])
    return m


def page_material(name, png, emission, alpha):
    """A baked page as the game draws it: its texture times the multiplier,
    taking no light; the alpha test at half."""
    m, nt = _nodes(name)
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    uvn = nt.nodes.new("ShaderNodeUVMap")
    uvn.uv_map = "tex"
    tn = nt.nodes.new("ShaderNodeTexImage")
    tn.image = image(png)
    nt.links.new(uvn.outputs[0], tn.inputs[0])
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Strength"].default_value = emission
    nt.links.new(tn.outputs["Color"], em.inputs["Color"])
    shader = em.outputs[0]
    if alpha:
        shader = _cut(nt, shader, tn.outputs["Alpha"], 0.25)  # the templates' alpha_ref
    nt.links.new(shader, out.inputs[0])
    return m


def hybrid_material(name, albedo_png, indirect_png, emission, alpha):
    """A hybrid page as the game draws it: its albedo lit (by direct light
    alone: the caller renders without bounces, as the engine lights) plus its
    indirect page times the multiplier."""
    m, nt = _nodes(name)
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    uvn = nt.nodes.new("ShaderNodeUVMap")
    uvn.uv_map = "tex"
    alb = nt.nodes.new("ShaderNodeTexImage")
    alb.image = image(albedo_png)
    ind = nt.nodes.new("ShaderNodeTexImage")
    ind.image = image(indirect_png)
    for t in (alb, ind):
        nt.links.new(uvn.outputs[0], t.inputs[0])
    bsdf = nt.nodes.new("ShaderNodeBsdfDiffuse")
    nt.links.new(alb.outputs["Color"], bsdf.inputs["Color"])
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Strength"].default_value = emission
    nt.links.new(ind.outputs["Color"], em.inputs["Color"])
    add = nt.nodes.new("ShaderNodeAddShader")
    nt.links.new(bsdf.outputs[0], add.inputs[0])
    nt.links.new(em.outputs[0], add.inputs[1])
    shader = add.outputs[0]
    if alpha:
        shader = _cut(nt, shader, alb.outputs["Alpha"], 0.25)
    nt.links.new(shader, out.inputs[0])
    return m


def new_object(name, pos, nrm, uv, tris, material, matrix=None, weights=None):
    me = bpy.data.meshes.new(name)
    tris = [t for t in tris if len(set(t)) == 3]
    me.from_pydata(pos, [], tris)
    me.update()
    layer = me.uv_layers.new(name="tex")
    loop_vi = [0] * len(me.loops)
    me.loops.foreach_get("vertex_index", loop_vi)
    layer.data.foreach_set("uv", [c for vi in loop_vi for c in uv[vi]])
    me.polygons.foreach_set("use_smooth", [True] * len(me.polygons))
    me.normals_split_custom_set_from_vertices(nrm)
    if weights is not None:  # the blend weight a vertex-blended material reads
        me.attributes.new("blend", "FLOAT", "POINT").data.foreach_set("value", weights)
    me.materials.append(material)
    ob = bpy.data.objects.new(name, me)
    if matrix is not None:
        ob.matrix_world = matrix
    bpy.context.scene.collection.objects.link(ob)
    return ob


class Materials:
    def __init__(self):
        self.made = {}

    def lit(self, spec):
        if spec["name"] not in self.made:
            self.made[spec["name"]] = lit_material(spec)
        return self.made[spec["name"]]


def add_props(scene_json, materials, skip):
    for p in scene_json["props"]:
        if any(k in p["visual"] for k in skip):
            continue
        m = placement(p)
        for s in p["surfaces"]:
            pos, nrm, uv, tris, weights = read_model(s["file"])
            new_object(p["visual"] + "/" + s["name"], pos, nrm, uv, tris, materials.lit(s["material"]), m, weights)


def skipped(name, skip):
    """A light named in `skip`: by its name, its last part, or a prefix
    ending in '/' (a group of the project's lights)."""
    return any(name == k or name.endswith("/" + k) or (k.endswith("/") and name.startswith(k)) for k in skip)


def add_lights(scene_json, skip, match, bulb):
    """Point and spot lights matching the engine's at `match` metres (the
    engine: albedo * I * colour * N.L * (1 - d^2/range^2)^power,
    point_light_accumulator.ps; Cycles: albedo * P / (4 pi^2 d^2) * N.L):
    nearer brighter, further darker. A spot shines along its local -y (the
    engine turns its frame -90 degrees about x: lights_db.cpp fill_light),
    full brightness inside its umbra, none past its penumbra. The sun
    (scene.json's `sun`, the environment's) matches the engine's at every
    distance (Cycles: albedo * S / pi * N.L), the sky is the world's
    radiance (a matte surface under it sends albedo x sky)."""
    tint = scene_json.get("tint", [1.0, 1.0, 1.0])
    warm = lambda c: [c[k] * tint[k] for k in range(3)]
    for l in scene_json["lights"]:
        if skipped(l["name"], skip) or not l["intensity"] or l.get("enabled") is False:
            continue
        # matched at `match`, or half its range for a short light (the
        # engine's window is 0 at the range); with unbaked zones (prep.py),
        # at the nearest zone's centre (a metre at least), so the engine's
        # light on what it draws there meets the bake's at the seam
        r, p = l["range"], l["attenuation_power"]
        centres = [[(z["box"][0][k] + z["box"][1][k]) / 2 for k in range(3)] for z in scene_json.get("zones", [])]
        near = max(1.0, min(math.dist(c, l["position"]) for c in centres)) if centres else match
        at = min(near, 0.5 * r)
        window = max(1.0 - min(at * at / (r * r), 1.0), 1e-3) ** p
        power = 4 * math.pi ** 2 * at * at * l["intensity"] * window
        spot = l.get("light_type") == 1
        light = bpy.data.lights.new(l["name"], "SPOT" if spot else "POINT")
        light.color = warm(l["color"])
        light.energy = power
        light.shadow_soft_size = bulb
        lo = bpy.data.objects.new(l["name"], light)
        lo.location = to_b(l["position"])
        if spot:
            outer = min(l.get("spot_penumbra_angle") or math.pi, math.pi)
            light.spot_size = outer
            light.spot_blend = max(0.0, min(1.0, 1.0 - (l.get("spot_umbra_angle") or outer) / outer))
            lo.rotation_euler = direction(l["rotation"], (0, -1, 0)).to_track_quat("-Z", "Y").to_euler()
        bpy.context.scene.collection.objects.link(lo)
        log(f"{'spot' if spot else 'light'} {l['name']}: {power:.0f} W (engine {l['intensity']} x {window:.2f} at {at} m)")
    sun = scene_json.get("sun")
    if sun and sun["intensity"] and not skipped("environment_sun", skip):
        light = bpy.data.lights.new("environment_sun", "SUN")
        light.color = warm(sun["color"])
        light.energy = math.pi * sun["intensity"]
        light.angle = math.radians(1.0)
        lo = bpy.data.objects.new("environment_sun", light)
        lo.rotation_euler = mathutils.Vector(to_b(sun["to_sun"])).to_track_quat("Z", "Y").to_euler()
        bpy.context.scene.collection.objects.link(lo)
        log(f"sun: {light.energy:.2f} W/m2 (engine {sun['intensity']}), towards {sun['to_sun']}")
    sky = scene_json.get("sky")
    if sky and any(sky):
        world = bpy.context.scene.world
        if world.node_tree is None:
            world.use_nodes = True
        background = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")
        background.inputs["Color"].default_value = warm(sky) + [1.0]
        background.inputs["Strength"].default_value = 1.0
        log(f"sky: radiance {sky}")


# The views rendered (0.26g lobby frame, the character at the origin): the
# scene's own (the lobby's bake section), else the stock camera's, up at the
# bunker store room's pendant (the owner's screenshot, 2026-11-01), and across
# the room to the vaults.
def views(scene_json):
    cp = scene_json["camera"]["position"]
    cd = scene_json["camera"]["direction"]
    stock = (cp, [cp[i] + cd[i] for i in range(3)])
    if scene_json.get("views"):
        return {"stock": stock, **{k: tuple(v) for k, v in scene_json["views"].items()}}
    return {
        "stock": (cp, [cp[i] + cd[i] for i in range(3)]),
        "lamp": ((0.15, 1.35, -1.2), (-0.25, 2.75, 3.2)),
        "room": ((-1.3, 1.65, -2.2), (2.5, 1.0, 3.0)),
    }


def render(directory, name, view, fov, samples, exposure):
    sc = bpy.context.scene
    cam = bpy.data.cameras.new(name)
    cam.sensor_fit = "HORIZONTAL"
    cam.angle = math.radians(fov)
    cam.clip_start = 0.05
    co = bpy.data.objects.new(name, cam)
    eye, target = view
    co.location = to_b(eye)
    co.rotation_euler = (mathutils.Vector(to_b(target)) - mathutils.Vector(to_b(eye))).to_track_quat("-Z", "Y").to_euler()
    sc.collection.objects.link(co)
    sc.camera = co
    sc.view_settings.exposure = exposure
    sc.cycles.samples = samples
    sc.render.filepath = os.path.join(directory, name)
    t = time.time()
    bpy.ops.render.render(write_still=True)
    log(f"render {name}: {time.time() - t:.0f} s")


def add_decals(scene_json):
    """The projected decals painted into the albedo of the matte surfaces
    their boxes reach (the engine writes them into the g-buffer's albedo, so
    a bake must hold them: on a baked page's black albedo the engine would
    light them on their own). A decal's box is half-extents (width, height,
    depth) about its position along its local axes; its texture spans the
    width and the height."""
    def bounds(points):
        return [min(p[k] for p in points) for k in range(3)], [max(p[k] for p in points) for k in range(3)]

    for d in scene_json.get("decals", []):
        if not d["diffuse"] or not d["alpha"]:
            continue
        box = bpy.data.objects.new(d["name"], None)
        box.matrix_world = placement({"position": d["position"], "rotation": d["rotation"], "scale": d["extent"]})
        bpy.context.scene.collection.objects.link(box)
        lo, hi = bounds([box.matrix_world @ mathutils.Vector(c) for c in
                         [(x, y, z) for x in (-1, 1) for y in (-1, 1) for z in (-1, 1)]])
        reached = set()
        for ob in bpy.context.scene.objects:
            if ob.type != "MESH":
                continue
            olo, ohi = bounds([ob.matrix_world @ mathutils.Vector(c) for c in ob.bound_box])
            if all(olo[k] <= hi[k] and lo[k] <= ohi[k] for k in range(3)):
                reached.update(m for m in ob.data.materials if m)
        for m in reached:
            nt = m.node_tree
            bsdf = next((n for n in nt.nodes if n.type == "BSDF_DIFFUSE"), None)
            if bsdf is None:
                continue
            colour = bsdf.inputs["Color"]
            below = colour.links[0].from_socket if colour.links else None
            coords = nt.nodes.new("ShaderNodeTexCoord")
            coords.object = box
            xyz = nt.nodes.new("ShaderNodeSeparateXYZ")
            nt.links.new(coords.outputs["Object"], xyz.inputs[0])
            # Blender's local (x, y, z) is the decal's (x, depth, height)
            uv = nt.nodes.new("ShaderNodeCombineXYZ")
            nt.links.new(xyz.outputs[0], uv.inputs[0])
            nt.links.new(xyz.outputs[2], uv.inputs[1])
            scaled = nt.nodes.new("ShaderNodeVectorMath")
            scaled.operation = "MULTIPLY_ADD"
            nt.links.new(uv.outputs[0], scaled.inputs[0])
            scaled.inputs[1].default_value = (0.5, 0.5, 0.0)
            scaled.inputs[2].default_value = (0.5, 0.5, 0.0)
            tex = nt.nodes.new("ShaderNodeTexImage")
            tex.image = image(d["diffuse"])
            tex.extension = "CLIP"
            nt.links.new(scaled.outputs[0], tex.inputs[0])
            inside = nt.nodes.new("ShaderNodeVectorMath")  # max(|x|, |y|, |z|) <= 1
            inside.operation = "ABSOLUTE"
            nt.links.new(coords.outputs["Object"], inside.inputs[0])
            parts = nt.nodes.new("ShaderNodeSeparateXYZ")
            nt.links.new(inside.outputs[0], parts.inputs[0])
            top = parts.outputs[0]
            for k in (1, 2):
                mx = nt.nodes.new("ShaderNodeMath")
                mx.operation = "MAXIMUM"
                nt.links.new(top, mx.inputs[0])
                nt.links.new(parts.outputs[k], mx.inputs[1])
                top = mx.outputs[0]
            mask = nt.nodes.new("ShaderNodeMath")
            mask.operation = "LESS_THAN"
            mask.inputs[1].default_value = 1.0
            nt.links.new(top, mask.inputs[0])
            fac = nt.nodes.new("ShaderNodeMath")
            fac.operation = "MULTIPLY"
            nt.links.new(tex.outputs["Alpha"], fac.inputs[0])
            nt.links.new(mask.outputs[0], fac.inputs[1])
            weight = nt.nodes.new("ShaderNodeMath")
            weight.operation = "MULTIPLY"
            nt.links.new(fac.outputs[0], weight.inputs[0])
            weight.inputs[1].default_value = d["alpha"]
            mix = nt.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            nt.links.new(weight.outputs[0], mix.inputs[0])
            if below is not None:
                nt.links.new(below, mix.inputs[6])
            else:
                mix.inputs[6].default_value = colour.default_value
            nt.links.new(_multiply(nt, tex.outputs["Color"], d["colour"]), mix.inputs[7])
            nt.links.new(mix.outputs[2], colour)
        log(f"decal {d['name']}: on {len(reached)} materials")
