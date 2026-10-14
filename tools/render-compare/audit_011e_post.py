#!/usr/bin/env python3
"""Extract the original 011e post contracts without launching the game.

The index is `resources-db list SOURCE_DB` output. Program hashes and
authored parameters support manual equation review, not native visual parity.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct


def audit(database, index, output):
    decoder = Path(__file__).resolve().parents[1] / 'chemical-plant/audit_collision_region.py'
    spec = importlib.util.spec_from_file_location('collision_config', decoder)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    output.mkdir(parents=True, exist_ok=True)
    records, materials = [], {}
    with database.open('rb') as source:
        for line in index.read_text().splitlines():
            fields = line.split(maxsplit=4)
            if len(fields) != 5:
                continue
            _, size, stored, offset, resource = fields
            shader = any(resource.startswith('shaders/sm_4_0/' + family + '/')
                         for family in ('complex_post_process_blend.ps', 'present.ps'))
            material = resource in ('material_instances/post_process/level_03_day.material',
                                    'material_instances/post_process/level_04_day.material')
            if not shader and not material:
                continue
            if size != stored:
                raise ValueError('compressed entry requires archive decoder: ' + resource)
            source.seek(int(offset))
            data = source.read(int(size))
            if len(data) != int(size):
                raise ValueError('truncated resource: ' + resource)
            digest = hashlib.sha256(data).hexdigest()
            if material:
                decoded = module.config(data)['material']['post_process']
                values = {key: value['value'] for key, value in decoded.items()
                          if key.startswith('filmic_') or key in
                          ('frame_dark_desaturation_limit', 'image_grain_scale', 'vignette_power')}
                materials[resource] = dict(sha256=digest, values=values)
                continue
            start = data.index(b'DXBC')
            length, count = struct.unpack_from('<II', data, start + 24)
            code = data[start:start + length]
            if len(code) != length or 32 + 4 * count > length:
                raise ValueError('truncated DXBC')
            chunks = {}
            for at in struct.unpack_from('<' + 'I' * count, code, 32):
                if at + 8 > length:
                    raise ValueError('invalid chunk offset')
                chunk_size, = struct.unpack_from('<I', code, at + 4)
                if at + 8 + chunk_size > length:
                    raise ValueError('invalid chunk size')
                chunks[code[at:at + 4]] = code[at + 8:at + 8 + chunk_size]
            program = chunks.get(b'SHDR', chunks.get(b'SHEX'))
            if program is None:
                raise ValueError('missing shader program')
            sha = hashlib.sha256(code).hexdigest()
            (output / (sha + '.dxbc')).write_bytes(code)
            records.append(dict(resource=resource, archive_offset=int(offset),
                                resource_sha256=digest, dxbc_sha256=sha,
                                program_sha256=hashlib.sha256(program).hexdigest(), bytes=length))
    unique_post = {row['dxbc_sha256'] for row in records
                   if '/complex_post_process_blend.ps/' in row['resource']}
    if len(unique_post) != 9 or len(materials) != 2:
        raise ValueError('unexpected donor post inventory')
    # This is the manually decoded quality2/3 program used for the adapter.
    if '3eefb58778b7932d90b8f4af638dcc077ab6b59a9076bfa45e3e44de55f976af' not in unique_post:
        raise ValueError('reviewed original post program changed')
    result = dict(source=str(database), shaders=records, materials=materials,
                  scope='Original archive contracts only; runtime bindings and output parity require captures.')
    (output / 'source-contracts.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(unique_post_programs=len(unique_post), materials=len(materials))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('database', 'index', 'output'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    audit(args.database, args.index, args.output)
