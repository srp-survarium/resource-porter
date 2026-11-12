#!/usr/bin/env python3
"""Check live ambient reset/steady packet preparation and rejected bad captures."""
import argparse
import json
from pathlib import Path
import shutil
import struct
import tempfile

from replay_live_ambient import Packet, prepare


def check(packets, reset, steady):
    accumulation = Packet((packets / 'accum.bin').read_bytes())
    h = accumulation.header
    first, = struct.unpack_from('<H', accumulation.data, accumulation.indices.start + h[14] + h[9] * 2)
    base, = struct.unpack('<i', struct.pack('<I', h[10]))
    eye_offset = h[12] + (base + first) * 36 + 16
    results = []
    for snapshot, is_reset in ((reset, True), (steady, False)):
        with tempfile.TemporaryDirectory(prefix='check-live-ambient-') as directory:
            root = Path(directory)
            for path in snapshot.parent.glob(snapshot.name + '.*'):
                shutil.copyfile(path, root / path.name)
            local = root / snapshot.name
            valid = root / 'valid'
            receipt = prepare(packets, local, valid)
            assert receipt['capture']['history_reset'] is is_reset
            rejected = []

            def rejects(name, output=None):
                target = output or root / name
                try:
                    prepare(packets, local, target)
                except ValueError:
                    assert target == valid or not target.exists()
                    rejected.append(name)
                else:
                    raise AssertionError(name + ' accepted')

            def mutate(suffix, name, transform):
                path = Path(str(local) + suffix)
                original = path.read_bytes()
                path.write_bytes(transform(original))
                try:
                    rejects(name)
                finally:
                    path.write_bytes(original)

            rejects('overwrite', valid)
            mutate('.globals.bin', 'truncated-globals', lambda b: b[:-4])
            mutate('.rt07.bin', 'truncated-current-depth', lambda b: b[:-2])
            mutate('.pass2.cb0.bin', 'mismatched-eye', lambda b: b[:20] + bytes(12) + b[32:])
            mutate('.vertices.bin', 'stale-eye-vertices', lambda b: b[:eye_offset] + bytes(12) + b[eye_offset + 12:])
            mutate('.targets.json', 'missing-normal', lambda b: json.dumps(json.loads(b)[:-2] + json.loads(b)[-1:]).encode())
            if is_reset:
                mutate('.rt05.bin', 'reset-retained-ao', lambda b: b'\xff' + b[1:])
                mutate('.rt06.bin', 'reset-retained-depth', lambda b: b'\xff' + b[1:])
                mutate('.rt01.bin', 'reset-retained-mask', lambda b: b'\xff' + b[1:])
            packet = (valid / 'accum.bin').read_bytes()
            for data in (packet[:20], packet[:-1], packet + b'\0'):
                try:
                    Packet(data)
                except ValueError:
                    pass
                else:
                    raise AssertionError('invalid packet extent accepted')
            results.append(dict(reset=is_reset, rejected=rejected, packet_extent_rejections=3))
    print(json.dumps(dict(preparation='passed', results=results), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('packets', 'reset', 'steady'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    check(args.packets, args.reset, args.steady)
