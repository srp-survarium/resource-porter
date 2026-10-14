#!/usr/bin/env python3
"""Audit an additive 0.11e Chemical Plant candidate without packing a database."""
import argparse
import hashlib
import json
import math
from pathlib import Path
from audit_collision_region import config, rotation, transform


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bundle', type=Path)
    parser.add_argument('--source-client', required=True, type=Path)
    parser.add_argument('--source-server', required=True, type=Path)
    parser.add_argument('--native-battery-options', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    a = parser.parse_args()
    record = json.loads((a.bundle/'port-manifest.json').read_text())
    assert record['donor_build'] == '0.11e-884'
    assert record['maps'] == [{'id': 194, 'level': 'level_04', 'weather': 'day'}]
    for item in record['files']:
        data = (a.bundle/'overlay'/item['path']).read_bytes()
        assert len(data) == item['size'], item['path']
        assert hashlib.sha256(data).hexdigest() == item['sha256'], item['path']
        assert not item['path'].startswith('shaders/'), item['path']
    source = config(a.source_client.read_bytes())
    old_groups = config(a.source_server.read_bytes())['objects']
    project = config((a.bundle/'overlay/projects/v011e_level_04/day_2/client_project').read_bytes())
    new_groups = project['server_objects']
    offset = config(a.native_battery_options.read_bytes())['collision_geometries'][0]['position']
    spawns = []
    assert len(old_groups['victory_item_spawners']) == len(new_groups['victory_item_spawners'])
    for i, (old, new) in enumerate(zip(old_groups['victory_item_spawners'], new_groups['victory_item_spawners'])):
        for field in ('position', 'rotation', 'point_id', 'priority', 'full_name'):
            assert old[field] == new[field], (i, field)
        expected = transform(offset, rotation(old['rotation']), old['position'])
        error = math.dist(expected, new['spawned_item']['position'])
        assert error < 0.00002, (i, error)
        assert new['spawned_item']['rotation'] == old['rotation']
        spawns.append({'index': i, 'point_id': old['point_id'], 'authored_position': old['position'],
                       'native_body_position': new['spawned_item']['position'], 'offset_error_m': error})
    source_probes = [o for o in source['objects'] if o.get('game_object_type') == 'environment_probe']
    occluders = []
    for old in source['objects']:
        if old.get('game_object_type') != 'lpv_occluder':
            continue
        new = next(o for o in project['client_objects'] if o.get('full_name') == old['full_name'])
        assert all(new[k] == old[k] for k in ('position', 'rotation', 'scale'))
        occluders.append({k: new[k] for k in ('full_name', 'position', 'rotation', 'scale')})
    probes = []
    for old in source_probes:
        new = next(o for o in project['client_objects'] if o.get('full_name') == old['full_name'])
        assert new['probe_side_width'] == old['probe_side_width']
        assert new['position'] == old['position'] and new['geometry'] == old['geometry']
        sphere = old['geometry'] == 0
        outer = .98*old['radius'] if sphere else None
        inner = old['radius']-old['probe_side_width']-.05 if sphere else None
        probes.append({'name': old['full_name'], 'geometry': old['geometry'], 'radius': old['radius'],
                       'width': old['probe_side_width'], 'source_texture': old['texture'],
                       'target_outer': outer, 'target_inner': inner,
                       'falloff_mapping_verified': False,
                       'inverted_native_sphere_interval': sphere and inner >= outer})
    collectors = []
    zones = {o['full_name']: o for o in source['objects'] if o.get('game_object_type') == 'collision_geometry'}
    for old, new in zip(old_groups['victory_items_containers'], new_groups['victory_items_containers']):
        assert old['team'] == new['team']
        assert old['collision_geometries'] == new['collision_geometries']
        zone = zones[old['collision_geometries'][0]['name']]
        collectors.append({'team': old['team'], 'zone': zone['full_name'], 'zone_position': zone['position'],
                           'source_editor_marker': old['position'], 'native_mesh_position': new['visual_geometry']['position']})
    assert len(collectors) == 2 and len({c['zone'] for c in collectors}) == 2
    summary = {'status': 'CPU-audited candidate; not runtime promoted', 'files_hashed': len(record['files']),
               'total_bytes': sum(f['size'] for f in record['files']), 'maps': record['maps'],
               'native_material_templates': len(record['converted_materials']),
               'dependency_closure': 'builder succeeded with zero unresolved dependencies',
               'donor_shaders_imported': 0, 'runtime_verified': False,
               'blockers': ['0.11e probe upload/attenuation mapping needs renderer comparison',
                            'legacy versus native cylinder dimensions differ; payload preservation does not prove collision parity'],
               'batteries': spawns, 'collectors': collectors, 'probes': probes, 'lpv_occluders': occluders,
               'compatibility_notes': record['compatibility_notes']}
    a.output.write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps({k: v for k, v in summary.items() if k not in ('batteries', 'collectors', 'probes', 'compatibility_notes')}))


if __name__ == '__main__':
    main()
