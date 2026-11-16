#!/usr/bin/env python3
"""drop_live_files.py PACK_DIR PORTER [--keep-live PATH]... PATCH...  (the live resources.db.patch_<n> files, oldest first)

A pack converted against the stock archive (`pack from-bundle` without --base-patch: a bundle built on stock
cannot be converted over the live patches, whose dictionaries it would seem to remove) adds files that live
patches already hold, the shared ported/v021d resources of the maps players have. For each add_file whose path a
live patch has, the newest such patch's bytes are compared: identical, the op and its file are dropped; different,
it stops (the pack would replace what players have), unless --keep-live names the path: then the live file
stays as it is for the maps already shipped, and the new maps share it (one path is one resource).
"""
import json, subprocess, sys, tempfile
from pathlib import Path

pack, porter, rest = Path(sys.argv[1]), sys.argv[2], sys.argv[3:]
keep_live = set()
while rest and rest[0] == '--keep-live':
    keep_live.add(rest[1])
    rest = rest[2:]
patches = rest
listing = {}
for patch in patches:
    for path in subprocess.run([porter, 'list', '--db', patch], check=True, capture_output=True, text=True).stdout.splitlines():
        listing[path] = patch
manifest = json.loads((pack / 'pack.json').read_text())
kept, dropped, differ = [], [], []
with tempfile.TemporaryDirectory() as tmp:
    out = Path(tmp) / 'live'
    for op in manifest['operations']:
        path = op.get('path') if op['op'] == 'add_file' else None
        if path is None or path not in listing:
            kept.append(op)
            continue
        subprocess.run([porter, 'extract', '--db', listing[path], '--path', path, '--output', str(out)],
                       check=True, capture_output=True)
        same = out.read_bytes() == (pack / 'files' / path).read_bytes()
        (dropped if same or path in keep_live else differ).append(path)
        if not same and path in keep_live:
            print(f'{path}: live bytes kept (differs from this build)')
        if dropped and dropped[-1] == path:
            continue
        kept.append(op)
unused = keep_live - set(dropped)
if unused:
    sys.exit('--keep-live paths that differ in no live file: ' + ', '.join(sorted(unused)))
if differ:
    sys.exit('files live with other bytes:\n  ' + '\n  '.join(differ))
for path in dropped:
    (pack / 'files' / path).unlink()
manifest['operations'] = kept
(pack / 'pack.json').write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')
print(f'{len(dropped)} add_file ops dropped (identical to the live patches), {len(kept)} operations kept')
