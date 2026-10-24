#!/usr/bin/env python3
"""Write the 0.26g import spec of the STALKER team-deathmatch maps.

    make_spec.py MAP... --donor-sha256 HEX --templates TEMPLATES.json --output SPEC.json
                 [--maps ports/stalker-tdm/maps.json] [--spots ports/stalker-tdm/spots.json]

Each MAP is a key of maps.json (cs-mp_bath, cs-mp_military_1, training-camp-cs-dm,
rembasa-cs-dm, dl-mp_station, dl-mp_depository). The spawns and ammo stashes
come from spots.json, the map spot planner's list: every spot of the map, by
team (team 0 = team 1). TEMPLATES.json is xray_material_templates.py's output
for the map's donor. No ramps, lids or blockers: tall ladders stay as they are.
"""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET_SHA256 = 'c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('map', nargs='+')
    ap.add_argument('--donor-sha256', required=True)
    ap.add_argument('--templates', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--maps', type=Path, default=ROOT / 'ports/stalker-tdm/maps.json')
    ap.add_argument('--spots', type=Path, default=ROOT / 'ports/stalker-tdm/spots.json')
    a = ap.parse_args()
    entries = [entry_of(key, json.loads(a.maps.read_text())['maps'][key], json.loads(a.spots.read_text())['spots']) for key in a.map]
    spec = {
        'source_build': '0.10b',
        'donor_sha256': a.donor_sha256,
        'target_sha256': TARGET_SHA256,
        'strict_finite': True,
        'maps': entries,
        'material_templates': json.loads(a.templates.read_text()),
    }
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(spec, indent=1, ensure_ascii=False) + '\n')


def entry_of(key, m, all_spots):
    spots = [s for s in all_spots if s['map'] == key]
    teams = [[], []]
    stashes = []
    for s in sorted(spots, key=lambda s: s['id']):
        if s['kind'] == 'spawn':
            teams[s['team']].append([s['x'], s['y'], s['z'], s.get('yaw', 0.0)])
        else:
            stashes.append({'team': s['team'], 'position': [s['x'], s['y'], s['z']],
                            **({'reason': s['reason']} if s.get('reason') else {})})
    if not teams[0] or not teams[1]:
        raise SystemExit(f'{key}: the planner has no spawns for both teams')
    entry = {
        'id': m['id'], 'level': 'level_03', 'weather': 'day', 'mode': 4,
        'donor_project': m['project'], 'hud_key': m['project'],
        'map_name': m['map_name'], 'height_ambient': True,
        'skylight_probe': m['skylight_probe'],
        'deathmatch': {'source': f"the map spot planner: {len(teams[0])} + {len(teams[1])} spawns by team", 'teams': teams},
        'level_number': m['level_number'],
    }
    if m.get('loading_screen_id') is not None:
        entry['loading_screen_id'] = m['loading_screen_id']
    if m.get('map_names'):
        entry['map_names'] = m['map_names']
    if m.get('campfires'):
        entry['campfires'] = m['campfires']
    if stashes:
        entry['resupply'] = stashes
    print(f'map {m["id"]} {m["project"]}: {len(teams[0])}+{len(teams[1])} spawns, {len(stashes)} stashes')
    return entry


if __name__ == '__main__':
    main()
