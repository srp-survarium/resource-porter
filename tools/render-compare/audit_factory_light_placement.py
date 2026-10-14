#!/usr/bin/env python3
"""Check decoded Factory day projects without mistaking opacity for placement."""
import argparse
import json
from pathlib import Path


def audit(source, target):
    rows = []
    for source_key, target_key, kind in (
        ('objects', 'client_objects', 'local_light'),
        ('render_visuals', 'render_visuals', 'cone'),
        ('render_visuals', 'render_visuals', 'lamp'),
    ):
        for index, item in enumerate(source[source_key]):
            name = item.get('lib_name', '')
            selected = (kind == 'local_light' and item.get('game_object_type') == 'light' and name != 'sun'
                        or kind == 'cone' and name == 'editor/cone'
                        or kind == 'lamp' and name.startswith('props/light/'))
            if not selected:
                continue
            candidates = [(i, obj) for i, obj in enumerate(target[target_key])
                          if obj.get('lib_name', '').removeprefix('ported/v010w/') == name
                          and obj.get('position') == item.get('position')]
            if len(candidates) != 1:
                raise ValueError(f'{kind} {index}: expected one unchanged origin, found {len(candidates)}')
            target_index, match = candidates[0]
            for field in ('position', 'rotation', 'scale'):
                if match[field] != item[field]:
                    raise ValueError(f'{kind} {index}: changed {field}')
            if kind == 'local_light':
                for field in ('light_type', 'range', 'spot_penumbra_angle', 'spot_umbra_angle'):
                    if match[field] != item[field]:
                        raise ValueError(f'local light {index}: changed {field}')
                offset = match.get('offset')
                if offset != {'position': [0, 0, 0], 'rotation': [0, 0, 0], 'scale': [1, 1, 1]}:
                    raise ValueError(f'local light {index}: nonidentity offset')
            rows.append(dict(kind=kind, source_index=index, target_index=target_index,
                             model=name, position=item['position']))
    counts = {kind: sum(row['kind'] == kind for row in rows) for kind in ('local_light', 'cone', 'lamp')}
    if counts['local_light'] != 18 or counts['cone'] != 12:
        raise ValueError(f'unexpected Factory day counts: {counts}')
    return dict(counts=counts, instances=rows,
                scope='Project placement and cone parameters only; shader opacity and runtime transforms require separate verification.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('target', type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(json.loads(args.source.read_text()), json.loads(args.target.read_text())), indent=2))
