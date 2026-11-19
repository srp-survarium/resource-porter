#!/usr/bin/env python3
"""Remove byte-identical add_file resources supplied by a required earlier pack.

Usage: share_pack_files.py PACK PROVIDER [REVIEWED_DIFFERENCES.json]
All collisions are checked before changing either manifest or files. Different
bytes need a source-reviewed old/new hash pair and reason; all other differences
are errors: one resource path cannot represent two appearances.
"""
import hashlib
import json
import sys
from pathlib import Path

pack, provider = map(Path, sys.argv[1:3])
reviewed = {r["path"]: r for r in json.loads(Path(sys.argv[3]).read_text())} if len(sys.argv) == 4 else {}
used = set()
manifest = json.loads((pack / 'pack.json').read_text())
shared = json.loads((provider / 'pack.json').read_text())
providers = {op['path']: op for op in shared['operations'] if op['op'] in ('add_file', 'replace_file')}
remove = []
for op in manifest['operations']:
    if op['op'] != 'add_file' or op['path'] not in providers:
        continue
    other = providers[op['path']]
    a = pack / op.get('source', 'files/' + op['path'])
    b = provider / other.get('source', 'files/' + other['path'])
    if a.read_bytes() != b.read_bytes():
        review = reviewed.get(op['path'])
        if (review is None or not review.get('why')
                or hashlib.sha256(a.read_bytes()).hexdigest() != review['from_sha256']
                or hashlib.sha256(b.read_bytes()).hexdigest() != review['to_sha256']):
            raise SystemExit('unreviewed different shared bytes: ' + op['path'])
        used.add(op['path'])
        print(op['path'] + ': ' + review['why'])
    remove.append(op)
if used != set(reviewed):
    raise SystemExit('stale reviewed differences: ' + ', '.join(sorted(set(reviewed) - used)))
manifest['operations'] = [op for op in manifest['operations'] if op not in remove]
manifest['requires'] = sorted(set(manifest.get('requires', [])) | {shared['id']})
for op in remove:
    (pack / op.get('source', 'files/' + op['path'])).unlink()
(pack / 'pack.json').write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')
print(f'{len(remove)} files supplied by {shared["id"]}@{shared["version"]}')
