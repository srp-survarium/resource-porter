#!/usr/bin/env python3
"""Extract complete E2 checkpoints from the private server's TSV packet log.

Use the game's existing `survarium --example batteries` decoder on the output.
Only snapshot bytes are saved; no client state is changed.
"""
import argparse
import struct
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('packet_log', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
lengths = {0xD8: 5, 0xDF: 5, 0xD9: 4, 0xDA: 0, 0xE0: 0,
           0xE1: 0, 0xDB: 6, 0xDC: 9, 0xDD: 10, 0xDE: 2}
parts = {}
for line in args.packet_log.read_text().splitlines():
    fields = line.split('\t')
    if len(fields) != 4 or fields[1] != 'in':
        continue
    packet = bytes.fromhex(fields[3])
    if len(packet) < 3:
        continue
    end, control = len(packet), packet[2]
    while control & 0x80:
        end -= 1
        assert end > 2
        control = packet[end]
    single = not control & 1
    payload, cursor = packet[3:end], 0
    while cursor < len(payload):
        kind = payload[cursor]
        cursor += 1
        if kind == 0xE2:
            if single:
                count = len(payload) - cursor
            else:
                count = payload[cursor]
                cursor += 1
                if count == 255:
                    count, = struct.unpack_from('<H', payload, cursor)
                    cursor += 2
            body = payload[cursor:cursor + count]
            assert len(body) == count and count >= 9
            time, offset, total = struct.unpack_from('<IHH', body)
            data = body[8:]
            assert 4 <= total <= 32764 and offset % 480 == 0
            assert len(data) == min(480, total - offset)
            key = (fields[2], time, total)
            chunks = parts.setdefault(key, {})
            assert chunks.get(offset, data) == data
            chunks[offset] = data
        elif kind in lengths:
            count = lengths[kind]
        elif 0xFA <= kind <= 0xFE:
            count = 0
        else:
            raise ValueError(f'Unexpected client message {kind:#x}')
        cursor += count
        assert cursor <= len(payload)
        if single:
            assert cursor == len(payload)

written = 0
for (peer, time, total), chunks in parts.items():
    if len(chunks) != (total + 479) // 480:
        continue
    data = b''.join(chunks[offset] for offset in range(0, total, 480))
    assert len(data) == total
    file = args.output / f'match026g_{peer.replace(":", "-")}_{time}_battery.bin'
    file.write_bytes(data)
    written += 1
print(f'Extracted {written} complete snapshots')
