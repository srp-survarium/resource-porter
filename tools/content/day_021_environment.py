#!/usr/bin/env python3
"""One 0.21 day map's environment edit list, by the rule of three_maps_021_environment.py.

Arguments: PORTER DONOR_DB PACK LEVEL OUTPUT_JSON MODE...   (day_021_environment.py ... level_04 out.json 2 4)
"""
import json
from pathlib import Path
import sys

from three_maps_021_environment import ABOUT, author


def main():
    porter, donor, pack, level, out, *modes = sys.argv[1:]
    root = Path(__file__).resolve().parents[2]
    profiles = json.loads((root / "ports/three-maps-021-lighting.json").read_text())["profiles"]
    profile, = (p for p in profiles if p["project"] == f"projects/{level}/day/client_project")
    edit = {"_about": ABOUT, "set_entry": []}
    author(porter, donor, profile, pack, tuple(map(int, modes)), edit)
    Path(out).write_text(json.dumps(edit, indent=2) + "\n")


if __name__ == "__main__":
    main()
