#!/usr/bin/env python3
"""Leave out of a pack the files players have already, byte for byte.

    leave_out_live.py --pack PACK --stock DB --patch resources.db.patch_<n>... --vostok-port VP

A pack of maps added after a released one (stalker-maps-2 after stalker-maps)
converts the same v0.100b template files (`textures/ported/v010b/...`, a
few `material_instances/ported/v010b/...`) the released pack added: `pack
from-bundle` against the stock db adds them again, and `pack check` over the
live patches refuses an add of a path players have. Each `add_file` of the
pack whose path a live archive holds (the newest of the patches, else the
stock db) is compared with that file: the same bytes, the operation and its
file are left out (players keep theirs); other bytes stop it (the pack would
change a file players have). pack.json is rewritten in place; it prints what
was left out.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path


def listing(vp, db):
    out = subprocess.run([vp, 'list', '--db', str(db)], check=True, capture_output=True, text=True).stdout
    return set(out.split())


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--pack', type=Path, required=True)
    ap.add_argument('--stock', type=Path, required=True)
    ap.add_argument('--patch', type=Path, action='append', default=[])
    ap.add_argument('--vostok-port', required=True)
    a = ap.parse_args()
    number = lambda p: int(re.fullmatch(r'resources\.db\.patch_(\d+)', p.name)[1])
    archives = sorted(a.patch, key=number, reverse=True) + [a.stock]   # newest first
    manifest = json.loads((a.pack / 'pack.json').read_text())
    adds = {o['path'] for o in manifest['operations'] if o['op'] == 'add_file'}
    where = {}
    for db in archives:
        for path in adds & listing(a.vostok_port, db):
            where.setdefault(path, db)
    same, differ = [], []
    with tempfile.TemporaryDirectory() as tmp:
        for path, db in sorted(where.items()):
            live = Path(tmp) / 'live'
            subprocess.run([a.vostok_port, 'extract', '--db', str(db), '--path', path, '--output', str(live)],
                           check=True, stdout=subprocess.DEVNULL)
            (same if live.read_bytes() == (a.pack / 'files' / path).read_bytes() else differ).append((path, db.name))
    for path, db in differ:
        print(f'differs from players\' file ({db}): {path}', file=sys.stderr)
    if differ:
        sys.exit(f'{len(differ)} file(s) of the pack would change what players have')
    left = {p for p, _ in same}
    manifest['operations'] = [o for o in manifest['operations'] if not (o['op'] == 'add_file' and o['path'] in left)]
    for path in left:
        (a.pack / 'files' / path).unlink()
    (a.pack / 'pack.json').write_text(json.dumps(manifest, indent=2, ensure_ascii=False))
    for path, db in same:
        print(f'left out, players have it ({db}): {path}')
    print(f'{len(left)} file(s) left out; {sum(o["op"] == "add_file" for o in manifest["operations"])} added')


if __name__ == '__main__':
    main()
