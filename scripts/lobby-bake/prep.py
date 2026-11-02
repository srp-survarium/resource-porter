"""prep.py <pack dir> <resources.db> <out dir> [<spec.json>]: the bake scene of a built lobby pack.

Reads a built lobby pack (its project, the models and materials it places, the
0.26g natives it names) and writes <out dir>/scene.json for bake.py: every
surface's converted_model file and diffuse texture (DDS, which Blender reads),
where it is placed, the lights, the sun and the camera, in lobby space (0.26g:
x right, y up, z ahead). Native files (props and their textures) are extracted
into <out dir>/native/.

<spec.json> holds the lobby's `bake` section (a build-stk2-lobby-026 manifest,
or scenes/<lobby>.json for a lobby another builder made). What is baked:
  - with `slice` (build-stk2-lobby-026): the slice's merged model
    (level/<scene>/<slice>_v<n>); the other models are props, lit by the
    engine and in the trace (they shade and bounce);
  - else every placed model, each placement on its own, but those whose name
    starts with one of `engine_lit` (lit by the engine, in the trace) or
    `left_out` (lit by the engine, not in the trace: far background, trees).
`outdoor` ({"sky": [r, g, b], "sun_scale": 1}) adds the environment's sun and
a sky of that radiance (0.26g light units); `skip_lights` the project's lights
left out of the trace (names, or prefixes ending in '/': the fills that only
stand in for a bounce); `views` ({name: [eye, target]}) the views rendered;
`eye` the point the texel density is highest around (the character's chest,
(0, 1.2, 0) by default) and `box` ([min, max]) the space the lobby is seen in:
a baked face outside it gets the fewest texels (bake.py --outside); `tint`
([r, g, b]) multiplies every light's colour, the sun's and the sky's (warmer
light); `lamp_kelvin` gives every project light a black body's colour at that
temperature (as bright as its own) and `white_kelvin` the white point every
source is seen against, `light_kelvin` some lights' own (bake.py's
light_colours); `unbaked` ([[min, max], ...], lobby frame) the zones whose
faces stay lit by the engine (under the squad and the weapon: their shadows).
Without a spec: the stk2 rule for slice "main".
"""
import hashlib
import json
import math
import os
import subprocess
import sys

PORT = os.environ.get("VOSTOK_PORT", "vostok-port")


def leaves(args):
    out = subprocess.run([PORT, *args, "--leaves"], check=True, capture_output=True, text=True).stdout
    values = {}
    for line in out.splitlines():
        key, _, value = line.partition("\t")
        kind, _, raw = value.partition(":")
        if kind == "string":
            values[key] = json.loads(raw)
        elif kind in ("f32", "i32"):
            values[key] = float(raw)
        elif kind == "bool":
            values[key] = raw == "true"
        elif kind.startswith("float"):
            values[key] = json.loads(raw)
        else:
            values[key] = raw
    return values


def indexed(project, array):
    """`array[i].field...` leaves as [{field...: value}], in index order."""
    items = {}
    for key, value in project.items():
        head, _, field = key.partition("].")
        if head.startswith(array + "["):
            items.setdefault(int(head[len(array) + 1:]), {})[field] = value
    return [v for _, v in sorted(items.items())]


class Source:
    """The pack's files first, then the 0.26g database (extracted once)."""

    def __init__(self, pack, db, out):
        self.files, self.db, self.native = os.path.join(pack, "files"), db, os.path.join(out, "native")
        self.models = {}

    def path(self, name):
        packed = os.path.join(self.files, name)
        if os.path.exists(packed):
            return packed
        local = os.path.join(self.native, name)
        if not os.path.exists(local):
            os.makedirs(os.path.dirname(local), exist_ok=True)
            subprocess.run([PORT, "extract", "--db", self.db, "--path", name, "--output", local],
                           check=True, capture_output=True)
        return local

    def config(self, name):
        return leaves(["dump-config", "--file", self.path(name)])


def material(src, name):
    v = src.config(f"material_instances/{name}.material")
    e = "material.g_stage.effect."
    get = lambda k, d=None: v.get(e + k + ".value", d)
    diffuse = get("texture_diffuse", "")
    out = {
        "name": name,
        "diffuse": src.path(f"textures/{diffuse}.dds") if diffuse and get("use_tdiffuse", True) else None,
        "colour": get("constant_diffuse", [1, 1, 1, 1])[:3],
        "alpha_test": bool(get("use_alpha_test", False)),
        "alpha_ref": get("alpha_ref", 0.25),
        "two_sided": bool(get("two_sided", False)),
        "emission": None,
        # a second layer drawn where the vertices' weight leaves the first
        "blended": (src.path(f"textures/{get('texture_vertex_blended_diffuse')}.dds")
                    if get("use_vertex_blended_textures", False) and get("use_vertex_blended_diffuse", False)
                    and get("texture_vertex_blended_diffuse") else None),
        # no g-buffer stage: drawn over the lit frame (glass, fx): not baked,
        # and the trace sees through it
        "transparent": not any(k.startswith("material.g_stage.") for k in v),
    }
    # the specular the engine draws (a hybrid page keeps the source's, laid
    # out as the page: bake.py's gloss pages)
    texture = lambda use, field: (src.path(f"textures/{get(field)}.dds") if get(use, False) and get(field) else None)
    # gbuffer_pass.ps: the texture's green channel mapped onto the
    # material's constant_roughness_min..max (51 lobby materials 0..0.2)
    out["roughness"] = {"texture": texture("use_troughness", "texture_roughness"),
                        "constant": get("constant_roughness", 0.5),
                        "range": [get("constant_roughness_min", 0.0), get("constant_roughness_max", 1.0)]}
    colour = get("constant_specular_color", [1, 1, 1, 1])[:3]
    scale = get("constant_specular_color_multiplier", [1, 1, 1])
    out["specular"] = {"texture": texture("use_tspecular_intensity", "texture_specular_intensity"),
                       "constant": [colour[k] * scale[k] for k in range(3)]}
    if get("use_emissive", False):
        texture = get("texture_emissive", "") if get("use_emissive_map", False) else ""
        out["emission"] = {
            "texture": src.path(f"textures/{texture}.dds") if texture else None,
            "colour": get("constant_emissive", [0, 0, 0, 1])[:3],
            "multiplier": get("constant_emissive_multiplier", 1.0),
        }
    return out


def decal(src, o):
    """A projected decal: its box (half-extents width, height and depth
    about its position, along its local x, y and z: decal_instance.cpp's
    orthographic projection) and its material's albedo layer."""
    v = src.config(f"material_instances/{o['decal_material']}.material")
    get = lambda k, d=None: v.get(f"material.decals.effect.{k}.value", d)
    texture = get("texture_diffuse", "")
    return {
        "name": o.get("full_name") or o["name"], "position": o["position"], "rotation": o["rotation"],
        "extent": [o["decal_width"], o["decal_height"], o["decal_far_distance"]],
        "diffuse": src.path(f"textures/{texture}.dds") if texture and get("use_tdiffuse", True) else None,
        "colour": get("constant_diffuse", [1, 1, 1, 1])[:3], "alpha": get("diffuse_alpha", 1.0),
    }


def model(src, visual):
    if visual not in src.models:
        settings = src.config(f"models/{visual}.model/settings")
        surfaces = []
        for key, value in sorted(settings.items()):
            if key.startswith("lod_hierrarchy.LOD0.surfaces["):
                file = src.path(f"models/{visual}.model/render/{value}/converted_model")
                surfaces.append({
                    "name": value,
                    "file": file,
                    "sha256": hashlib.sha256(open(file, "rb").read()).hexdigest(),
                    "material": material(src, settings[f"material_settings.{value}.material_name"]),
                })
        src.models[visual] = surfaces
    return src.models[visual]


def to_sun(properties):
    """The direction to 0.26g's sun (x right, y up, z ahead) from its
    environment's elevation and azimuth (degrees), as the engine's shader
    constants have it (tools/render-compare/README.md)."""
    el = math.radians(properties["sun_angle"])
    az = math.radians(properties["sun_position_azimut"])
    return [-math.cos(az) * math.cos(el), math.sin(el), -math.sin(az) * math.cos(el)]


def main(pack, db, out, spec_file=None):
    os.makedirs(out, exist_ok=True)
    spec = json.load(open(spec_file)).get("bake") if spec_file else {"slice": "main"}
    if not spec:
        sys.exit(f"{spec_file}: no bake section")
    src = Source(pack, db, out)
    folder = os.path.join(pack, "files/projects/lobby_scene")
    project_dir = next(d for d in sorted(os.listdir(folder)) if d != "vegetation")
    project = leaves(["dump-config", "--file", os.path.join(folder, project_dir, "client_project")])
    scene = {"surfaces": [], "props": [], "lights": [], "probes": [], "decals": [], "camera": {
        "position": project["camera.position"], "direction": project["camera.direction"]}}
    for field in ("views", "skip_lights", "eye", "box", "tint", "lamp_kelvin", "white_kelvin", "light_kelvin",
                  "unbaked"):
        if spec.get(field):
            scene[field] = spec[field]
    engine_lit, left_out = spec.get("engine_lit", []), spec.get("left_out", [])
    for k, v in enumerate(indexed(project, "render_visuals")):
        name = v.get("visual_name") or v["lib_name"]
        if any(name.startswith(p) for p in left_out):
            continue
        place = {"position": v["position"], "rotation": v["rotation"], "scale": v["scale"]}
        surfaces = model(src, name)
        if "slice" in spec:
            leaf = name.rsplit("/", 1)[-1]
            baked = name.startswith("level/") and (leaf == spec["slice"] or leaf.startswith(spec["slice"] + "_v"))
            if baked and place != {"position": [0.0] * 3, "rotation": [0.0] * 3, "scale": [1.0] * 3}:
                sys.exit(f"{name}: the slice's model is placed away from the origin")
            # the slice's surfaces keep their names: build-stk2-lobby-026 checks them
            scene["surfaces" if baked else "props"].extend(
                surfaces if baked else [dict(place, visual=name, surfaces=surfaces)])
        elif any(name.startswith(p) for p in engine_lit):
            scene["props"].append(dict(place, visual=name, surfaces=surfaces))
        else:
            for s in surfaces:  # each placement's surfaces its own, on pages of their own
                scene["surfaces"].append(dict(
                    s, name=f"{k:03d}_{name}/{s['name']}", place=place,
                    sha256=hashlib.sha256((s["sha256"] + json.dumps(place)).encode()).hexdigest()))
    for o in indexed(project, "client_objects"):
        kind = o.get("game_object_type")
        name = o.get("full_name") or o.get("name")
        if kind == "light":
            light = {f: o.get(f) for f in (
                "position", "rotation", "color", "intensity", "range", "attenuation_power", "light_type",
                "spot_umbra_angle", "spot_penumbra_angle", "spot_falloff", "is_cast_shadow", "enabled")}
            scene["lights"].append(dict(light, name=name))
        elif kind == "decal" and o.get("projection_on_static_geometry", True):
            scene["decals"].append(decal(src, o))
        elif kind == "environment_probe":
            # by its object name: build-stk2-lobby-026's probe_cubes name it so
            scene["probes"].append({"name": o["name"], "position": o["position"]})
        elif kind == "environment" and "outdoor" in spec:
            properties = {f[len("properties."):]: value for f, value in o.items() if f.startswith("properties.")}
            outdoor = spec["outdoor"]
            scene["sun"] = {"to_sun": to_sun(properties), "color": properties["sun_color"][:3],
                            "intensity": properties["sun_intensity"] * outdoor.get("sun_scale", 1.0)}
            scene["sky"] = outdoor.get("sky", [0.0, 0.0, 0.0])
    with open(os.path.join(out, "scene.json"), "w") as f:
        json.dump(scene, f, indent=1)
    through = sorted({s["material"]["name"] for s in scene["surfaces"] if s["material"]["transparent"]})
    print(f"scene: {len(scene['surfaces'])} surfaces, {len(scene['props'])} props, {len(scene['lights'])} lights,",
          f"{len(scene['decals'])} decals,",
          f"{len(scene['probes'])} probes, sun {'yes' if 'sun' in scene else 'no'};",
          f"seen through: {', '.join(through) or 'none'}")


if __name__ == "__main__":
    main(*(os.path.abspath(a) for a in sys.argv[1:5]))
