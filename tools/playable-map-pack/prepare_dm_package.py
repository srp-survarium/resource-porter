#!/usr/bin/env python3
"""Assemble the X-Ray deathmatch package around a verified resources.db.

The client, match server, Wine template and play-maps wrapper come from an
existing package (the three-day candidate); launch.py from this tree; the
presets from build_day_presets.py --maps pool-soc-dm,training-camp-cs-dm
(the `source` look). OUTPUT must already hold the packed resources.db.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

MAPS = [
    ('pool-soc-dm', 'S.T.A.L.K.E.R. SoC mp_pool / team deathmatch', 197),
    ('training-camp-cs-dm', 'S.T.A.L.K.E.R. Clear Sky Training Camp (summer meadow) / team deathmatch', 198),
    ('rembasa-cs-dm', 'S.T.A.L.K.E.R. Clear Sky Repair depot (mp_rembasa) / team deathmatch', 199),
    ('sport-center-cs-dm', 'S.T.A.L.K.E.R. Clear Sky Sports center (mp_sport_center) / team deathmatch', 200),
    ('firestation-cs-dm', 'S.T.A.L.K.E.R. Clear Sky Fire station (mp_firestation) / team deathmatch', 201),
]


def fingerprint(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        while block := source.read(8 * 1024 * 1024):
            digest.update(block)
    return {'size': path.stat().st_size, 'sha256': digest.hexdigest()}


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base-package', type=Path, required=True)
    parser.add_argument('--presets', type=Path, required=True, help='build_day_presets.py output')
    parser.add_argument('--bundle', type=Path, required=True, help='the 0.26g bundle (port-manifest.json)')
    parser.add_argument('--runtime', default='/tmp/soc-dm-maps-runtime')
    parser.add_argument('--port', type=int, default=2297)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    if not (out / 'resources.db').is_file():
        parser.error('pack and verify resources.db first')
    if any(path.name not in {'resources.db', 'validation'} for path in out.iterdir()):
        parser.error('output already contains package files')
    bundle = json.loads((args.bundle / 'port-manifest.json').read_text())
    ids = sorted(item['id'] for item in bundle['maps'])
    if not set(ids) <= {m[2] for m in MAPS}:
        parser.error(f'unexpected map registrations {ids}')
    base = args.base_package.resolve()
    for name in ('client', 'server'):
        shutil.copytree(base / name, out / name)
    for name in ('wineprefix.tar.gz', 'play-maps'):
        shutil.copy2(base / name, out / name)
    shutil.copy2(Path(__file__).with_name('launch.py'), out / 'launch.py')
    catalog = json.loads((base / 'maps.json').read_text())
    catalog.update(created='soc/cs deathmatch candidate', maps=[], default_runtime=args.runtime,
                   default_port=args.port, default_ui='on',
                   notes=['X-Ray maps through 0.10b: team deathmatch (mode 4), no batteries or chargers.',
                          'Presets carry only the hook and user.cfg (native 0.26 lighting with height ambient).'])
    looks = {}
    for look in ('source', 'legacy-010b-atmosphere'):
        if (args.presets / look).is_dir():
            looks[look] = args.presets / look
    for key, title, map_id in MAPS:
        if map_id not in ids:
            continue
        source = looks['source'] / 'presets' / key
        shutil.copytree(source, out / 'presets' / key)
        catalog['maps'].append({'key': key, 'title': title, 'map_id': map_id, 'database': 'resources.db',
                                'preset': f'presets/{key}/preset.json',
                                'validation': 'Offline-verified candidate; runtime validation pending.'})
    write(out / 'maps.json', catalog)
    write(out / 'validation/port-manifest-summary.json',
          {'maps': bundle['maps'], 'compatibility_notes': bundle['compatibility_notes'],
           'files': len(bundle['files'])})
    records = {str(path.relative_to(out)): fingerprint(path) for path in sorted(out.rglob('*'))
               if path.is_file() and '__pycache__' not in path.parts}
    write(out / 'manifest.json', {'files': records, 'candidate': True})
    print(json.dumps({'package': str(out), 'maps': len(catalog['maps']), 'files': len(records)}))


if __name__ == '__main__':
    main()
