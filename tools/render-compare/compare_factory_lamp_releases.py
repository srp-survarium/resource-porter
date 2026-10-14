#!/usr/bin/env python3
"""Compare Factory source lamps, beam assets, executable shaders and lighting."""

import argparse
import collections
import hashlib
import json
import struct
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "chemical-plant"))
from audit_collision_region import config


def changes(old, new, path=""):
    if isinstance(old, dict) and isinstance(new, dict):
        for key in sorted(old.keys() | new.keys()):
            yield from changes(old.get(key), new.get(key), f"{path}.{key}".lstrip("."))
    elif old != new:
        yield {"path": path, "old": old, "new": new}


parser = argparse.ArgumentParser(description=__doc__)
for name in ("porter", "source_010b", "source_011a", "source_011e", "output"):
    parser.add_argument(name, type=Path)
args = parser.parse_args()
porter = str(args.porter)
out = args.output
out.mkdir(parents=True, exist_ok=False)
sources = {
    "010b": str(args.source_010b),
    "011a": str(args.source_011a),
    "011e": str(args.source_011e),
}
resources = [
    "projects/level_03/client_project",
    "material_instances/fx/cone_volume_material.material",
    "material_instances/props/light/lamp_greenhouse_clone.material",
    "material_instances/post_process/level_03_day.material",
    "models/editor/cone.model/render/cone2sg1/converted_model",
    "models/editor/cone.model/settings",
    "models/props/light/green_lamp_with_wire.model/render/lamp_lights_s/converted_model",
    "models/props/light/green_lamp_with_wire.model/settings",
    "textures/fx/spot_falloff.dds",
    "textures/fx/sphere_falloff.dds",
]
records = {}
decoded = {}
for version, db in sources.items():
    paths = (
        resources
        + subprocess.check_output(
            [
                porter,
                "list",
                "--db",
                db,
                "--prefix",
                "shaders/sm_4_0/forward_volume_cone_base.ps/",
            ],
            text=True,
        ).splitlines()
    )
    records[version] = {}
    decoded[version] = {}
    for path in paths:
        dest = out / version / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [porter, "extract", "--db", db, "--path", path, "--output", str(dest)],
            check=True,
            capture_output=True,
        )
        data = dest.read_bytes()
        record = {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
        if "shaders/" in path:
            at = data.index(b"DXBC")
            size = struct.unpack_from("<I", data, at + 24)[0]
            record["dxbc_hash"] = hashlib.sha256(data[at : at + size]).hexdigest()
            record["dxbc_size"] = size
            code = data[at : at + size]
            executable = []
            for i in range(struct.unpack_from("<I", code, 28)[0]):
                offset = struct.unpack_from("<I", code, 32 + 4 * i)[0]
                length = struct.unpack_from("<I", code, offset + 4)[0]
                if code[offset : offset + 4] in (b"SHDR", b"SHEX"):
                    executable.append(
                        hashlib.sha256(
                            code[offset + 8 : offset + 8 + length]
                        ).hexdigest()
                    )
            if len(executable) != 1:
                raise ValueError("expected one shader executable chunk: " + path)
            record["executable_sha256"] = executable[0]
        elif path.endswith((".material", "/settings", "/client_project")):
            decoded[version][path] = config(data)
        records[version][path] = record
report = {"sources": sources, "resources": records, "comparisons": {}}


def canon(value):
    return json.dumps(value, sort_keys=True)


for oldversion in ["010b", "011a"]:
    old = decoded[oldversion]["projects/level_03/client_project"]
    new = decoded["011e"]["projects/level_03/client_project"]
    comparison = {}
    for label, predicate in [
        ("cone", lambda x: x.get("lib_name") == "editor/cone"),
        ("lamp", lambda x: x.get("lib_name", "").startswith("props/light/")),
    ]:
        selected = [
            [
                {k: x[k] for k in ["lib_name", "position", "rotation", "scale"]}
                for x in project["render_visuals"]
                if predicate(x)
            ]
            for project in [old, new]
        ]
        a, b = [collections.Counter(map(canon, items)) for items in selected]
        comparison[label] = {
            "old_count": sum(a.values()),
            "new_count": sum(b.values()),
            "removed": [json.loads(x) for x in (a - b).elements()],
            "added": [json.loads(x) for x in (b - a).elements()],
        }
    kinds = {"light", "environment", "environment_probe", "sky"}
    a, b = [
        {
            x["full_name"]: x
            for x in project["objects"]
            if x["game_object_type"] in kinds
        }
        for project in [old, new]
    ]
    for project, objects in [(old, a), (new, b)]:
        if len(objects) != sum(
            x["game_object_type"] in kinds for x in project["objects"]
        ):
            raise ValueError("duplicate lighting object identity")
    comparison["lighting_changes"] = {
        key: list(changes(a.get(key), b.get(key)))
        for key in sorted(a.keys() | b.keys())
        if a.get(key) != b.get(key)
    }
    comparison["config_changes"] = {
        path: list(changes(decoded[oldversion][path], decoded["011e"][path]))
        for path in decoded[oldversion]
        if path != "projects/level_03/client_project"
        and decoded[oldversion][path] != decoded["011e"][path]
    }
    comparison["resource_hash_equal"] = {
        path: record["sha256"] == records["011e"].get(path, {}).get("sha256")
        for path, record in records[oldversion].items()
        if not path.startswith("shaders/")
    }
    a, b = [
        collections.Counter(
            r["executable_sha256"]
            for path, r in records[v].items()
            if path.startswith("shaders/")
        )
        for v in [oldversion, "011e"]
    ]
    comparison["cone_shader_executables"] = {
        "old_variants": sum(a.values()),
        "new_variants": sum(b.values()),
        "old_unique": len(a),
        "new_unique": len(b),
        "identical_multiset": a == b,
        "removed": dict(a - b),
        "added": dict(b - a),
    }
    report["comparisons"][oldversion + "-to-011e"] = comparison
(out / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
print(
    json.dumps(
        {
            "report": str(out / "comparison.json"),
            "comparisons": {
                key: {
                    "lighting_identity_changes": len(value["lighting_changes"]),
                    "shader_executables": value["cone_shader_executables"],
                }
                for key, value in report["comparisons"].items()
            },
        },
        indent=2,
    )
)
