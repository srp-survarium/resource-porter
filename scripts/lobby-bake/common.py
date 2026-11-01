"""What bake.py and show.py share: 0.26g models in Blender, the materials,
the lights in 0.26g's units, the views."""
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
    """A converted_model in Blender's frame: positions, normals, uvs, triangles."""
    d = open(path, "rb").read()
    tag, _, n = struct.unpack_from("<III", d, 0)
    assert tag == 3, path
    pos, nrm, uv = [], [], []
    for i in range(n):
        x, y, z, pn, _, _, u, v = struct.unpack_from("<3f3I2H", d, 12 + 28 * i)
        pos.append(to_b((x, y, z)))
        nrm.append(to_b(unpack_basis(pn)))
        uv.append((half(u), 1.0 - half(v)))
    o = 12 + 28 * n
    tag, _, m = struct.unpack_from("<III", d, o)
    assert tag == 4, path
    idx = struct.unpack_from(f"<{m}H", d, o + 12)
    return pos, nrm, uv, [(idx[i], idx[i + 2], idx[i + 1]) for i in range(0, m, 3)]


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
def new_scene(threads):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    sc.render.threads_mode = "FIXED"
    sc.render.threads = threads
    sc.cycles.use_denoising = True
    sc.cycles.max_bounces = 8
    sc.cycles.diffuse_bounces = 6
    sc.cycles.glossy_bounces = 2
    sc.cycles.transparent_max_bounces = 16
    sc.world = bpy.data.worlds.new("black")
    sc.world.color = (0, 0, 0)
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
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    uvn = nt.nodes.new("ShaderNodeUVMap")
    uvn.uv_map = "tex"
    bsdf = nt.nodes.new("ShaderNodeBsdfDiffuse")
    tex = None
    if spec["diffuse"]:
        tex = nt.nodes.new("ShaderNodeTexImage")
        tex.image = image(spec["diffuse"])
        nt.links.new(uvn.outputs[0], tex.inputs[0])
        nt.links.new(_multiply(nt, tex.outputs["Color"], spec["colour"]), bsdf.inputs["Color"])
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


def new_object(name, pos, nrm, uv, tris, material, matrix=None):
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
        if tuple(p["rotation"]) != (0.0, 0.0, 0.0):
            log("prop with a rotation, placed unrotated:", p["visual"])
        m = mathutils.Matrix.Translation(to_b(p["position"]))
        for s in p["surfaces"]:
            pos, nrm, uv, tris = read_model(s["file"])
            new_object(p["visual"] + "/" + s["name"], pos, nrm, uv, tris, materials.lit(s["material"]), m)


def add_lights(scene_json, skip, match, bulb):
    """Point lights matching the engine's at `match` metres (the engine: albedo
    * I * colour * N.L * (1 - d^2/range^2)^power, point_light_accumulator.ps;
    Cycles: albedo * P / (4 pi^2 d^2) * N.L): nearer brighter, further darker."""
    for l in scene_json["lights"]:
        if l["name"] in skip or not l["intensity"]:
            continue
        r, p = l["range"], l["attenuation_power"]
        window = max(1.0 - min(match * match / (r * r), 1.0), 1e-3) ** p
        power = 4 * math.pi ** 2 * match * match * l["intensity"] * window
        light = bpy.data.lights.new(l["name"], "POINT")
        light.color = l["color"]
        light.energy = power
        light.shadow_soft_size = bulb
        lo = bpy.data.objects.new(l["name"], light)
        lo.location = to_b(l["position"])
        bpy.context.scene.collection.objects.link(lo)
        log(f"light {l['name']}: {power:.0f} W (engine {l['intensity']} x {window:.2f} at {match} m)")


# The views rendered (0.26g lobby frame, the character at the origin): the stock
# camera's, up at the store room's pendant (the owner's screenshot, 2026-11-01),
# and across the room to the vaults.
def views(scene_json):
    cp = scene_json["camera"]["position"]
    cd = scene_json["camera"]["direction"]
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
