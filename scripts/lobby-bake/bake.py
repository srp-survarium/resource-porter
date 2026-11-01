"""bake.py: path-trace a lobby's light into unique-uv pages (Blender Cycles, CPU).

  blender -b --factory-startup --python bake.py -- <scene dir> [options]

<scene dir>/scene.json is prep.py's. Writes into <scene dir>:
  pages/page_NN.exr        the baked radiance (linear, 0.26g light units)
  pages/page_NN.png        the same over the emission multiplier, sRGB-encoded
                           (alpha: the alpha test's cut)
  baked/page_NN_K          the page's surfaces as 0.26g converted_models (uv: the page's)
  baked.json               the surfaces, pages, multiplier and the merged surfaces
                           baked (their sha256), for the porter's `bake`
  renders/traced-*.png     the views path-traced with the lit materials

The merged model's surfaces are cut in chunks (near and far of the character,
then halved while a page cannot hold one), each given its own islands (smart
project), and the chunks laid on pages by the texels they need; a page is baked
COMBINED with diffuse direct + indirect + colour + emission: the light a matte
surface sends, the same from wherever it is seen.
"""
import argparse
import json
import math
import os
import sys
import time

import bpy
import mathutils
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (Materials, add_decals, add_lights, add_props, log, new_object, new_scene, read_placed,  # noqa: E402
                    render, to_b, views, write_model)

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
ap = argparse.ArgumentParser()
ap.add_argument("scene")
ap.add_argument("--near", type=float, default=192.0, help="texels a metre within --radius of the character's eye")
ap.add_argument("--far", type=float, default=32.0, help="texels a metre beyond it")
ap.add_argument("--radius", type=float, default=8.0)
ap.add_argument("--outside", type=float, default=4.0,
                help="texels a metre outside the scene's box (the faces no view of the lobby reaches)")
ap.add_argument("--page", type=int, default=2048, help="the largest page")
ap.add_argument("--gpu", action="store_true", help="Cycles on the GPU (OptiX, else CUDA): Blender built with them")
ap.add_argument("--samples", type=int, default=128)
ap.add_argument("--render-samples", type=int, default=256)
ap.add_argument("--threads", type=int, default=12)
ap.add_argument("--match", type=float, default=2.0, help="distance where a light matches the engine's")
ap.add_argument("--bulb", type=float, default=0.03, help="light source radius, m")
ap.add_argument("--skip-lights", default="sun,character_fill")
ap.add_argument("--skip-props", default="_on")
ap.add_argument("--skip-materials", default="/fx/", help="surfaces left as they are, by material")
ap.add_argument("--exposure", type=float, default=0.0)
ap.add_argument("--fov", type=float, default=70.0, help="horizontal field of view, degrees")
ap.add_argument("--only-render", action="store_true", help="the traced views only, no bake")
ap.add_argument("--probe-samples", type=int, default=512)
ap.add_argument("--probe-at", default="0,1.2,0", help="where a probe without its own trace_at (prep.py) is traced (lobby frame): the character's chest; "
                "a probe object's own place can be inside a prop (the bunker's room probe is in the gas-mask crates)")
ap.add_argument("--hybrid", action="store_true",
                help="pages of albedo (lit by the project's lights in the game) and of indirect light only "
                     "(emitted, at a quarter of the size): the lamps' direct light, shadows and every "
                     "dynamic light stay the engine's")
ap.add_argument("--only-probes", action="store_true", help="the probes only (no renders, no bake)")
ap.add_argument("--only-pack", action="store_true", help="lay out the pages and stop (no renders, no bake)")
args = ap.parse_args(argv)
D = args.scene
scene_json = json.load(open(os.path.join(D, "scene.json")))
args.hybrid = args.hybrid or bool(scene_json.get("hybrid"))  # a scene spec's `hybrid`
for sub in ("pages", "baked", "renders", "probes"):
    os.makedirs(os.path.join(D, sub), exist_ok=True)

sc = new_scene(args.threads, args.gpu)
materials = Materials()


def centroid(pos, t):
    return (mathutils.Vector(pos[t[0]]) + mathutils.Vector(pos[t[1]]) + mathutils.Vector(pos[t[2]])) / 3


def tri_area(pos, t):
    a, b, c = (mathutils.Vector(pos[i]) for i in t)
    return 0.5 * (b - a).cross(c - a).length


# the orbit's centre (the scene's `eye`, else the character's chest at the
# origin) and the box the lobby is seen in (the scene's `box`), Blender frame
EYE = mathutils.Vector(to_b(scene_json.get("eye", (0.0, 1.2, 0.0))))
BOX = [mathutils.Vector(to_b(c)) for c in scene_json["box"]] if scene_json.get("box") else None


def density_of(pos, t):
    """A triangle's texels a metre: the fewest outside the scene's box (by its
    centroid), the most when any corner is within --radius of the eye."""
    c = centroid(pos, t)
    if BOX and not all(min(BOX[0][k], BOX[1][k]) <= c[k] <= max(BOX[0][k], BOX[1][k]) for k in range(3)):
        return args.outside
    return args.near if min((mathutils.Vector(pos[i]) - EYE).length for i in t) < args.radius else args.far



CAPACITY = args.page * args.page * 0.5
skip_m = [s for s in args.skip_materials.split(",") if s]
chunks = []  # (object, density, area, alpha, two_sided)
kept = []    # surfaces left as they are, still in the scene
for s in scene_json["surfaces"]:
    pos, nrm, uv, tris, weights = read_placed(s)
    mat = materials.lit(s["material"])
    if s["material"].get("transparent") or any(k in s["material"]["name"] for k in skip_m):
        new_object(s["name"], pos, nrm, uv, tris, mat, weights=weights)
        kept.append(s["name"])
        continue
    groups = {}
    for t in tris:
        # near when any corner is (a large wall triangle by the character is near)
        groups.setdefault(density_of(pos, t), []).append(t)
    work = list(groups.items())
    while work:
        density, ts = work.pop()
        area = sum(tri_area(pos, t) for t in ts)
        cs = [centroid(pos, t) for t in ts]
        extent = [max(c[k] for c in cs) - min(c[k] for c in cs) for k in range(3)]
        # halved while a page cannot hold it, or while it is longer than a
        # third of a page (one long island, a pipe or a vault's strip, would
        # shrink every other island on its page)
        if (area * density * density > CAPACITY or max(extent) > math.sqrt(CAPACITY) / density / 3) and len(ts) > 1:
            axis = max(range(3), key=lambda k: extent[k])
            order = sorted(range(len(ts)), key=lambda i: cs[i][axis])
            mid = len(order) // 2
            work.append((density, [ts[i] for i in order[:mid]]))
            work.append((density, [ts[i] for i in order[mid:]]))
            continue
        used = sorted({i for t in ts for i in t})
        remap = {v: k for k, v in enumerate(used)}
        ob = new_object(f"{s['name']}_{int(density)}_{len(chunks)}", [pos[i] for i in used],
                        [nrm[i] for i in used], [uv[i] for i in used],
                        [tuple(remap[i] for i in t) for t in ts], mat,
                        weights=[weights[i] for i in used] if weights is not None else None)
        chunks.append((ob, density, area, s["material"]["alpha_test"], s["material"]["two_sided"]))
log(f"{len(chunks)} chunks to bake, {len(kept)} surfaces kept;",
    ", ".join(f"{sum(c[2] for c in chunks if c[1] == v):.0f} m2 at {v:g} texels/m"
              for v in sorted({c[1] for c in chunks}, reverse=True)))
add_props(scene_json, materials, [s for s in args.skip_props.split(",") if s])
add_decals(scene_json)  # into the pages: the baked pack leaves the decals out
add_lights(scene_json, [s for s in args.skip_lights.split(",") if s] + scene_json.get("skip_lights", []), args.match, args.bulb)

for name, view in views(scene_json).items() if not (args.only_pack or args.only_probes) else ():
    render(os.path.join(D, "renders"), f"traced-{name}.png", view, args.fov, args.render_samples, args.exposure)
if args.only_render:
    sys.exit(0)

# --- the probes ----------------------------------------------------------------
# Each environment probe's six faces traced at the character, 0.26g's texel
# order (probe_bake::direction: forward, right, up of a face's rows), the
# lamps themselves unseen (the project's lights light the character directly;
# the probe gives it what the room sends back).
PROBE_SIZE = 128
FACES = [((1, 0, 0), (0, 0, -1), (0, 1, 0)), ((-1, 0, 0), (0, 0, 1), (0, 1, 0)),
         ((0, 1, 0), (1, 0, 0), (0, 0, -1)), ((0, -1, 0), (1, 0, 0), (0, 0, 1)),
         ((0, 0, 1), (1, 0, 0), (0, 1, 0)), ((0, 0, -1), (-1, 0, 0), (0, 1, 0))]
probes_made = []
if scene_json.get("probes") and not args.only_pack:
    from common import to_b
    lights = [o for o in sc.objects if o.type == "LIGHT"]
    for o in lights:
        o.visible_camera = False
    size_x, size_y = sc.render.resolution_x, sc.render.resolution_y
    sc.render.resolution_x = sc.render.resolution_y = PROBE_SIZE
    sc.render.image_settings.file_format = "OPEN_EXR"
    sc.render.image_settings.color_depth = "32"
    sc.cycles.samples = args.probe_samples
    cam = bpy.data.cameras.new("probe")
    cam.angle = math.radians(90.0)
    cam.clip_start = 0.02
    co = bpy.data.objects.new("probe", cam)
    sc.collection.objects.link(co)
    sc.camera = co
    for probe in scene_json["probes"]:
        at = probe.get("trace_at") or [float(v) for v in args.probe_at.split(",")]
        stem = probe["name"].replace("/", "__")
        out = np.zeros((6, PROBE_SIZE, PROBE_SIZE, 3), dtype=np.float32)
        for f, (fwd, right, up) in enumerate(FACES):
            x, y, z = (mathutils.Vector(to_b(v)) for v in (right, up, fwd))
            mirrored = mathutils.Matrix((x, y, -z)).transposed().determinant() < 0
            if mirrored:
                x = -x
            co.matrix_world = mathutils.Matrix.Translation(to_b(at)) @ \
                mathutils.Matrix((x, y, -z)).transposed().to_4x4()
            path = os.path.join(D, "probes", f"{stem}_{f}.exr")
            sc.render.filepath = path
            bpy.ops.render.render(write_still=True)
            im = bpy.data.images.load(path)
            px = np.empty(PROBE_SIZE * PROBE_SIZE * 4, dtype=np.float32)
            im.pixels.foreach_get(px)
            face = px.reshape(PROBE_SIZE, PROBE_SIZE, 4)[::-1, :, :3]  # Blender's rows run up
            out[f] = face[:, ::-1] if mirrored else face
            bpy.data.images.remove(im)
        name = f"probes/{stem}.f32"
        out.astype("<f4").tofile(os.path.join(D, name))
        probes_made.append({"object": probe["name"], "file": name, "size": PROBE_SIZE})
        log(f"probe {probe['name']}: traced at {at}, mean radiance {out.mean(axis=(0, 1, 2))}")
    for o in lights:
        o.visible_camera = True
    sc.render.resolution_x, sc.render.resolution_y = size_x, size_y
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_depth = "8"
if args.only_probes:
    sys.exit(0)

# --- pages ---------------------------------------------------------------------
pages = []
# a page's material is alpha-tested or not, two-sided or not: its chunks' own
for ob, density, area, alpha, two_sided in sorted(chunks, key=lambda c: -c[2] * c[1] * c[1]):
    need = area * density * density
    for page in pages:
        if (page["alpha"], page["two_sided"], page["density"]) == (alpha, two_sided, density) \
                and page["need"] + need <= CAPACITY:
            break
    else:
        page = {"objects": [], "need": 0.0, "alpha": alpha, "two_sided": two_sided, "density": density}
        pages.append(page)
    page["objects"].append(ob)
    page["need"] += need
for page in pages:  # each the smallest square that holds what it needs, 1024 at least (margins)
    side = 1024
    while side * side * 0.5 < page["need"] and side < args.page:
        side *= 2
    page["size"] = side
log(f"{len(pages)} pages:", ", ".join(f"{len(p['objects'])} chunks {p['need'] / 1e6:.1f} Mtexels {p['size']} px" for p in pages))


def select(objects):
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.select_all(action="DESELECT")
    for o in objects:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]


# A chunk's islands: a prop's are its model's own (laid out on its atlas;
# smart project cut the 14 000-face valves and the debris into thousands of
# islands, each with its margin, and filled pages to 1 %); the architecture's,
# few large faces whose own uvs tile and stretch (streaks on the pillars in
# the game), smart project's.
DENSE = 300.0  # faces a square metre above which a chunk is a prop
for ob, _, area, *_ in chunks:
    ob.data.uv_layers.active = ob.data.uv_layers.new(name="bake", do_init=True)
    if len(ob.data.polygons) / max(area, 1e-6) < DENSE:
        select([ob])
        bpy.ops.object.mode_set(mode="EDIT")
        bpy.ops.mesh.select_all(action="SELECT")
        bpy.ops.uv.smart_project(angle_limit=math.radians(66.0), island_margin=0.0, area_weight=0.0,
                                 correct_aspect=True, scale_to_bounds=False)
        bpy.ops.object.mode_set(mode="OBJECT")
for k, page in enumerate(pages):
    margin = 4.0 / page["size"]
    # every chunk's islands scaled to its metres
    for ob in page["objects"]:
        me = ob.data
        uvs = me.uv_layers["bake"].data
        uv_area = sum(abs(sum((uvs[li[j]].uv.x - uvs[li[0]].uv.x) * (uvs[li[j + 1]].uv.y - uvs[li[0]].uv.y)
                                - (uvs[li[j + 1]].uv.x - uvs[li[0]].uv.x) * (uvs[li[j]].uv.y - uvs[li[0]].uv.y)
                                for j in range(1, len(li) - 1))) * 0.5
                      for li in (list(p.loop_indices) for p in me.polygons))
        area = sum(p.area for p in me.polygons)
        scale = math.sqrt(area / uv_area) if uv_area > 0 else 1.0
        flat = [0.0] * (2 * len(uvs))
        uvs.foreach_get("uv", flat)
        uvs.foreach_set("uv", [c * scale for c in flat])
    select(page["objects"])
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.select_all(action="SELECT")
    bpy.ops.uv.pack_islands(udim_source="ACTIVE_UDIM", rotate=True, scale=True, margin_method="FRACTION",
                            margin=margin, shape_method="CONCAVE")
    bpy.ops.object.mode_set(mode="OBJECT")
    # how much of the page the islands fill, and that they keep inside it
    used, lo, hi = 0.0, [1.0, 1.0], [0.0, 0.0]
    for ob in page["objects"]:
        uvs = ob.data.uv_layers["bake"].data
        for poly in ob.data.polygons:
            c = [uvs[i].uv for i in poly.loop_indices]
            for u in c:
                lo = [min(lo[0], u[0]), min(lo[1], u[1])]
                hi = [max(hi[0], u[0]), max(hi[1], u[1])]
            used += 0.5 * abs(sum((c[j].x - c[0].x) * (c[j + 1].y - c[0].y)
                                  - (c[j + 1].x - c[0].x) * (c[j].y - c[0].y) for j in range(1, len(c) - 1)))
    page["fill"] = abs(used)
    log(f"page {k}: {len(page['objects'])} chunks packed, filling {abs(used):.0%}, uv {lo[0]:.3f}..{hi[0]:.3f} x {lo[1]:.3f}..{hi[1]:.3f}")
if args.only_pack:
    sys.exit(0)

# A page's chunks joined in one object: Cycles bakes the selected objects one
# by one, each a scene update (the bunker's page of 119 chunks baked six
# times slower than one of 12 holding as many texels).
for page in pages:
    page["chunks"] = len(page["objects"])
    if len(page["objects"]) > 1:
        select(page["objects"])
        bpy.ops.object.join()
        page["objects"] = [bpy.context.view_layer.objects.active]

# --- bake ----------------------------------------------------------------------
sc.cycles.samples = args.samples
sc.render.bake.margin = 8
sc.render.bake.margin_type = "EXTEND"
copies = {}
for k, page in enumerate(pages):
    img = bpy.data.images.new(f"page_{k:02d}", page["size"], page["size"], float_buffer=True, alpha=True)
    page["image"] = img
    for ob, slot in ((ob, slot) for ob in page["objects"] for slot in range(len(ob.data.materials))):
        src = ob.data.materials[slot]  # one material copy a page, the page its target
        if (src.name, k) not in copies:
            m = src.copy()
            tn = m.node_tree.nodes.new("ShaderNodeTexImage")
            tn.image = img
            uvb = m.node_tree.nodes.new("ShaderNodeUVMap")
            uvb.uv_map = "bake"
            m.node_tree.links.new(uvb.outputs[0], tn.inputs[0])
            m.node_tree.nodes.active = tn
            copies[(src.name, k)] = m
        ob.data.materials[slot] = copies[(src.name, k)]
    select(page["objects"])
    t = time.time()
    if args.hybrid:
        # the albedo, then the light that reached the surface after a bounce
        # (times the albedo): the live lamps' direct light is the engine's
        sc.cycles.samples = 4
        bpy.ops.object.bake(type="DIFFUSE", pass_filter={"COLOR"}, margin=8, use_clear=True,
                            target="IMAGE_TEXTURES")
        small = max(page["size"] // 4, 128)
        ind = bpy.data.images.new(f"indirect_{k:02d}", small, small, float_buffer=True, alpha=True)
        page["indirect_image"] = ind
        for m in {m for ob in page["objects"] for m in ob.data.materials}:
            next(n for n in m.node_tree.nodes if n.type == "TEX_IMAGE" and n.image is img).image = ind
        sc.cycles.samples = args.samples
        bpy.ops.object.bake(type="DIFFUSE", pass_filter={"INDIRECT", "COLOR"}, margin=4, use_clear=True,
                            target="IMAGE_TEXTURES")
        for m in {m for ob in page["objects"] for m in ob.data.materials}:
            next(n for n in m.node_tree.nodes if n.type == "TEX_IMAGE" and n.image is ind).image = img
    else:
        bpy.ops.object.bake(type="COMBINED", pass_filter={"DIRECT", "INDIRECT", "DIFFUSE", "EMIT"},
                            margin=8, use_clear=True, target="IMAGE_TEXTURES")
    img.filepath_raw = os.path.join(D, "pages", f"page_{k:02d}.exr")
    img.file_format = "OPEN_EXR"
    img.save()
    log(f"page {k}: baked in {time.time() - t:.0f} s")

# the alpha test's cut, baked as emission into a second image
sc.cycles.samples = 4
for k, page in enumerate(pages):
    if not page["alpha"]:
        continue
    aimg = bpy.data.images.new(f"alpha_{k:02d}", page["size"], page["size"], float_buffer=False)
    page["alpha_image"] = aimg
    saved = {}
    for m in {m for ob in page["objects"] for m in ob.data.materials}:
        nt = m.node_tree
        tex = next(n for n in nt.nodes if n.type == "TEX_IMAGE" and n.image is not page["image"])
        em = nt.nodes.new("ShaderNodeEmission")
        nt.links.new(tex.outputs["Alpha"], em.inputs["Color"])
        out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL")
        saved[m] = out.inputs[0].links[0].from_socket
        nt.links.new(em.outputs[0], out.inputs[0])
        target = next(n for n in nt.nodes if n.type == "TEX_IMAGE" and n.image is page["image"])
        target.image = aimg
        nt.nodes.active = target
    select(page["objects"])
    bpy.ops.object.bake(type="EMIT", margin=8, use_clear=True, target="IMAGE_TEXTURES")
    for m, socket in saved.items():
        nt = m.node_tree
        nt.links.new(socket, next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL").inputs[0])
        next(n for n in nt.nodes if n.type == "TEX_IMAGE" and n.image is aimg).image = page["image"]
    log(f"page {k}: alpha baked")

# --- the multiplier and the 8-bit pages ----------------------------------------
# One multiplier for every page: the 99.8th percentile of the baked radiance is
# the texture's white (the hot spots by the bulbs clip above it).
def pixels(img):
    px = np.empty(img.size[0] * img.size[1] * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    return px.reshape(-1, 4)


values = []
for page in pages:
    page["px"] = pixels(page["image"])
    emitted = pixels(page["indirect_image"]) if args.hybrid else page["px"]
    values.append(emitted[emitted[:, 3] > 0.5, :3].max(axis=1))
allv = np.concatenate(values)
emission = float(np.percentile(allv, 99.8))
log(f"emission multiplier {emission:.3f} (radiance median {np.median(allv):.3f}, max {allv.max():.2f})")


def srgb(x):
    x = np.clip(x, 0.0, 1.0)
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - 0.055)


def write_png(name, size, rgb, a):
    out = bpy.data.images.new(name, size, size, alpha=True)
    out.colorspace_settings.name = "Non-Color"  # written as they are: sRGB-encoded already
    out.pixels.foreach_set(np.concatenate([rgb, a[:, None]], axis=1).astype(np.float32).ravel())
    out.filepath_raw = os.path.join(D, "pages", f"{name}.png")
    out.file_format = "PNG"
    out.save()


for k, page in enumerate(pages):
    a = pixels(page["alpha_image"])[:, 0] if page.get("alpha_image") else np.ones(len(page["px"]), dtype=np.float32)
    if args.hybrid:
        write_png(f"page_{k:02d}", page["size"], srgb(page["px"][:, :3]), a)
        ind = pixels(page["indirect_image"])
        write_png(f"page_{k:02d}_indirect", page["indirect_image"].size[0], srgb(ind[:, :3] / emission),
                  np.ones(len(ind), dtype=np.float32))
    else:
        write_png(f"page_{k:02d}", page["size"], srgb(page["px"][:, :3] / emission), a)
if args.hybrid:  # the template's normal and roughness maps, flat (they would tile over the pages' layout)
    write_png("flat_normal", 4, np.tile([[0.5, 0.5, 1.0]], (16, 1)), np.ones(16))
    write_png("flat_roughness", 4, np.tile([[0.85, 0.85, 0.85]], (16, 1)), np.ones(16))

# --- the baked surfaces as 0.26g models ----------------------------------------
LIMIT = 35000  # stk2_lobby_026's MAX_STATIC_SURFACE_VERTICES, less a little
baked = {"emission": emission, "pages": [], "surfaces": []}
if args.hybrid:
    baked["flat"] = {"normal": "pages/flat_normal.png", "roughness": "pages/flat_roughness.png"}
for k, page in enumerate(pages):
    pos, nrm, uv, tris = [], [], [], []
    for ob in page["objects"]:
        me = ob.data
        layer = me.uv_layers["bake"].data
        normals = me.corner_normals
        index = {}  # one vertex a corner (position, normal, page uv), welded where all three agree
        for poly in me.polygons:
            corners = []
            for li in poly.loop_indices:
                co = me.vertices[me.loops[li].vertex_index].co
                n = tuple(normals[li].vector)
                u = tuple(layer[li].uv)
                key = (tuple(round(c, 6) for c in co), tuple(round(c, 3) for c in n), tuple(round(c, 7) for c in u))
                if key not in index:
                    pos.append(tuple(co))
                    nrm.append(n)
                    uv.append(u)
                    index[key] = len(pos) - 1
                corners.append(index[key])
            for j in range(1, len(corners) - 1):
                tris.append((corners[0], corners[j], corners[j + 1]))
    piece, start = 0, 0
    while start < len(tris):  # pieces under the vertex limit, whole triangles
        used, ts = {}, []
        while start < len(tris):
            new = [i for i in dict.fromkeys(tris[start]) if i not in used]
            if len(used) + len(new) > LIMIT:
                break
            for i in new:
                used[i] = len(used)
            ts.append(tuple(used[i] for i in tris[start]))
            start += 1
        order = sorted(used, key=used.get)
        name = f"page_{k:02d}_{piece}"
        write_model(os.path.join(D, "baked", name), [pos[i] for i in order], [nrm[i] for i in order],
                    [uv[i] for i in order], ts)
        baked["surfaces"].append({"name": name, "file": f"baked/{name}", "page": f"page_{k:02d}",
                                  "vertices": len(order), "triangles": len(ts)})
        piece += 1
    kind = {"albedo": f"pages/page_{k:02d}.png", "indirect": f"pages/page_{k:02d}_indirect.png"} if args.hybrid \
        else {"png": f"pages/page_{k:02d}.png"}
    baked["pages"].append({"name": f"page_{k:02d}", **kind, "alpha": page["alpha"], "two_sided": page["two_sided"],
                           "size": page["size"], "density": page["density"], "texels": page["need"],
                           "chunks": page["chunks"]})
baked["kept"] = kept
baked["sources"] = {s["name"]: s["sha256"] for s in scene_json["surfaces"]}
baked["probes"] = probes_made
json.dump(baked, open(os.path.join(D, "baked.json"), "w"), indent=1)
log(f"baked.json: {len(baked['surfaces'])} surfaces on {len(pages)} pages")
