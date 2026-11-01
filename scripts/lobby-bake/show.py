"""show.py: the views as the game would draw a bake, from the files the porter takes.

  blender -b --factory-startup --python show.py -- <scene dir> [--fov 70] [--samples 64]

The baked surfaces (baked.json's converted_models and 8-bit pages) emit their
page times the multiplier and take no light; the merged surfaces left unbaked
and the native props keep their lit materials under the bake's lights.
Writes <scene dir>/renders/baked-<view>.png.
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (Materials, add_lights, add_props, hybrid_material, log, new_object, new_scene, page_material,  # noqa: E402
                    read_model, read_placed, render, views)

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
if hybrid:  # the engine lights the albedo directly; the bounce is the indirect page's
    bpy_scene.cycles.max_bounces = 0
    args.samples = max(args.samples, 128)
wanted = [v for v in args.views.split(",") if v]
for name, view in views(scene_json).items():
    if not wanted or name in wanted:
        render(os.path.join(D, "renders"), f"{'hybrid' if hybrid else 'baked'}-{name}.png", view, args.fov, args.samples, args.exposure)
log("done")
