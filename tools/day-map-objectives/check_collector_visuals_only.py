#!/usr/bin/env python3
"""Check that a rebuilt project differs from its baseline only by restored collector visuals.

Usage: check_collector_visuals_only.py BASELINE_PROJECT REBUILT_PROJECT

The rebuilt project must equal the baseline once the static render visuals
placed exactly at its collectors' visual_geometry transforms are removed,
and there must be one such visual per collector. Exits non-zero otherwise.
"""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_collision_region import config

def main():
    base, new = (config(Path(p).read_bytes()) for p in sys.argv[1:3])
    collectors = new['server_objects']['victory_items_containers']
    marks = [(c['visual_geometry']['lib_name'], c['visual_geometry']['position'], c['visual_geometry']['rotation']) for c in collectors]
    restored = [v for v in new['render_visuals'] if (v.get('lib_name'), v.get('position'), v.get('rotation')) in marks]
    assert len(restored) == len(collectors), f'{len(restored)} restored visuals for {len(collectors)} collectors'
    new['render_visuals'] = [v for v in new['render_visuals'] if v not in restored]
    assert new == base, 'the rebuilt project differs beyond the restored collector visuals'
    print(f'only {len(restored)} restored collector visuals differ')

if __name__ == '__main__':
    main()
