#!/usr/bin/env python3
"""Prepare the remaining Radar 011e BC2 texture for native streaming validation."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from prepare_factory_day_material_flags import boolean, leaf
from prepare_factory_remains_bc3 import transcode

RESOURCE = 'textures/ported/v011e/door/door_metal.dds'
SOURCE_SHA = '4009f1b744d8f7a693178950cf5698ab980db746b1307d6727bc292a7be682d6'
MATERIAL = 'material_instances/ported/v011e/tiled/door/door_metal.material'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output already exists')
    manifest = json.loads((args.bundle / 'port-manifest.json').read_text())
    records = {entry['path']: entry for entry in manifest['files']}
    raw = (args.bundle / 'overlay' / RESOURCE).read_bytes()
    assert sha(raw) == SOURCE_SHA == records[RESOURCE]['sha256']
    material = (args.bundle / 'overlay' / MATERIAL).read_bytes()
    assert sha(material) == records[MATERIAL]['sha256']
    assert not boolean(material, 'material.g_stage.effect.use_alpha_test.value')[1]
    offset = leaf(material, 'material.g_stage.effect.texture_diffuse.value')
    assert struct.unpack_from('<H', material, offset + 20)[0] == 5
    offset = struct.unpack_from('<Q', material, offset)[0]
    assert material[offset:material.index(0, offset)].decode() == 'ported/v011e/door/door_metal'
    fixed, mips = transcode(raw, expected_shape=(1024, 1024, 9, 1))
    destination = args.output / 'overlay' / RESOURCE
    destination.parent.mkdir(parents=True)
    destination.write_bytes(fixed)
    result = {'format_version': 1, 'asset_id': 'radar-011e-door-streaming-candidate',
              'target_db_sha256': manifest['target_db_sha256'],
              'files': [{'path': RESOURCE, 'source': 'authored BC2 RGB blocks; BC3 alpha encoding',
                         'expected_sha256': SOURCE_SHA, 'sha256': sha(fixed), 'size': len(fixed)}]}
    (args.output / 'port-manifest.json').write_text(json.dumps(result, indent=2) + '\n')
    (args.output / 'audit.json').write_text(json.dumps({
        'state': 'prepared; native texture binding and image comparison pending',
        'material_sha256': sha(material), 'alpha_test': False, 'mips': mips,
        'scope': 'Exact RGB blocks, authored mip chain and sRGB preserved; no material changes.'
    }, indent=2) + '\n')
    print(json.dumps({'output': str(args.output), 'mips': len(mips), 'size': len(fixed)}))


if __name__ == '__main__':
    main()
