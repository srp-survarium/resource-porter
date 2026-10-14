#!/usr/bin/env python3
"""Prepare a guarded additive-overlay patch for `vostok-port apply`; never writes an archive.

Copies the named resources from a freshly rebuilt bundle overlay and records,
for each, whether it replaces an existing archive entry (pinned by its current
SHA-256) or adds a new path. `--check ARCHIVE` re-verifies every guard against
an archive immediately before applying, so a changed candidate is rejected.

    prepare_overlay_patch.py BUILT_BUNDLE OUTPUT ARCHIVE --archive-sha256 HASH --prefix RES_DIR ...
    prepare_overlay_patch.py --check OUTPUT ARCHIVE
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

PORTER = '/home/sheep/Projects/survarium/resource-porter/target/release/vostok-port'
ENV = dict(os.environ, LD_LIBRARY_PATH='/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib')


def sha(data):
    return hashlib.sha256(data).hexdigest()


def file_sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        while block := f.read(16 << 20):
            h.update(block)
    return h.hexdigest()


def current(porter, archive, path):
    """Current archive bytes of a resource, or None when the path is absent."""
    listing = subprocess.run([porter, 'list', '--db', str(archive), '--prefix', path],
                             env=ENV, text=True, capture_output=True, check=True).stdout.split()
    if path not in listing:
        return None
    with __import__('tempfile').TemporaryDirectory() as tmp:
        out = Path(tmp) / 'r'
        subprocess.run([porter, 'extract', '--db', str(archive), '--path', path, '--output', str(out)],
                       env=ENV, check=True, capture_output=True)
        return out.read_bytes()


def check(porter, bundle, archive):
    record = json.loads((bundle / 'port-manifest.json').read_text())
    if file_sha(archive) != record['target_db_sha256']:
        raise SystemExit('archive hash differs from the patch baseline')
    for guard in record['guards']:
        data = current(porter, archive, guard['path'])
        if guard['baseline_sha256'] is None:
            if data is not None:
                raise SystemExit('path unexpectedly exists: ' + guard['path'])
        elif data is None or sha(data) != guard['baseline_sha256']:
            raise SystemExit('baseline mismatch: ' + guard['path'])
    for entry in record['files']:
        data = (bundle / 'overlay' / entry['path']).read_bytes()
        if sha(data) != entry['sha256'] or len(data) != entry['size']:
            raise SystemExit('patch payload changed: ' + entry['path'])
    return record


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--check', action='store_true')
    ap.add_argument('--porter', default=PORTER)
    ap.add_argument('args', nargs='+')
    ap.add_argument('--archive-sha256')
    ap.add_argument('--prefix', action='append', default=[])
    ap.add_argument('--reason', default='')
    a = ap.parse_args()
    if a.check:
        bundle, archive = map(Path, a.args)
        record = check(a.porter, bundle, archive)
        print(json.dumps({'verified_guards': len(record['guards'])}))
        return
    built, output, archive = map(Path, a.args)
    if output.exists():
        raise SystemExit('choose a fresh output directory')
    built_record = json.loads((built / 'port-manifest.json').read_text())
    selected = [f for f in built_record['files'] if any(f['path'].startswith(p.rstrip('/') + '/') for p in a.prefix)]
    if not selected:
        raise SystemExit('no built files under the requested prefixes')
    files, guards = [], []
    for entry in selected:
        source = built / 'overlay' / entry['path']
        data = source.read_bytes()
        if sha(data) != entry['sha256']:
            raise SystemExit('built bundle file changed: ' + entry['path'])
        old = current(a.porter, archive, entry['path'])
        guards.append({'path': entry['path'], 'baseline_sha256': None if old is None else sha(old),
                       'action': 'add' if old is None else 'replace'})
        destination = output / 'overlay' / entry['path']
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        files.append(entry)
    record = {'format_version': 1, 'target_db_sha256': a.archive_sha256, 'reason': a.reason,
              'built_bundle': str(built), 'files': files, 'guards': guards}
    (output / 'port-manifest.json').write_text(json.dumps(record, indent=2) + '\n')
    check(a.porter, output, archive)
    print(json.dumps({'patch': str(output), 'guards': guards}, indent=2))


if __name__ == '__main__':
    sys.exit(main())
