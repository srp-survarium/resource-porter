#!/usr/bin/env python3
"""Statically check that a candidate database requests no unshipped shader.

    verify_shader_variants.py --porter VOSTOK_PORT BASELINE_DB CANDIDATE_DB

BASELINE_DB must be a database known to load (every shader its materials
request ships). For every material whose decoded shader switches differ
between the two databases, every shipped key (of any shader family) that
agrees with the baseline material on all switch digits the family uses may
be a key that material requests; the same key with the candidate's digits
must ship in the candidate's archive. Geometry, LOD, instancing, vertex
input and undecoded material digits are carried over unchanged, so no
assumption about how the material is drawn is needed. Also reported:
materials whose shadow casting is newly enabled (new shadow-pass requests)
and materials present only in the candidate (not provable this way).
Exit status is 1 when anything is missing or unprovable.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_collision_region import config  # noqa: E402

# Global configuration positions (67-character keys); see material_conversion.rs.
SWITCHES = [(5, 'use_tdiffuse'), (6, 'use_nmap'), (7, 'use_detail_nmap'), (8, 'use_alpha_test'),
            (10, 'use_parallax'), (11, 'use_ttranslucency'), (12, 'use_tspecular_intensity'),
            (13, 'use_tfresnel'), (14, 'use_troughness'), (16, 'use_tditail_diffuse'),
            (28, 'use_alphablended_normal'), (35, 'use_tiled_vertex_blended_mask'),
            (44, 'use_uv_scrolling'), (46, 'use_fuzziness'), (53, 'wind_motion')]


def position(length, p):
    return p+1 if length == 68 and p > 22 else p


def digits(material):
    effect = (material.get('material', {}).get('g_stage') or {}).get('effect') or {}
    out = {}
    for p, name in SWITCHES:
        value = effect.get(name, {}).get('value', False) if isinstance(effect.get(name), dict) else False
        out[p] = str(int(value))
    return out


def shadow(material):
    stage = material.get('material', {}).get('g_stage') or {}
    value = stage.get('is_cast_shadow')
    return bool(value.get('value')) if isinstance(value, dict) else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    listing = lambda db: subprocess.check_output([str(args.porter), 'list', '--db', str(db)], text=True,
                                                  stderr=subprocess.DEVNULL).split()
    candidate_paths = listing(args.candidate)
    baseline_paths = set(listing(args.baseline))
    families = {}
    for path in candidate_paths:
        parts = path.split('/')
        if len(parts) == 4 and parts[0] == 'shaders' and len(parts[3]) in (67, 68):
            families.setdefault(parts[2], set()).add(parts[3])
    report = {'baseline': str(args.baseline), 'candidate': str(args.candidate),
              'checked_materials': 0, 'missing': {}, 'new_shadow_casters': [], 'unprovable_new_materials': []}
    with tempfile.TemporaryDirectory() as scratch:
        def read(db, path):
            out = Path(scratch)/'resource'
            subprocess.run([str(args.porter), 'extract', '--db', str(db), '--path', path, '--output', str(out)],
                           check=True, stdout=subprocess.DEVNULL)
            return out.read_bytes()
        for path in candidate_paths:
            if not (path.startswith('material_instances/') and path.endswith('.material')):
                continue
            new_bytes = read(args.candidate, path)
            if path not in baseline_paths:
                if '/ported/' in path:
                    report['unprovable_new_materials'].append(path)
                continue
            old_bytes = read(args.baseline, path)
            if old_bytes == new_bytes:
                continue
            old, new = config(old_bytes), config(new_bytes)
            report['checked_materials'] += 1
            if shadow(old) is False and shadow(new) is True:
                report['new_shadow_casters'].append(path)
            before, after = digits(old), digits(new)
            changed = {p: after[p] for p in after if after[p] != before[p]}
            if not changed:
                continue
            missing = []
            for family, keys in families.items():
                for key in keys:
                    if any(key[position(len(key), p)] not in ('_', before[p]) for p in before):
                        continue
                    target = list(key)
                    for p, value in changed.items():
                        if key[position(len(key), p)] != '_':
                            target[position(len(key), p)] = value
                    target = ''.join(target)
                    if target != key and target not in keys:
                        missing.append(f'shaders/sm_4_0/{family}/{target}')
            if missing:
                report['missing'][path] = sorted(set(missing))
    report['missing_shader_count'] = sum(len(v) for v in report['missing'].values())
    text = json.dumps(report, indent=2)+'\n'
    if args.output:
        args.output.write_text(text)
    print(json.dumps({k: (len(v) if isinstance(v, (list, dict)) else v) for k, v in report.items()}, indent=2))
    bad = report['missing'] or report['new_shadow_casters'] or report['unprovable_new_materials']
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
