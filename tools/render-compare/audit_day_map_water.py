#!/usr/bin/env python3
"""Audit water contracts and locate release water surfaces without rebuilding archives."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

MAPS = [('radar-011e-day', 'v011e', '03'),
        ('chemical-011e-day', 'v011e', '04'),
        ('radar-020e-day', 'v020e', '03')]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('exe', 'db', 'composition', 'source-011e', 'source-020e', 'output'):
        parser.add_argument('--'+name, required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    def leaves(db, resource):
        text = subprocess.check_output([str(args.exe), 'dump-db-config', '--db', str(db),
                                        '--path', resource, '--leaves'], text=True)
        return {k: json.loads(v.split(':', 1)[1])
                for k, v in (line.split('\t', 1) for line in text.splitlines())}
    overlay = args.composition/'overlay'
    materials = {}
    for path in sorted((overlay/'material_instances/ported').glob('*/water/*.material')):
        resource = path.relative_to(overlay).as_posix()
        namespace = resource.split('/')[2]
        if namespace not in ('v011e', 'v020e'):
            continue
        source_resource = resource.replace(f'ported/{namespace}/', '', 1)
        source_db = args.source_011e if namespace == 'v011e' else args.source_020e
        source, target = leaves(source_db, source_resource), leaves(args.db, resource)
        effect = 'material.forward.effect.effect_id'
        foam = 'material.forward.effect.foam_intensity.value'
        assert source[effect] == target[effect] == 'forward_default_simpe_water'
        expected = source.get(foam, 0)
        assert target[foam] == expected, (resource, source.get(foam), target.get(foam))
        materials[resource] = dict(source_database=str(source_db), source_resource=source_resource,
            source_foam=source.get(foam), target_foam=target[foam],
            reason='authored value' if foam in source else 'source has no foam layer',
            source=source, target=target)
    maps = {}
    for key, namespace, level in MAPS:
        project = f'projects/{namespace}_level_{level}/day_2/client_project'
        scene = leaves(args.db, project)
        instances = {}
        for field, value in scene.items():
            if field.startswith('render_visuals[') and field.endswith('.lib_name'):
                prefix = field[:-len('.lib_name')]
                instances.setdefault(value, []).append({k: scene.get(prefix+'.'+k)
                    for k in ('position', 'rotation', 'scale')})
        surfaces = []
        for path in sorted((overlay/f'models/ported/{namespace}').rglob('settings')):
            if b'water/' not in path.read_bytes():
                continue
            model = path.parent.relative_to(overlay/'models').as_posix().removesuffix('.model')
            if model not in instances:
                continue
            settings = leaves(args.db, path.relative_to(overlay).as_posix())
            for field, material in settings.items():
                if not field.endswith('.material_name') or '/water/' not in str(material):
                    continue
                surface = field.split('.')[1]
                properties = f'models/{model}.model/render/{surface}/export_properties'
                bounds = leaves(args.db, properties)
                surfaces.append(dict(model=model, surface=surface, material=material,
                    instances=instances[model],
                    bounds={k:v for k,v in bounds.items() if k.startswith('bounding_')}))
        maps[key] = dict(project=project, surfaces=surfaces)
    shader_bindings = []
    paths = subprocess.check_output([str(args.exe), 'list', '--db', str(args.source_011e),
        '--prefix', 'shaders/sm_4_0/forward_simple_water.ps/'], text=True).splitlines()
    assert paths, 'source water shader permutations missing'
    with tempfile.TemporaryDirectory(prefix='water-shader-audit-') as temporary:
        shader_file = Path(temporary)/'water.bin'
        for path in paths:
            subprocess.run([str(args.exe), 'extract', '--db', str(args.source_011e),
                '--path', path, '--output', str(shader_file)], check=True,
                stdout=subprocess.DEVNULL)
            raw = shader_file.read_bytes()
            assert b'DXBC' in raw and b'packed_water_parameters_0\0' in raw
            assert b'foam_intensity\0' not in raw and b't_foam\0' not in raw, path
            shader_bindings.append(dict(resource=path, bytes=len(raw),
                sha256=hashlib.sha256(raw).hexdigest(), foam_binding=False))
    report = dict(database=str(args.db), materials=materials, maps=maps,
                  source_011e_shader_bindings=shader_bindings)
    (args.output/'contracts.json').write_text(json.dumps(report, indent=2)+'\n')
    # Radar's authored water bounds are unchanged; reuse the confirmed puddle poses.
    radar = [dict(name='water-normal', position=[26.39,4.5,-38.382],
                  forward=[0,-0.99995,-0.01], buffers=False, settle_ms=3000),
             dict(name='water-oblique', position=[20,2.5,-40],
                  forward=[1,-0.25,0], buffers=False, settle_ms=3000)]
    for key in ('radar-011e-day', 'radar-020e-day'):
        assert len(maps[key]['surfaces']) == 1
        surface = maps[key]['surfaces'][0]
        assert surface['bounds']['bounding_box.min'] == [10.74,0.427,-53.05]
        assert surface['bounds']['bounding_box.max'] == [40.6,0.496,-25.0]
        assert all(i == dict(position=[0,0,0], rotation=[0,0,0], scale=[1,1,1])
                   for i in surface['instances'])
        (args.output/(key+'-poses.json')).write_text(json.dumps(radar, indent=2)+'\n')
    chemical = maps['chemical-011e-day']['surfaces']
    terrain = next(s for s in chemical if s['model'].endswith('/terrain2'))
    center = terrain['bounds']['bounding_sphere.center']
    cube = next(s for s in chemical if s['model'].endswith('/mattest_cube01'))
    raised = max(cube['instances'], key=lambda i: i['position'][1])
    assert raised['rotation'] == [0, 0, 0]
    top = (raised['position'][1] + raised['scale'][1] *
           cube['bounds']['bounding_box.max'][1])
    chemical_poses = [
        dict(name='chemical-water-normal', position=[center[0], center[1]+3, center[2]],
             forward=[0,-0.99995,-0.01], buffers=False, settle_ms=3000),
        dict(name='chemical-green-water-normal',
             position=[raised['position'][0], top+3, raised['position'][2]],
             forward=[0,-0.99995,-0.01], buffers=False, settle_ms=3000),
    ]
    (args.output/'chemical-011e-day-poses.json').write_text(
        json.dumps(chemical_poses, indent=2)+'\n')
    print(json.dumps(dict(materials=len(materials),
        surfaces={k:len(v['surfaces']) for k,v in maps.items()})))


if __name__ == '__main__':
    main()
