#!/usr/bin/env python3
"""Compare same-tick world hashes from two native clients in a private packet log.

The wire tick wraps every 12.8 seconds. Pair only reports received within one
second, and collapse duplicate datagrams. This reports observed agreement;
it does not claim coverage of a tick that only one client reported.
"""
import argparse
import json
import struct
from collections import defaultdict
from pathlib import Path


def compare(path):
    lengths = {0xD8: 5, 0xDF: 5, 0xD9: 4, 0xDA: 0, 0xE0: 0,
               0xE1: 0, 0xDB: 6, 0xDC: 9, 0xDD: 10, 0xDE: 2}
    reports = defaultdict(list)
    for line in path.read_text().splitlines():
        fields = line.split('\t')
        if len(fields) != 4 or fields[1] != 'in':
            continue
        received, peer = int(fields[0]), fields[2]
        packet = bytes.fromhex(fields[3])
        if len(packet) < 3:
            continue
        end, control = len(packet), packet[2]
        while control & 0x80:
            end -= 1
            if end <= 2:
                raise ValueError('invalid acknowledgement tail')
            control = packet[end]
        single = not control & 1
        payload, cursor = packet[3:end], 0
        while cursor < len(payload):
            kind = payload[cursor]
            cursor += 1
            if kind == 0xE2:
                if single:
                    count = len(payload)-cursor
                else:
                    count = payload[cursor]
                    cursor += 1
                    if count == 255:
                        count, = struct.unpack_from('<H', payload, cursor)
                        cursor += 2
            elif kind in lengths:
                count = lengths[kind]
            elif 0xFA <= kind <= 0xFE:
                count = 0
            else:
                raise ValueError(f'unexpected message {kind:#x}')
            body = payload[cursor:cursor+count]
            if len(body) != count:
                raise ValueError('truncated message')
            if kind == 0xDF:
                tick, crc = struct.unpack('<BI', body)
                reports[peer].append((received, tick, crc))
            cursor += count
            if single and cursor != len(payload):
                raise ValueError('single packet has trailing data')
    if len(reports) != 2:
        raise ValueError(f'expected two reporting peers, found {len(reports)}')
    peers = sorted(reports)
    second = defaultdict(list)
    for row in reports[peers[1]]:
        second[row[1]].append(row)
    pairs, seen, mismatches = [], set(), []
    for row in reports[peers[0]]:
        candidates = [r for r in second[row[1]] if abs(r[0]-row[0]) <= 1000]
        if not candidates:
            continue
        other = min(candidates, key=lambda r: abs(r[0]-row[0]))
        # Duplicate packet copies name the same tick/CRC within one second.
        key = (row[1], row[2], other[2])
        if any(k == key and abs(t-row[0]) <= 1000 for t, k in seen):
            continue
        seen.add((row[0], key))
        pair = {'received_ms': row[0], 'wire_tick': row[1],
                'crc': [f'{row[2]:08x}', f'{other[2]:08x}']}
        pairs.append(pair)
        if row[2] != other[2]:
            mismatches.append(pair)
    return {'peers': peers, 'raw_report_counts': [len(reports[p]) for p in peers],
            'paired_ticks': len(pairs), 'matching_ticks': len(pairs)-len(mismatches),
            'first_received_ms': pairs[0]['received_ms'] if pairs else None,
            'last_received_ms': pairs[-1]['received_ms'] if pairs else None,
            'mismatches': mismatches}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('packet_log', type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(args.packet_log), indent=2))
