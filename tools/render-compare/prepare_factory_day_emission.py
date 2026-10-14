#!/usr/bin/env python3
"""Prepare, never apply, the original Factory lamp emission switches."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

RESOURCE = 'material_instances/ported/v010w/props/light/lamp_greenhouse_clone.material'
FLAGS = ('use_emissive', 'use_emissive_map')
CONSTANTS = ('constant_fresnel', 'constant_fresnel_min', 'constant_roughness_min')


def leaf(data, path):
    offset = 0
    for part in path.split('.'):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        if kind != 3:
            raise ValueError('expected named table: ' + path)
        matches = []
        for i in range(count):
            child = table + i * 24
            name = struct.unpack_from('<Q', data, child + 8)[0]
            if data[name:data.index(0, name)].decode() == part:
                matches.append(child)
        if len(matches) != 1:
            raise ValueError('missing or duplicate field: ' + path)
        offset = matches[0]
    return offset


def main(args):
    args.output.mkdir(parents=True, exist_ok=False)
    def run(*parameters):
        return subprocess.check_output([str(args.porter), *map(str, parameters)], text=True)
    def leaves(db, resource):
        return dict(line.split('\t', 1) for line in run('dump-db-config', '--db', db,
                    '--path', resource, '--leaves').splitlines())
    day = leaves(args.archive, RESOURCE)
    rain = leaves(args.archive, RESOURCE.replace('v010w', 'v010b'))
    source = leaves(args.source, RESOURCE.replace('ported/v010w/', ''))
    paths = ['material.g_stage.effect.' + flag + '.value' for flag in FLAGS]
    allowed = {'material.g_stage.effect.' + name + '.value' for name in CONSTANTS}
    differences = {}
    for path in day.keys() | rain.keys():
        a = day.get(path, '').replace('v010w', 'v010b')
        b = rain.get(path, '')
        if a != b:
            differences[path] = [a, b]
            if path not in allowed | set(paths):
                raise ValueError('unverified difference from working rain permutation: ' + path)
    # This exact shader was observed in four emitting rain-lamp draws.
    native_shader = 'a2e26698335ab986a7241bcc68291a10'
    shader_matches = []
    prefix = 'shaders/sm_4_0/gbuffer_emissive_pass.ps/'
    for resource in run('list', '--db', args.archive, '--prefix', prefix).splitlines():
        if resource.rsplit('/', 1)[1][15] != '2':
            continue
        temporary = args.output / 'native-shader.tmp'
        run('extract', '--db', args.archive, '--path', resource, '--output', temporary)
        data = temporary.read_bytes()
        temporary.unlink()
        start = data.index(b'DXBC')
        size = struct.unpack_from('<I', data, start + 24)[0]
        code = data[start:start + size]
        if size == 2584 and code[4:20].hex() == native_shader:
            shader_matches.append(resource)
            (args.output / 'live-rain-emission.dxbc').write_bytes(code)
    if not shader_matches:
        raise ValueError('previously observed rain-lamp shader is absent from package')
    baseline = args.output / 'baseline.material'
    run('extract', '--db', args.archive, '--path', RESOURCE, '--output', baseline)
    raw = baseline.read_bytes()
    fixed = bytearray(raw)
    changes = []
    for path in paths:
        if (day[path], source[path], rain[path]) != ('bool:false', 'bool:true', 'bool:true'):
            raise ValueError('unexpected emission state: ' + path)
        at = leaf(raw, path)
        if struct.unpack_from('<H', raw, at + 20)[0] != 0 or struct.unpack_from('<Q', raw, at)[0] != 0:
            raise ValueError('expected false boolean: ' + path)
        struct.pack_into('<Q', fixed, at, 1)
        changes.append({'path': path, 'offset': at, 'before': False, 'after': True})
    (args.output / 'fixed.material').write_bytes(fixed)
    stat = run('stat', '--db', args.archive, '--path', RESOURCE)
    if 'compressed: false, inlined: false' not in stat:
        raise ValueError('same-length patch requires an uncompressed external resource')
    position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
    with args.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    needle = struct.pack('<I4xQ', len(raw), position)
    offset = fat.find(needle)
    if offset < 0 or fat.find(needle, offset + 1) >= 0:
        raise ValueError('ambiguous resource index record')
    report = {'scope': 'Two original lamp emission switches; no transforms or light cones changed. '
              'Candidate only, not applied. All selectors match the previously running rain lamp material; '
              'three scalar differences are runtime constants. Runtime day validation remains required.',
              'day_rain_differences_before': differences,
              'live_rain_emission_dxbc': native_shader, 'native_shader_resources': shader_matches,
              'entries': [{'resource': RESOURCE, 'position': position, 'crc_offset': offset + 40,
                'baseline': 'baseline.material', 'fixed': 'fixed.material',
                'baseline_sha256': hashlib.sha256(raw).hexdigest(),
                'fixed_sha256': hashlib.sha256(fixed).hexdigest(), 'changes': changes}]}
    (args.output / 'patch.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'plan': str(args.output / 'patch.json'), 'switches': len(changes)}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('archive', 'source', 'porter', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    main(parser.parse_args())
