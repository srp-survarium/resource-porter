#!/usr/bin/env python3
"""Statically check that every texture a material binds resolves to a resource.

    verify_material_textures.py --porter VOSTOK DB [--prefix material_instances/ported/] [--output JSON]

For every material under PREFIX, every non-empty texture slot of every stage
must name an existing `textures/<name>.dds`. A missing texture is a defect
when the slot is sampled: its gating switch (GATED below, from the
converter's table) is on, or the slot has no gate. The engine then binds its
"missing texture" placeholder. Missing textures behind a switch that is off
are listed separately as inert (the converter now binds every missing
reference to an all-zero texture, so both lists should be empty). Exit
status 1 when any reference is missing.
"""
import argparse, json, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_collision_region import config  # noqa: E402

GATED = {'texture_bump': 'use_parallax', 'texture_vertex_blended_normal': 'use_vertex_blended_normal',
         'texture_vertex_blended_diffuse': 'use_vertex_blended_diffuse', 'texture_vertex_blended_specular': 'use_vertex_blended_specular',
         'texture_roughness': 'use_troughness', 'texture_fresnel': 'use_tfresnel', 'texture_specular_intensity': 'use_tspecular_intensity',
         'texture_specular_power': 'use_tspecular_power', 'texture_normal': 'use_nmap', 'texture_detail_normal': 'use_detail_nmap',
         'texture_ditail_diffuse': 'use_tditail_diffuse', 'texture_alphablended_normal': 'use_alphablended_normal',
         'texture_alphablended_diffuse': 'use_alphablended_diffuse', 'texture_vertex_blended_tiled_mask': 'use_tiled_vertex_blended_mask',
         'texture_emissive': 'use_emissive_map', 'texture_translucency': 'use_ttranslucency', 'normal_waves_texture': 'use_normal_waves',
         'texture_diffuse': 'use_tdiffuse'}


def value(entry):
    return entry.get('value') if isinstance(entry, dict) else None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--porter', required=True); ap.add_argument('db'); ap.add_argument('--prefix', default='material_instances/ported/')
    ap.add_argument('--output', type=Path)
    a = ap.parse_args()
    paths = subprocess.check_output([a.porter, 'list', '--db', a.db], text=True).splitlines()
    have = set(paths)
    report = {'db': a.db, 'materials': 0, 'sampled_missing': [], 'inert_missing': []}
    for m in paths:
        if not (m.startswith(a.prefix) and m.endswith('.material')):
            continue
        report['materials'] += 1
        c = config(subprocess.run([a.porter, 'extract', '--db', a.db, '--path', m, '--output', '/dev/stdout'],
                                  capture_output=True, check=True).stdout)
        for stage_name, stage in (c.get('material') or {}).items():
            if not isinstance(stage, dict) or not isinstance(stage.get('effect'), dict):
                continue
            effect = stage['effect']
            for slot, entry in effect.items():
                name = value(entry)
                if not (isinstance(name, str) and name and ('texture' in slot)):
                    continue
                if 'textures/'+name+'.dds' in have:
                    continue
                gate = GATED.get(slot)
                on = True if gate is None else bool(value(effect.get(gate, {})))
                item = {'material': m, 'stage': stage_name, 'slot': slot, 'texture': name, 'gate': gate, 'gate_on': on}
                report['sampled_missing' if on else 'inert_missing'].append(item)
    text = json.dumps(report, indent=1)+'\n'
    if a.output: a.output.write_text(text)
    print(json.dumps({'db': a.db, 'materials': report['materials'], 'sampled_missing': len(report['sampled_missing']),
                      'inert_missing': len(report['inert_missing'])}))
    for i in report['sampled_missing']: print('  SAMPLED', i['material'], i['stage'], i['slot'], i['texture'], i['gate'])
    sys.exit(1 if report['sampled_missing'] or report['inert_missing'] else 0)


if __name__ == '__main__':
    main()
