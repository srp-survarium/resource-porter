#!/usr/bin/env python3
"""Prepare a guarded ID195-only cone preset overlay; never modify a package."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil


def sha(data):
    return hashlib.sha256(data).hexdigest()


def prepare(args):
    baseline = args.baseline_preset.read_bytes()
    preset = json.loads(baseline)
    relative = Path("presets/radar-011e-day")
    if preset["config"] != str(relative / "user.cfg"):
        raise ValueError("expected the ID195 Radar 0.11e day preset")
    keys = ("SRP_LEGACY_VOLUME_CONE", "SRP_LEGACY_VOLUME_ASSETS")
    if any(key in preset["assets"] for key in keys):
        raise ValueError("baseline already contains a cone replacement")
    expected = {
        "camera_hook.dll": (
            args.hook,
            "2ba17830aa5fd256c452d641cb84bf30de8ee58f7f5ed6e09c82c9deafd93c7d",
        ),
        "cone.dxbc": (
            args.shader,
            "c6f5566ccfc47c9fbba5eb94e2385aef465b97c78797dde619f27b2aea75c896",
        ),
        "old-spot_falloff.dds": (
            args.falloffs / "old-spot_falloff.dds",
            "2aaabea0da1d8e5402ca72ebc1dd45af278580c361af8213ef03ab2d7382d50b",
        ),
        "old-sphere_falloff.dds": (
            args.falloffs / "old-sphere_falloff.dds",
            "7cb7da5821e77a34e0b0f579ad08c169f20a392b5459d26a154632b49ba8413f",
        ),
    }
    for source, expected_sha in expected.values():
        if sha(source.read_bytes()) != expected_sha:
            raise ValueError(
                "asset differs from tested two-arm experiment: " + str(source)
            )
    args.output.mkdir(parents=True, exist_ok=False)
    overlay = args.output / "overlay"
    assets = relative / "cone-011e-validated"
    entries = []
    for name, (source, digest) in expected.items():
        target = assets / name
        destination = overlay / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        entries.append(dict(path=str(target), before_sha256=None, after_sha256=digest))
    preset["hook"] = str(assets / "camera_hook.dll")
    preset["assets"][keys[0]] = str(assets / "cone.dxbc")
    preset["assets"][keys[1]] = str(assets)
    target = relative / "preset.json"
    fixed = (json.dumps(preset, indent=2) + "\n").encode()
    (overlay / target).write_bytes(fixed)
    (args.output / "baseline-preset.json").write_bytes(baseline)
    entries.append(
        dict(path=str(target), before_sha256=sha(baseline), after_sha256=sha(fixed))
    )
    plan = dict(
        map_id=195,
        map_key="radar-011e-day",
        entries=entries,
        scope="Cone opacity and paired falloffs only; same tested hook in both runtime arms. "
        "Do not overwrite a changed parent preset or extend to other map presets.",
        apply_order="Guard every before hash and require new paths absent; copy four assets, then preset. "
        "Revert only if after hashes match: restore baseline preset, then remove four assets.",
    )
    (args.output / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    print(
        json.dumps(
            {
                "plan": str(args.output / "plan.json"),
                "parent_preset_sha256": sha(baseline),
            }
        )
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("baseline_preset", "hook", "shader", "falloffs", "output"):
        parser.add_argument(name, type=Path)
    prepare(parser.parse_args())
