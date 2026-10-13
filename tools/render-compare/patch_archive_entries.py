#!/usr/bin/env python3
"""Apply or revert saved, same-length capture experiments after stopping the client."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zlib


def crc(data):
    return int(f'{(zlib.crc32(data, 0xffffffff) ^ 0xffffffff):032b}'[::-1], 2)


def patch(archive, plan, mode):
    entries = json.loads(plan.read_text())['entries']
    pending, report = [], []
    with archive.open('rb' if mode == 'inspect' else 'r+b') as db:
        for entry in entries:
            baseline = (plan.parent / entry['baseline']).read_bytes()
            fixed = (plan.parent / entry['fixed']).read_bytes()
            assert hashlib.sha256(baseline).hexdigest() == entry['baseline_sha256']
            assert hashlib.sha256(fixed).hexdigest() == entry['fixed_sha256']
            assert len(baseline) == len(fixed)
            db.seek(entry['position'])
            current = db.read(len(baseline))
            assert current in (baseline, fixed), entry['resource'] + ': unrecognized data'
            db.seek(entry['crc_offset'] - 16)
            size, position, stored_crc = struct.unpack('<I4xQI', db.read(20))
            assert (size, position, stored_crc) == (len(current), entry['position'], crc(current))
            desired = fixed if mode == 'apply' else baseline if mode == 'revert' else current
            pending.append((entry, desired))
            report.append({'resource': entry['resource'], 'before': 'fixed' if current == fixed else 'baseline',
                           'after': 'fixed' if desired == fixed else 'baseline'})
        if mode != 'inspect':
            for entry, desired in pending:
                db.seek(entry['position'])
                db.write(desired)
                db.seek(entry['crc_offset'])
                db.write(struct.pack('<I', crc(desired)))
            db.flush()
            for entry, desired in pending:
                db.seek(entry['position'])
                assert db.read(len(desired)) == desired
                db.seek(entry['crc_offset'])
                assert db.read(4) == struct.pack('<I', crc(desired))
    return {'mode': mode, 'entries': report}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    parser.add_argument('plan', type=Path)
    parser.add_argument('mode', choices=['inspect', 'apply', 'revert'])
    args = parser.parse_args()
    print(json.dumps(patch(args.archive, args.plan, args.mode), indent=2))
