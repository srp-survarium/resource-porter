#!/usr/bin/env python3
"""Every map's grass instances, as players have them, against the importer's cap.

    grass_counts.py --vostok-port VP --db RESOURCES.DB [--patch resources.db.patch_<n>...] [--max 120000]

A map's grass is the `.veg` layers of its project folder (projects/<map>/<weather>_<mode>/
vegetation/*.veg, positions in chunk 0x58, 12 bytes each). 0.26's client cooks them all at
once with 16 bytes of stack an instance on a 2 MiB thread (`grass_cook::on_layers_loaded`
0x12A820; Skhid's exe patch 128): about 130,000 overflow it. build-survarium-maps-026 thins
every map it writes to at most 120,000 (src/survarium_026/grass.rs); this reports the maps of
any archive (stock, the live patches, a new one) and names those over --max. Each file is read
from the newest archive that has it, as the game mounts them. Exit 1 when a map is over.
"""
import argparse
import collections
import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


def listing(vp, db):
    out = subprocess.run([vp, 'list', '--db', str(db), '--prefix', 'projects/'], check=True,
                         capture_output=True, text=True).stdout
    return [p for p in out.split() if p.endswith('.veg') and '/vegetation/' in p]


def instances(data):
    at = 0
    while at + 8 <= len(data):
        tag, size = struct.unpack_from('<II', data, at)
        if tag == 0x58:
            return size // 12
        at += 8 + size
    raise ValueError('no positions chunk')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--vostok-port', required=True)
    ap.add_argument('--db', type=Path, required=True)
    ap.add_argument('--patch', type=Path, action='append', default=[])
    ap.add_argument('--max', type=int, default=120_000)
    a = ap.parse_args()
    number = lambda p: int(re.fullmatch(r'resources\.db\.patch_(\d+)', p.name)[1])
    archives = sorted(a.patch, key=number, reverse=True) + [a.db]   # newest first
    where = {}
    for db in archives:
        for path in listing(a.vostok_port, db):
            where.setdefault(path, db)
    maps = collections.Counter()
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / 'layer'
        for path, db in sorted(where.items()):
            subprocess.run([a.vostok_port, 'extract', '--db', str(db), '--path', path, '--output', str(out)],
                           check=True, stdout=subprocess.DEVNULL)
            maps[path.rsplit('/vegetation/', 1)[0]] += instances(out.read_bytes())
    over = {m: n for m, n in maps.items() if n > a.max}
    for m, n in sorted(maps.items(), key=lambda kv: -kv[1]):
        print(f'{n:8d}  {m}' + ('  OVER' if m in over else ''))
    print(f'{len(maps)} maps with grass; {len(over)} over {a.max}')
    sys.exit(1 if over else 0)


if __name__ == '__main__':
    main()
