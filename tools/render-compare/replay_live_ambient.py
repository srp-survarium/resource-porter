#!/usr/bin/env python3
"""Prepare independent GPU replays of original_ambient_live.c snapshots.

Use replay_ssao_chain.exe with the four prepared packets and --reset-history
when receipt.json says history_reset. These are private original-shader outputs
on native inputs; this check does not establish final scene lighting parity.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from replay_post import require
from replay_ssao_011e import PASSES

NAMES = ('accum', 'mask', 'temporal', 'sky')
SHADERS = [PASSES[name][0] for name in ('ssao-accumulation', 'ssao-history-mask', 'ssao-temporal')]
SHADERS.append('ced5345f1cf23b1c8ce9eb050debc71c4fb281f0072aae2d813cc7a5d2232b11')
VERTEX_SHADERS = [PASSES[name][1] for name in ('ssao-accumulation', 'ssao-history-mask', 'ssao-temporal')]
VERTEX_SHADERS.append('9dab4eced49b2980e4e70843a6f42e9f59a84bf6e696121bc0c931e685a9f6f4')
OUTPUTS = (('.accum.bin', 0), ('.mask.bin', 1), ('.ao.bin', 2), ('.depth.bin', 3), ('', 4))


class Packet:
    """Locate fixed-size fields; replacements cannot change packet extents."""

    def __init__(self, data):
        self.data = bytearray(data)
        self.at = 8
        require(self.data[:8] in (b'SSA011E1', b'SKY011E2'), 'unsupported packet')
        self.header = self.words(15)
        self.pixel_shader = self.blob()
        self.vertex_shader = self.blob()
        self.constants = {}
        for stage in range(2):
            count, = self.words(1)
            require(count <= 4, 'too many constant buffers')
            for _ in range(count):
                slot, = self.words(1)
                require(slot < 4 and (stage, slot) not in self.constants, 'constant slot differs')
                self.constants[stage, slot] = self.blob()
        self.vertices = self.blob()
        self.indices = self.blob()
        self.textures = {}
        count, = self.words(1)
        require(count <= 16, 'too many textures')
        for _ in range(count):
            offset = self.at
            metadata = self.words(10)
            require(metadata[1] == 2 and metadata[4:6] == (1, 1), 'expected single-mip 2D input')
            require(metadata[0] not in self.textures, 'duplicate texture slot')
            self.words(2)  # row and slice pitches
            self.textures[metadata[0]] = (offset, metadata, self.blob())
        count, = self.words(1)
        require(count <= 16, 'too many samplers')
        self.take(count * 56)
        self.before = self.blob()
        self.secondary = self.depth = None
        if self.data[:8] == b'SSA011E1':
            self.take(24)  # viewport
            count, = self.words(1)
            require(count <= 1, 'too many secondary targets')
            if count:
                self.words(2)
                self.secondary = self.blob()
            bound, = self.words(1)
            require(bound <= 1, 'invalid depth binding')
            if bound:
                self.words(1)
                self.blob()  # depth/stencil state
                self.depth = self.blob()
        else:
            self.words(2)
            self.blob()
            self.depth = self.blob()
        require(self.at == len(self.data), 'trailing packet data')

    def take(self, size):
        require(0 <= size <= len(self.data) - self.at, 'truncated packet')
        span = slice(self.at, self.at + size)
        self.at += size
        return span

    def words(self, count):
        return struct.unpack('<' + 'I' * count, self.data[self.take(count * 4)])

    def blob(self):
        size, = self.words(1)
        require(0 < size <= 64 * 1024 * 1024, 'invalid blob size')
        return self.take(size)

    def replace(self, span, data):
        require(span is not None and span.stop - span.start == len(data), 'replacement extent differs')
        self.data[span] = data


def prepare(packets, snapshot, output):
    require(not output.exists(), 'refusing to overwrite replay evidence')
    files = {}

    def read(path):
        data = path.read_bytes()
        files[str(path.resolve())] = hashlib.sha256(data).hexdigest()
        return data

    def part(suffix):
        return read(Path(str(snapshot) + suffix))

    metadata = json.loads(part('.json'))
    require(type(metadata['history_reset']) is bool, 'invalid history state')
    globals_data = part('.globals.bin')
    require(len(globals_data) == 304, 'global constants differ')
    width, height, _, _ = struct.unpack_from('<4f', globals_data, 288)
    rows = json.loads(part('.targets.json'))
    require(len(rows) == 10 and [r['format'] for r in rows] == [34, 49, 49, 54, 24, 49, 54, 54, 34, 44],
            'snapshot texture formats differ')
    textures = []
    for slot, row in enumerate(rows):
        scale = 2 if slot < 2 else 1
        require(row['slot'] == slot and row['width'] * scale == width and row['height'] * scale == height and
                row['mip_levels'] == row['array_size'] == 1 and row['written'], 'snapshot texture contract differs')
        data = part(f'.rt{slot:02}.bin')
        require(len(data) == row['width'] * row['height'] * row['bytes_per_pixel'], 'snapshot extent differs')
        textures.append(data)
    if metadata['history_reset']:
        require(not any(textures[1]) and not any(textures[5]) and not any(textures[6]), 'reset retained history')
    vertices = part('.vertices.bin')
    parameters = {i: part(f'.pass{i}.cb0.bin') for i in range(1, 4)}
    eye = parameters[1][16:28]
    require(eye == parameters[2][20:32] == parameters[3][16:28], 'eye constants differ between passes')
    x, y, z = struct.unpack('<3f', eye)
    accumulation = Packet(read(packets / 'accum.bin'))
    h = accumulation.header
    require(h[11] == 36 and h[13] == 57, 'eye-quad layout differs')
    first = h[14] + h[9] * 2
    require(first + 12 <= accumulation.indices.stop - accumulation.indices.start, 'eye indices out of bounds')
    indices = struct.unpack_from('<6H', accumulation.data, accumulation.indices.start + first)
    require(indices == tuple(indices[0] + i for i in (0, 1, 2, 3, 2, 1)), 'eye-quad topology differs')
    base = struct.unpack('<i', struct.pack('<I', h[10]))[0]
    vertex_offset = h[12] + (base + indices[0]) * 36
    require(0 <= vertex_offset and vertex_offset + 144 <= len(vertices), 'eye vertices out of bounds')
    for corner in range(4):
        expected = (x if corner < 2 else -x, y if corner % 2 else -y, z)
        require(struct.unpack_from('<3f', vertices, vertex_offset + corner * 36 + 16) == expected, 'accumulation eye ray is stale')
    prepared = {}
    for i, name in enumerate(NAMES):
        packet = Packet(read(packets / (name + '.bin')))
        require(hashlib.sha256(packet.data[packet.pixel_shader]).hexdigest() == SHADERS[i], 'original pixel shader differs')
        require(hashlib.sha256(packet.data[packet.vertex_shader]).hexdigest() == VERTEX_SHADERS[i], 'original vertex shader differs')
        packet.replace(packet.constants[1, 1], globals_data)
        if i:
            packet.replace(packet.constants[1, 0], parameters[i])
        else:
            packet.replace(packet.vertices, vertices)
        packet.replace(packet.textures[0][2], textures[7])
        if i == 1:
            packet.replace(packet.textures[1][2], textures[6])
        if i == 2:
            for slot, index in ((1, 0), (2, 5), (3, 1)):
                packet.replace(packet.textures[slot][2], textures[index])
            packet.replace(packet.secondary, textures[6])
            packet.replace(packet.before, b'\xff' * (packet.before.stop - packet.before.start))
        if i == 3:
            at, description, span = packet.textures[1]
            require(description[6:8] == (24, 24), 'original normal contract differs')
            struct.pack_into('<II', packet.data, at + 24, 34, 34)
            packet.replace(span, textures[8])
            packet.replace(packet.textures[2][2], textures[2])
            packet.replace(packet.before, bytes(packet.before.stop - packet.before.start))
        if packet.depth:
            packet.replace(packet.depth, textures[9])
        prepared[name + '.bin'] = packet.data
    output.mkdir(parents=True)
    for name, data in prepared.items():
        (output / name).write_bytes(data)
    for i in range(5):
        (output / f'reference-{i}.bin').write_bytes(textures[i])
    receipt = dict(files=files, capture=metadata,
                   packets={name: hashlib.sha256(data).hexdigest() for name, data in prepared.items()},
                   references=[hashlib.sha256(data).hexdigest() for data in textures[:5]],
                   scope='Original shaders on captured live inputs/history. Private targets; native composition unchanged.')
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return {key: value for key, value in receipt.items() if key != 'files'}


def compare(prepared, rendered):
    receipt = json.loads((prepared / 'receipt.json').read_text())
    checks = {}
    for suffix, index in OUTPUTS:
        expected = (prepared / f'reference-{index}.bin').read_bytes()
        require(hashlib.sha256(expected).hexdigest() == receipt['references'][index], 'reference changed')
        actual = Path(str(rendered) + suffix).read_bytes()
        require(len(actual) == len(expected), 'rendered extent differs')
        checks[str(index)] = dict(byte_identical=actual == expected,
                                 differing_bytes=sum(a != b for a, b in zip(actual, expected)),
                                 sha256=hashlib.sha256(actual).hexdigest())
    return dict(capture=receipt['capture'], checks=checks,
                all_targets_byte_identical=all(row['byte_identical'] for row in checks.values()))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    command = commands.add_parser('prepare')
    for name in ('packets', 'snapshot', 'output'):
        command.add_argument(name, type=Path)
    command = commands.add_parser('compare')
    for name in ('prepared', 'rendered'):
        command.add_argument(name, type=Path)
    args = parser.parse_args()
    result = prepare(args.packets, args.snapshot, args.output) if args.command == 'prepare' else compare(args.prepared, args.rendered)
    print(json.dumps(result, indent=2))
    if args.command == 'compare' and not result['all_targets_byte_identical']:
        raise SystemExit(1)
