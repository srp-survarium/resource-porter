#!/usr/bin/env python3
"""Make a private copy of a candidate map package for guarded lighting arms.

Client, server and Wine template files are hard-linked read-only inputs. The
resource database is copied, so same-length experiment patches never reach the
shared package. Presets are copied; maps.json receives a private runtime and
port. The source package is only read.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(8*1024*1024), b''):
            h.update(block)
    return h.hexdigest()


def link_tree(source, destination):
    for path in sorted(source.rglob('*')):
        target = destination/path.relative_to(source)
        if path.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        elif path.is_file() and not path.is_symlink():
            target.parent.mkdir(parents=True, exist_ok=True)
            os.link(path, target)
        else:
            raise ValueError(f'unexpected package entry: {path}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    parser.add_argument('harness', type=Path)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--port', type=int, required=True)
    args = parser.parse_args()
    package, harness = args.package.resolve(), args.harness.resolve()
    if harness.exists() or args.runtime.exists():
        parser.error('harness and runtime must be new paths')
    catalog = json.loads((package/'maps.json').read_text())
    if args.port == catalog['default_port']:
        parser.error('use a private port')
    database_before = digest(package/'resources.db')
    harness.mkdir(parents=True)
    for name in ('client', 'server'):
        link_tree(package/name, harness/name)
    for name in ('launch.py', 'play-maps', 'manifest.json'):
        shutil.copy2(package/name, harness/name)
    os.link(package/'wineprefix.tar.gz', harness/'wineprefix.tar.gz')
    shutil.copytree(package/'presets', harness/'presets')
    shutil.copyfile(package/'resources.db', harness/'resources.db')
    catalog.update(default_runtime=str(args.runtime.resolve()), default_port=args.port)
    catalog['notes'] = catalog.get('notes', [])+[f'Private lighting harness copied from {package}.']
    (harness/'maps.json').write_text(json.dumps(catalog, indent=2)+'\n')
    database_after = digest(harness/'resources.db')
    if database_after != database_before or digest(package/'resources.db') != database_before:
        raise RuntimeError('database copy or source changed during preparation')
    record = {'source_package': str(package), 'database_sha256': database_before,
              'runtime': str(args.runtime.resolve()), 'port': args.port,
              'presets': {str(p.relative_to(harness)): digest(p)
                          for p in sorted((harness/'presets').rglob('*')) if p.is_file()}}
    (harness/'harness.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps({'harness': str(harness), 'database_sha256': database_before}))


if __name__ == '__main__':
    main()
