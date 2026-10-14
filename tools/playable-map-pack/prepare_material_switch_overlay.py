#!/usr/bin/env python3
"""Build a guarded material-only overlay for an existing day-map package DB.

OLD_BUNDLE and NEW_BUNDLE are converter outputs before and after a material
conversion change. Only `material_instances/**/*.material` files whose bytes
differ are emitted; projects, geometry and collision stay with their owners.
Every replaced resource must currently hold the OLD bundle bytes in DATABASE,
and the overlay manifest pins DATABASE's SHA-256, so `vostok-port apply`
refuses any other base. Extra guarded files (`--extra RESOURCE=FILE=SHA256`)
may add other reviewed same-path replacements, such as a BC3 texture repair.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', type=Path, required=True)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--pair', action='append', required=True, metavar='OLD_BUNDLE=NEW_BUNDLE')
    parser.add_argument('--extra', action='append', default=[], metavar='RESOURCE=FILE=EXPECTED_SHA256')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output already exists')
    database_sha = hashlib.sha256()
    with args.database.open('rb') as stream:
        for block in iter(lambda: stream.read(8 << 20), b''):
            database_sha.update(block)
    replacements = []
    for pair in args.pair:
        old, new = (Path(p) for p in pair.split('='))
        for path in sorted((new/'overlay').rglob('*.material')):
            relative = str(path.relative_to(new/'overlay'))
            previous = old/'overlay'/relative
            if not previous.is_file():
                raise ValueError(f'{relative}: absent from the old bundle')
            before, after = previous.read_bytes(), path.read_bytes()
            if before != after:
                replacements.append((relative, before, after, 'converter material switch repair'))
    for item in args.extra:
        resource, file, expected = item.split('=')
        replacements.append((resource, None, Path(file).read_bytes(), 'reviewed replacement', expected))
    records = []
    scratch = args.output.with_name(args.output.name+'.check')
    scratch.mkdir(parents=True, exist_ok=False)
    for entry in replacements:
        resource, before, after, reason = entry[:4]
        expected = entry[4] if len(entry) > 4 else sha(before)
        current = scratch/'current'
        subprocess.run([str(args.porter), 'extract', '--db', str(args.database), '--path', resource,
                        '--output', str(current)], check=True, stdout=subprocess.DEVNULL)
        if sha(current.read_bytes()) != expected:
            raise ValueError(f'{resource}: database does not hold the expected original bytes')
        target = args.output/'overlay'/resource
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(after)
        records.append({'path': resource, 'source': reason, 'size': len(after), 'sha256': sha(after),
                        'expected_sha256': expected})
        current.unlink()
    scratch.rmdir()
    manifest = {'format_version': 1, 'asset_id': 'day-map-material-switch-repair',
                'target_db_sha256': database_sha.hexdigest(), 'files': records}
    (args.output/'port-manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print(json.dumps({'files': len(records), 'target_db_sha256': manifest['target_db_sha256']}))


if __name__ == '__main__':
    main()
