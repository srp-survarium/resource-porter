#!/usr/bin/env python3
"""Name the native template of every material an X-Ray map build generated.

    xray_material_templates.py --porter VOSTOK DONOR_DB TARGET_DB PREFIX... [--families JSON]

build-soc-map clones one v0.100b template per shader family (the port
spec's material_templates) and rebinds only its textures. The 0.26 early
converter picks templates by path domain, which `ported/...` materials do
not have, so each generated material is matched back to the family template
whose non-texture values it shares (exact equality of every other leaf) and
that template's 0.26 namesake is declared explicitly. A material that matches
no family, or several equally, is an error to review. Prints the
`material_templates` entries as JSON.
"""
import argparse
import json
import subprocess
import sys

FAMILIES = [
    'material_instances/tiled/concrete/concrete_01.material',
    'material_instances/tiled/concrete/concrete_02.material',
    'material_instances/tiled/trash/trash_concrete_alpha.material',
    'material_instances/tiled/window/glass.material',
    'material_instances/water/water_green.material',
    'material_instances/flora/poplar_leaves_02.material',
    'material_instances/weapons/tracer.material',
    'material_instances/flora/sosna_bark_lod.material',
    'material_instances/terrain/03_base_final.material',
    'material_instances/flora/grass_l03_alt.material',
    'material_instances/flora/elka.material',
]


def leaves(porter, db, path):
    out = subprocess.run([porter, 'dump-db-config', '--db', db, '--path', path, '--leaves'],
                         capture_output=True, text=True, check=True).stdout
    return dict(line.split('\t', 1) for line in out.splitlines() if '\t' in line)


def shape(values):
    """Leaves that are not texture bindings (the builder rebinds those)."""
    return {k: v for k, v in values.items() if 'texture' not in k and not k.endswith('.description')}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--porter', required=True)
    ap.add_argument('donor')
    ap.add_argument('target')
    ap.add_argument('prefix', nargs='+')
    ap.add_argument('--skip', action='append', default=['post_process.material'])
    a = ap.parse_args()
    listing = subprocess.check_output([a.porter, 'list', '--db', a.donor], text=True).split()
    target = set(subprocess.check_output([a.porter, 'list', '--db', a.target, '--prefix', 'material_instances/'], text=True).split())
    families = {f: shape(leaves(a.porter, a.donor, f)) for f in FAMILIES}
    result, problems = {}, []
    for path in listing:
        if not (path.endswith('.material') and any(path.startswith(p) for p in a.prefix)):
            continue
        if any(path.endswith(s) for s in a.skip):
            continue
        mine = shape(leaves(a.porter, a.donor, path))
        scores = []
        for family, values in families.items():
            keys = set(mine) | set(values)
            differing = sum(mine.get(k) != values.get(k) for k in keys)
            scores.append((differing, family))
        scores.sort()
        best, family = scores[0]
        if len(scores) > 1 and scores[1][0] == best:
            problems.append(f'{path}: ties between {family} and {scores[1][1]} ({best} differing leaves)')
            continue
        if family not in target:
            problems.append(f'{path}: template {family} is not in the target')
            continue
        result[path] = family
        print(f'{path} -> {family} ({best} differing leaves)', file=sys.stderr)
    print(json.dumps(result, indent=2))
    if problems:
        print('\n'.join(problems), file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
