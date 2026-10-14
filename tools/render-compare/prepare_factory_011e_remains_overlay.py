#!/usr/bin/env python3
"""Prepare isolated v011e remains candidates before packing; never edit inputs."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

from prepare_factory_day_material_flags import boolean, leaf
from prepare_factory_remains_bc3 import SOURCE_HASHES, transcode


MATERIAL_HASHES = {
    'skeleton': 'bdf290e5fb5a29d1633e66c66ea90dd975604256bdc47a1a2a7cbf2318654a37',
    'rag': '39ff92b170205bca38b721c2ffb10d865a8172971cb7fc12a8beedcfa5bd5cb1',
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def extract(porter, archive, resource, output):
    subprocess.run([str(porter), 'extract', '--db', str(archive), '--path', resource,
                    '--output', str(output)], check=True, stdout=subprocess.DEVNULL)
    return output.read_bytes()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-db', type=Path, required=True)
    parser.add_argument('--native-db', type=Path, required=True)
    parser.add_argument('--overlay', type=Path, required=True)
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assert args.output.resolve() != args.overlay.resolve()
    report = {'state': 'prepared, not runtime validated or promoted',
              'namespace': 'ported/v011e', 'input_overlay': str(args.overlay), 'entries': [],
              'warning': 'Remains material diffuse-lighting parity is unresolved; alpha filtering differs slightly.'}
    with tempfile.TemporaryDirectory(prefix='factory-011e-contract-') as temporary:
        scratch = Path(temporary)
        for quality in (0, 3):
            path = ('shaders/sm_4_0/gbuffer_pass.ps/'
                    f'_{quality}___1101_00101_00____1_____0__0_00000____00000____000__________0__')
            raw = extract(args.porter, args.native_db, path, scratch/'shader')
            start = raw.index(b'DXBC')
            assert raw[start+4:start+20].hex() == 'e900e3e72a32c961f56a73ed8393a6c7'
            assert struct.unpack_from('<I', raw, start+24)[0] == 4120
        for name in ('skeleton', 'rag'):
            original_path = f'material_instances/characters/human/remains/{name}.material'
            authored = extract(args.porter, args.source_db, original_path, scratch/'material')
            assert digest(authored) == MATERIAL_HASHES[name]
            resource = f'material_instances/ported/v011e/characters/human/remains/{name}.material'
            baseline = (args.overlay/resource).read_bytes()
            fixed = bytearray(baseline)
            prefix = 'material.g_stage.effect.'
            assert not boolean(authored, prefix+'use_troughness.value')[1]
            assert boolean(authored, prefix+'use_tspecular_intensity.value')[1]
            assert boolean(baseline, prefix+'use_troughness.value')[1]
            offset, enabled = boolean(baseline, prefix+'use_tspecular_intensity.value')
            assert not enabled
            struct.pack_into('<Q', fixed, offset, 1)
            changes = ['use_tspecular_intensity: false -> true']
            for field, expected in [('constant_roughness_min', .1), ('constant_roughness_max', .2)]:
                offset = leaf(baseline, prefix+field+'.value')
                assert struct.unpack_from('<H', baseline, offset+20)[0] == 2
                assert abs(struct.unpack_from('<f', baseline, offset)[0]-expected) < 1e-7
                struct.pack_into('<f', fixed, offset, .5)
                changes.append(f'{field}: {expected} -> authored constant0.5')
            output = args.output/'material-overlay'/resource
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(fixed)
            report['entries'].append({'resource': resource, 'group': 'material-overlay',
                                      'baseline_sha256': digest(baseline), 'fixed_sha256': digest(fixed),
                                      'source_sha256': digest(authored), 'changes': changes})
            resource = f'textures/ported/v011e/character/human/remains/{name}.dds'
            original_path = f'textures/character/human/remains/{name}.dds'
            authored = extract(args.porter, args.source_db, original_path, scratch/'texture')
            baseline = (args.overlay/resource).read_bytes()
            assert baseline == authored and digest(authored) == SOURCE_HASHES[name]
            fixed, mips = transcode(baseline)
            output = args.output/'texture-overlay'/resource
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(fixed)
            report['entries'].append({'resource': resource, 'group': 'texture-overlay',
                                      'baseline_sha256': digest(baseline), 'fixed_sha256': digest(fixed),
                                      'source_sha256': digest(authored), 'mips': mips})
    (args.output/'manifest.json').write_text(json.dumps(report, indent=2)+'\n')
    # Standard compose inputs; hard links avoid duplicating the DDS payloads.
    # The porter's current compose implementation gives its FIRST bundle
    # precedence, so these belong before the unmodified base bundle.
    for group in ('material', 'texture'):
        bundle = args.output/f'{group}-bundle'
        files = []
        for entry in report['entries']:
            if entry['group'] != f'{group}-overlay':
                continue
            source = args.output/entry['group']/entry['resource']
            target = bundle/'overlay'/entry['resource']
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                target.unlink()
            target.hardlink_to(source)
            files.append({'path': entry['resource'], 'source': '011e-source-contract-remains-candidate',
                          'size': source.stat().st_size, 'sha256': entry['fixed_sha256']})
        (bundle/'port-manifest.json').write_text(json.dumps({
            'format_version': 1, 'asset_id': f'factory-011e-remains-{group}',
            'files': files, 'candidate_manifest': '../manifest.json'}, indent=2)+'\n')
    print(json.dumps({'output': str(args.output), 'entries': len(report['entries'])}))


if __name__ == '__main__':
    main()
