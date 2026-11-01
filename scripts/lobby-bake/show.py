"""show.py: the views as the game would draw a bake, from the files the porter takes.

  blender -b --factory-startup --python show.py -- <scene dir> [--fov 70] [--samples 64]

The baked surfaces (baked.json's converted_models and 8-bit pages) emit their
page times the multiplier and take no light; the merged surfaces left unbaked
and the native props keep their lit materials under the bake's lights.
Writes <scene dir>/renders/baked-<view>.png.

A hybrid bake (albedo and indirect pages) is drawn as the engine would: the
albedo lit by direct light alone (no bounce) plus the indirect page;
hybrid-<view>.png. --engine draws what the engine lights as it would
(roughly): the lights' direct light only, and an ambient of the probe traced
at the first unbaked zone (its mean radiance times the albedo), no bounce;
the baked surfaces as before. Its renders, engine-<view>.png, show the seam
at the unbaked zones.
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np  # noqa: E402

from common import (Materials, _multiply, add_lights, add_props, hybrid_material, log, new_object, new_scene,  # noqa: E402
                    page_material, read_model, read_placed, render, views)

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
ap = argparse.ArgumentParser()
ap.add_argument("scene")
ap.add_argument("--gpu", action="store_true")
ap.add_argument("--fov", type=float, default=70.0)
ap.add_argument("--samples", type=int, default=64)
ap.add_argument("--threads", type=int, default=12)
ap.add_argument("--match", type=float, default=2.0)
ap.add_argument("--bulb", type=float, default=0.03)
ap.add_argument("--skip-lights", default="sun,character_fill")
ap.add_argument("--skip-props", default="_on")
ap.add_argument("--exposure", type=float, default=0.0)
ap.add_argument("--views", default="", help="comma-separated; all by default")
ap.add_argument("--engine", action="store_true", help="what the engine lights: direct light and the probe's ambient")
args = ap.parse_args(argv)
D = args.scene
scene_json = json.load(open(os.path.join(D, "scene.json")))
baked = json.load(open(os.path.join(D, "baked.json")))

new_scene(args.threads, args.gpu)
materials = Materials()
hybrid = "albedo" in baked["pages"][0]
pages = {p["name"]: (hybrid_material(p["name"], os.path.join(D, p["albedo"]), os.path.join(D, p["indirect"]),
                                     baked["emission"], p["alpha"]) if hybrid else
                     page_material(p["name"], os.path.join(D, p["png"]), baked["emission"], p["alpha"]))
         for p in baked["pages"]}
for s in baked["surfaces"]:
    new_object(s["name"], *read_model(os.path.join(D, s["file"]))[:4], pages[s["page"]])
for s in scene_json["surfaces"]:
    if s["name"] in baked["kept"]:
        pos, nrm, uv, tris, weights = read_placed(s)
        new_object(s["name"], pos, nrm, uv, tris, materials.lit(s["material"]), weights=weights)
add_props(scene_json, materials, [s for s in args.skip_props.split(",") if s])
add_lights(scene_json, [s for s in args.skip_lights.split(",") if s] + scene_json.get("skip_lights", []), args.match, args.bulb)
bpy_scene = __import__("bpy").context.scene
bpy_scene.cycles.use_denoising = hybrid
prefix = "hybrid" if hybrid else "baked"
if hybrid:  # the engine lights the albedo directly; the bounce is the indirect page's
    bpy_scene.cycles.max_bounces = 0
    args.samples = max(args.samples, 128)
if args.engine:
    prefix = "engine-hybrid" if hybrid else "engine"
    zones = scene_json.get("zones", [])
    centre = [(zones[0]["box"][0][k] + zones[0]["box"][1][k]) / 2 for k in range(3)] if zones else None
    traced = {p["name"]: p.get("trace_at") for p in scene_json.get("probes", [])}
    ambient = [0.0, 0.0, 0.0]
    for probe in baked.get("probes", []):
        at = traced.get(probe["object"])
        if centre and at and all(abs(at[k] - centre[k]) < 1e-6 for k in range(3)):
            cube = np.fromfile(os.path.join(D, probe["file"]), dtype="<f4").reshape(-1, 3)
            ambient = [float(v) for v in cube.mean(axis=0)]
            log(f"engine: ambient {ambient} from probe {probe['object']}")
            break
    bpy_scene.cycles.max_bounces = 0
    for m in materials.made.values():
        nt = m.node_tree
        bsdf = next((n for n in nt.nodes if n.type == "BSDF_DIFFUSE"), None)
        if bsdf is None:
            continue
        colour = bsdf.inputs["Color"]
        em = nt.nodes.new("ShaderNodeEmission")
        if colour.links:
            nt.links.new(_multiply(nt, colour.links[0].from_socket, ambient), em.inputs["Color"])
        else:
            em.inputs["Color"].default_value = [colour.default_value[k] * ambient[k] for k in range(3)] + [1.0]
        add = nt.nodes.new("ShaderNodeAddShader")
        targets = [link.to_socket for link in bsdf.outputs[0].links]
        nt.links.new(bsdf.outputs[0], add.inputs[0])
        nt.links.new(em.outputs[0], add.inputs[1])
        for target in targets:
            nt.links.new(add.outputs[0], target)
wanted = [v for v in args.views.split(",") if v]
for name, view in views(scene_json).items():
    if not wanted or name in wanted:
        render(os.path.join(D, "renders"), f"{prefix}-{name}.png", view, args.fov, args.samples, args.exposure)
log("done")
