#!/usr/bin/env python3
"""Build isolated scalar roughness textures for old-red to native-green sampling.

Copies each authored mip into uncompressed BGRA8 with RGB=source red, preserving
alpha and the source sRGB flag. Original textures and non-roughness bindings are untouched. Input DDS must
be a legacy DXT1/3/5 2D texture with Survarium's trailing sRGB byte.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile


def leaves(data, offset=0, path=''):
    value, name, crc, kind, count = struct.unpack_from('<QQIHH', data, offset)
    if kind == 3:
        for i in range(count):
            child = value + 24 * i
            name = struct.unpack_from('<Q', data, child + 8)[0]
            key = data[name:data.index(0, name)].decode('cp1251')
            yield from leaves(data, child, path + '.' + key if path else key)
    else:
        yield path, (offset, kind, value)


def string(data, node):
    _, kind, offset = node
    assert kind == 5
    return data[offset:data.index(0, offset)].decode('cp1251')


def set_string(data, node, value):
    offset, kind, _ = node
    assert kind == 5
    encoded = value.encode('cp1251') + b'\0'
    assert len(encoded) <= 65535
    struct.pack_into('<Q', data, offset, len(data))
    struct.pack_into('<H', data, offset + 22, len(encoded))
    data.extend(encoded)


def scalar_dds(data, ffmpeg):
    assert data[:4] == b'DDS ' and struct.unpack_from('<I', data, 4)[0] == 124
    height, width = struct.unpack_from('<II', data, 12)
    mips = struct.unpack_from('<I', data, 28)[0] or 1
    assert width > 0 and height > 0 and 1 <= mips <= 16
    assert struct.unpack_from('<I', data, 112)[0] == 0, 'cube/volume not supported'
    fourcc = data[84:88]
    assert fourcc in (b'DXT1', b'DXT3', b'DXT5')
    block_bytes = 8 if fourcc == b'DXT1' else 16
    source_offset = 128
    output_mips, evidence = [], []
    with tempfile.TemporaryDirectory() as temporary:
        image = Path(temporary) / 'mip.dds'
        for level in range(mips):
            w, h = max(1, width >> level), max(1, height >> level)
            size = ((w + 3) // 4) * ((h + 3) // 4) * block_bytes
            payload = data[source_offset:source_offset + size]
            assert len(payload) == size
            header = bytearray(data[:128])
            struct.pack_into('<IIII', header, 12, h, w, size, 0)
            struct.pack_into('<I', header, 28, 1)
            image.write_bytes(header + payload)
            rgba = subprocess.check_output([str(ffmpeg), '-v', 'error', '-i', str(image),
                '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgba', '-'])
            assert len(rgba) == w * h * 4
            scalar = bytearray(rgba)
            scalar[1::4] = rgba[0::4]
            scalar[2::4] = rgba[0::4]
            assert scalar[0::4] == scalar[1::4] == scalar[2::4] == rgba[0::4]
            assert scalar[3::4] == rgba[3::4]
            output_mips.append(scalar)
            evidence.append({'level': level, 'width': w, 'height': h,
                'source_red_sha256': hashlib.sha256(rgba[0::4]).hexdigest(),
                'output_rgb_matches_source_red': True})
            source_offset += size
    assert len(data) == source_offset + 1 and data[-1] in (0, 1), 'unexpected DDS tail'
    header = bytearray(128)
    header[:4] = b'DDS '
    struct.pack_into('<7I', header, 4, 124, 0x2100f, height, width, width * 4, 0, mips)
    struct.pack_into('<8I', header, 76, 32, 0x41, 0, 32, 0xff0000, 0xff00, 0xff, 0xff000000)
    struct.pack_into('<I', header, 108, 0x401008 if mips > 1 else 0x1000)
    # The captured native roughness binding can use an sRGB view. Preserve the
    # authored transfer flag: treating the encoded red bytes as linear changes
    # smoothness far more than the intended red/green channel substitution.
    return bytes(header) + b''.join(output_mips) + data[-1:], evidence


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--archive', required=True, type=Path)
    p.add_argument('--overlay', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--porter', required=True, type=Path)
    p.add_argument('--ffmpeg', required=True, type=Path)
    p.add_argument('--texture', action='append', required=True, help='Original short texture path, e.g. flora/fir_translucence')
    a = p.parse_args()
    assert not a.output.exists(), 'refusing to overwrite a bundle'
    a.output.mkdir(parents=True)
    names = set(a.texture)
    assert all(n and not n.startswith('/') and all(c not in ('', '.', '..') for c in n.split('/')) for n in names)
    files, materials, textures = [], [], []
    def emit(path, data):
        destination = a.output / 'overlay' / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        files.append({'path': path, 'source': 'isolated-roughness-channel-adapter',
            'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
    def extract(resource, destination):
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(a.porter), 'extract', '--db', str(a.archive), '--path', resource,
            '--output', str(destination)], check=True, stdout=subprocess.DEVNULL)
        return destination.read_bytes()
    base = a.overlay / 'material_instances/ported/v010b'
    for path in sorted(base.rglob('*.material')):
        sample = path.read_bytes()
        nodes = dict(leaves(sample))
        texture_key = 'material.g_stage.effect.texture_roughness.value'
        flag = nodes.get('material.g_stage.effect.use_troughness.value')
        if texture_key not in nodes or not flag or flag[1] not in (0, 1) or not flag[2]:
            continue
        old_name = string(sample, nodes[texture_key])
        if old_name.removeprefix('ported/v010b/') not in names:
            continue
        resource = str(path.relative_to(a.overlay))
        current = extract(resource, a.output / 'baseline' / resource)
        nodes = dict(leaves(current))
        assert string(current, nodes[texture_key]) == old_name
        new_name = old_name.replace('ported/v010b/', 'ported/v010b/roughness-red/', 1)
        patched = bytearray(current)
        set_string(patched, nodes[texture_key], new_name)
        emit(resource, patched)
        materials.append({'resource': resource, 'old_binding': old_name, 'new_binding': new_name,
            'baseline_sha256': hashlib.sha256(current).hexdigest()})
    used = {m['old_binding'] for m in materials}
    assert used == {'ported/v010b/' + n for n in names}, 'each requested texture must be used by an enabled roughness input'
    for name in sorted(names):
        resource = 'textures/ported/v010b/' + name
        original = extract(resource + '.dds', a.output / 'baseline' / (resource + '.dds'))
        converted, levels = scalar_dds(original, a.ffmpeg)
        new_resource = resource.replace('ported/v010b/', 'ported/v010b/roughness-red/', 1)
        emit(new_resource + '.dds', converted)
        options = bytearray(extract(resource + '.options', a.output / 'baseline' / (resource + '.options')))
        nodes = dict(leaves(options)); set_string(options, nodes['options.format'], 'A8R8G8B8')
        _, kind, srgb = nodes['options.srgb']; assert kind in (0, 1)
        assert srgb == original[-1], 'DDS and options disagree on sRGB'
        emit(new_resource + '.options', options)
        textures.append({'source': resource, 'clone': new_resource,
            'source_sha256': hashlib.sha256(original).hexdigest(),
            'srgb': srgb, 'mips': levels})
    (a.output / 'port-manifest.json').write_text(json.dumps({'format_version': 1,
        'asset_id': 'isolated-original-roughness-channel-adapter', 'files': files}, indent=2) + '\n')
    (a.output / 'channel-audit.json').write_text(json.dumps({'materials': materials,
        'textures': textures, 'runtime_verified': False}, indent=2) + '\n')
    print(json.dumps({'materials': len(materials), 'textures': len(textures), 'files': len(files)}))


if __name__ == '__main__':
    main()
