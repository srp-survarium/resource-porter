#!/usr/bin/env python3
"""Extract unchanged weather DXBC and binding contracts from the original catalog.

No recompilation or shader substitution occurs. Archive blobs and embedded DXBC
must match the catalog hashes before they can enter the comparison fixture.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import struct

from reference_shader_index import FAMILIES


def reflection(code):
    count, = struct.unpack_from('<I', code, 28)
    chunks = {}
    for offset in struct.unpack_from('<' + 'I' * count, code, 32):
        size, = struct.unpack_from('<I', code, offset + 4)
        chunks[code[offset:offset+4]] = code[offset+8:offset+8+size]
    data = chunks[b'RDEF']

    def name(offset):
        return data[offset:data.index(0, offset)].decode()

    cb_count, cb_offset, resource_count, resource_offset = struct.unpack_from('<4I', data)
    resources, constants = {}, {}
    for index in range(resource_count):
        fields = struct.unpack_from('<8I', data, resource_offset + index * 32)
        resources[name(fields[0])] = dict(kind=fields[1], return_type=fields[2],
                                         dimension=fields[3], samples=fields[4],
                                         slot=fields[5], count=fields[6], flags=fields[7])
    for index in range(cb_count):
        label, count, offset, size, flags, kind = struct.unpack_from('<6I', data, cb_offset + index * 24)
        variables = {}
        for variable_index in range(count):
            variable = struct.unpack_from('<6I', data, offset + variable_index * 24)
            variables[name(variable[0])] = dict(offset=variable[1], size=variable[2],
                                                used=bool(variable[3] & 2))
        constants[name(label)] = dict(size=size, flags=flags, kind=kind, variables=variables)
    def signature(tag):
        chunk = chunks[tag]
        count, = struct.unpack_from('<I', chunk)
        result = []
        for index in range(count):
            label, semantic_index, system_value, component_type, register, mask, used = (
                struct.unpack_from('<5I2B2x', chunk, 8 + index * 24))
            semantic = chunk[label:chunk.index(0, label)].decode()
            result.append(dict(semantic=semantic, index=semantic_index,
                               system_value=system_value, component_type=component_type,
                               register=register, mask=mask, read_write_mask=used))
        return result

    return dict(resources=resources, constants=constants,
                input_signature_sha256=hashlib.sha256(chunks[b'ISGN']).hexdigest(),
                input_signature=signature(b'ISGN'), output_signature=signature(b'OSGN'))


def extract(database, catalog, output):
    if output.exists():
        raise ValueError('refusing to overwrite an existing source fixture')
    families = set(FAMILIES) | {'rain.ps', 'rain.vs', 'rain_resolve.ps', 'rain_resolve.vs',
                               'wet_sufrace.ps', 'wet_surface.ps', 'present.ps',
                               'complex_post_process_blend.vs', 'copy_image.vs', 'present.vs',
                               'post_process0.vs', 'post_process_fxaa.vs', 'skylight.vs', 'light.vs', 'accum_mask.vs'}
    shaders = {}
    with gzip.open(catalog, 'rt') as stream, database.open('rb') as archive:
        for line in stream:
            row = json.loads(line)
            if row['name'] not in families:
                continue
            archive.seek(row['offset'])
            blob = archive.read(row['blob_size'])
            if hashlib.sha256(blob).hexdigest() != row['blob_sha256']:
                raise ValueError('archive/catalog mismatch: ' + row['name'])
            start = blob.index(b'DXBC')
            size, = struct.unpack_from('<I', blob, start + 24)
            code = blob[start:start+size]
            digest = hashlib.sha256(code).hexdigest()
            if size != row['dxbc_size'] or digest != row['dxbc_sha256']:
                raise ValueError('embedded shader mismatch: ' + row['name'])
            if digest not in shaders:
                shaders[digest] = dict(name=row['name'], bytes=size, code=code,
                                       reflection=reflection(code), permutations=[])
            shaders[digest]['permutations'].append(dict(key=row['permutation'],
                                                        archive_offset=row['offset'],
                                                        blob_sha256=row['blob_sha256']))
    for digest in ('3eefb58778b7932d90b8f4af638dcc077ab6b59a9076bfa45e3e44de55f976af',
                   'e7ca858153aac900b1205eb8d3da914aa6aef06acee569f2b657076c59798f68',
                   'a7fc8c504884f2934fc257bea25fc5e8fb5f4032d69effeb44ffda2c4da02ceb'):
        if digest not in shaders:
            raise ValueError('missing reviewed 0.11e shader: ' + digest)
    output.mkdir(parents=True)
    for digest, row in shaders.items():
        filename = row['name'] + '-' + digest[:16] + '.dxbc'
        (output / filename).write_bytes(row.pop('code'))
        row['file'] = filename
    manifest = dict(source='original 0.11e build 884', database=str(database),
                    catalog_sha256=hashlib.sha256(catalog.read_bytes()).hexdigest(), shaders=shaders,
                    scope='Unmodified source bytecode and reflection; native bindings require adaptation.')
    (output/'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(dict(shaders=len(shaders), families=len({v['name'] for v in shaders.values()}),
                          output=str(output))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', type=Path, required=True)
    parser.add_argument('--catalog', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    extract(args.database, args.catalog, args.output)
