#!/usr/bin/env python3
"""Map authored source probe controls to frozen native probe constants."""
import argparse
import json
import math
from pathlib import Path
from audit_local_light_authored import constants


def probes(value):
    if isinstance(value,dict):
        if value.get('game_object_type')=='environment_probe': yield value
        else:
            for child in value.values(): yield from probes(child)
    elif isinstance(value,list):
        for child in value: yield from probes(child)


def box_center(matrix):
    rows=[[matrix[4*i+j] for j in range(3)]+[-matrix[4*i+3]] for i in range(3)]
    for col in range(3):
        pivot=max(range(col,3),key=lambda r:abs(rows[r][col]))
        rows[col],rows[pivot]=rows[pivot],rows[col]
        if abs(rows[col][col])<1e-12: raise ValueError('singular probe transform')
        scale=rows[col][col];rows[col]=[v/scale for v in rows[col]]
        for row in range(3):
            if row!=col:
                scale=rows[row][col];rows[row]=[a-scale*b for a,b in zip(rows[row],rows[col])]
    return [r[3] for r in rows]


def audit(project,directories):
    authored=list(probes(json.loads(project.read_text())))
    if not authored: raise ValueError('no authored probes')
    rows=[];skipped=[]
    for folder in directories:
        for path in sorted(folder.rglob('*.probe-draw*.dxbc')):
            digest,cb=constants(path)
            if 'world_to_outer_probe' in cb:
                kind=1;center=box_center(cb['world_to_outer_probe'])
            elif 'probe_location' in cb:
                kind=0;center=cb['probe_location']
            else:
                skipped.append(str(path));continue
            candidates=[p for p in authored if p['geometry']==kind and math.dist(p['position'],center)<.001]
            if len(candidates)!=1: raise ValueError(f'ambiguous or absent authored center: {path}: {center}')
            p=candidates[0]
            sizes=cb.get('outer_cube_side_sizes') if kind else None
            if kind and sizes is None:
                matrix=cb['world_to_outer_probe'];sizes=[1/math.sqrt(sum(matrix[4*i+j]**2 for j in range(3))) for i in range(3)]
            size_error=max(abs(a-b) for a,b in zip(p['scale'],sizes)) if kind else None
            if size_error is not None and size_error>.001: raise ValueError(f'box scale mismatch: {path}')
            rows.append(dict(capture=str(path),shader=digest,name=p['name'],geometry=kind,center=center,
                center_error=math.dist(p['position'],center),box_size_error=size_error,
                source_clip_by_normal=p['clip_by_normal'],source_with_shadows=p['with_shadows'],
                color_parameters=cb.get('color_parameters'),native_range=cb.get('light_range'),source_radius=p['radius']))
    controls=[]
    for p in authored:
        matched=[r for r in rows if r['name']==p['name']]
        controls.append(dict(name=p['name'],geometry=p['geometry'],position=p['position'],scale=p['scale'],rotation=p['rotation'],
            radius=p['radius'],clip_by_normal=p['clip_by_normal'],with_shadows=p['with_shadows'],texture=p['texture'],
            diffuse_multiplier=p['diffuse_multiplier'],specular_multiplier=p['specular_multiplier'],captured_draws=len(matched)))
    return dict(scope='Geometry identity only: center plus geometry kind and box side sizes. Weather-specific color/texture parity is not implied. Unobserved authored controls remain unverified at runtime.',project=str(project),controls=controls,draws=rows,skipped=skipped)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('project',type=Path);p.add_argument('captures',type=Path,nargs='+');p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();r=audit(a.project,a.captures);a.output.write_text(json.dumps(r,indent=2)+'\n')
    print(json.dumps(dict(draws=len(r['draws']),authored=len(r['controls']),observed=len([v for v in r['controls'] if v['captured_draws']]),clipped=[v for v in r['controls'] if v['clip_by_normal']]),indent=2))
