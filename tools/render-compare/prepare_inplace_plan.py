#!/usr/bin/env python3
"""Turn guarded same-length overlay files into a patch_archive_entries.py plan.

Each RESOURCE=FILE pair names an archive path and its prepared replacement.
The current archive bytes must hash to the overlay's declared original, the
entry must be stored uncompressed and the replacement must keep its length.
The archive is only read; apply the plan with patch_archive_entries.py after
stopping every client that uses it.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--scope', required=True)
    parser.add_argument('entry', nargs='+', help='RESOURCE=FILE=EXPECTED_ORIGINAL_SHA256')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    with args.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    entries = []
    for index, item in enumerate(args.entry):
        resource, source, expected = item.split('=')
        fixed = Path(source).read_bytes()
        baseline_path = args.output/f'{index:02d}-baseline.bin'
        fixed_path = args.output/f'{index:02d}-fixed.bin'
        subprocess.run([str(args.porter), 'extract', '--db', str(args.archive), '--path', resource,
                        '--output', str(baseline_path)], check=True, stdout=subprocess.DEVNULL)
        baseline = baseline_path.read_bytes()
        if sha(baseline) != expected:
            raise ValueError(f'{resource}: archive bytes are not the declared original')
        if len(baseline) != len(fixed) or baseline == fixed:
            raise ValueError(f'{resource}: replacement must change bytes and keep length')
        fixed_path.write_bytes(fixed)
        stat = subprocess.check_output([str(args.porter), 'stat', '--db', str(args.archive),
                                        '--path', resource], text=True)
        if 'compressed: false, inlined: false' not in stat:
            raise ValueError(f'{resource}: only uncompressed, non-inlined entries can be patched')
        position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
        needle = struct.pack('<I4xQ', len(baseline), position)
        found = fat.find(needle)
        if found < 0 or fat.find(needle, found+1) >= 0:
            raise ValueError(f'{resource}: file table entry is absent or ambiguous')
        entries.append({'resource': resource, 'position': position, 'crc_offset': 24+found+16,
                        'baseline': baseline_path.name, 'fixed': fixed_path.name,
                        'baseline_sha256': expected, 'fixed_sha256': sha(fixed)})
    plan = {'scope': args.scope, 'entries': entries}
    (args.output/'patch.json').write_text(json.dumps(plan, indent=2)+'\n')
    print(json.dumps({'plan': str(args.output/'patch.json'), 'entries': len(entries)}))


if __name__ == '__main__':
    main()
