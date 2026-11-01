"""prep.py <pack dir> <resources.db> <out dir>: the bake scene of a built lobby pack.

Reads the pack build-stk2-lobby-026 wrote (its merged model, materials, project)
and the 0.26g native props it places, and writes <out dir>/scene.json for bake.py:
every surface's converted_model file and diffuse texture (DDS, which Blender
reads), the lights and the camera, in lobby space (0.26g: x right, y up, z ahead).
Native files (props and their textures) are extracted into <out dir>/native/.
"""
import hashlib
import json
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


class Source:
    """The pack's files first, then the 0.26g database (extracted once)."""

    def __init__(self, pack, db, out):
        self.files, self.db, self.native = os.path.join(pack, "files"), db, os.path.join(out, "native")

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
    }
    if get("use_emissive", False):
        texture = get("texture_emissive", "") if get("use_emissive_map", False) else ""
        out["emission"] = {
            "texture": src.path(f"textures/{texture}.dds") if texture else None,
            "colour": get("constant_emissive", [0, 0, 0, 1])[:3],
            "multiplier": get("constant_emissive_multiplier", 1.0),
        }
    return out


def model(src, visual):
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
    return surfaces


def main(pack, db, out):
    os.makedirs(out, exist_ok=True)
    src = Source(pack, db, out)
    project = leaves(["dump-config", "--file", os.path.join(pack, "files/projects/lobby_scene")
                      + "/" + os.listdir(os.path.join(pack, "files/projects/lobby_scene"))[0] + "/client_project"])
    objects = {}
    for key, value in project.items():
        head, _, field = key.partition("].")
        if head.startswith("client_objects[") and "." not in field:
            objects.setdefault(int(head[len("client_objects["):]), {})[field] = value
    lights = [o for _, o in sorted(objects.items()) if o.get("game_object_type") == "light"]
    visuals = {}
    for key, value in project.items():
        head, _, field = key.partition("].")
        if head.startswith("render_visuals["):
            visuals.setdefault(int(head[len("render_visuals["):]), {})[field] = value
    scene = {"surfaces": [], "props": [], "lights": [], "probes": [], "camera": {
        "position": project["camera.position"], "direction": project["camera.direction"]}}
    for _, v in sorted(visuals.items()):
        entry = {"visual": v["visual_name"], "position": v["position"], "rotation": v["rotation"],
                 "scale": v["scale"], "surfaces": model(src, v["visual_name"])}
        if v["visual_name"].startswith("level/"):
            scene["surfaces"].extend(entry["surfaces"])
        else:
            scene["props"].append(entry)
    scene["probes"] = [{"name": o["name"], "position": o["position"]} for _, o in sorted(objects.items())
                       if o.get("game_object_type") == "environment_probe"]
    for light in lights:
        scene["lights"].append({k: light.get(k) for k in (
            "name", "position", "rotation", "color", "intensity", "range", "attenuation_power",
            "light_type", "spot_umbra_angle", "spot_penumbra_angle", "is_cast_shadow")})
    with open(os.path.join(out, "scene.json"), "w") as f:
        json.dump(scene, f, indent=1)
    print(f"scene: {len(scene['surfaces'])} surfaces, {len(scene['props'])} props, {len(scene['lights'])} lights,",
          f"{len(scene['probes'])} probes")


if __name__ == "__main__":
    main(*(os.path.abspath(a) for a in sys.argv[1:4]))
