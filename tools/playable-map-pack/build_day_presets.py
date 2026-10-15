#!/usr/bin/env python3
"""Build per-donor hook presets for maps 194/195/196 in each named look.

Looks (see docs/investigation/day-map-calibration-20261015.md):
  source                  0.11e LUT/geometry fog/clouds, 0.11e post tail,
                          god rays as in the source (on).
  legacy-010b-atmosphere  the 2026-10-15 snapshot's LUT, fog, clouds, tone
                          curve and god-ray choice.
Bug fixes are applied to every look: hook with resolution-independent SSAO,
donor 0.11e SSAO shader/noise/authored parameters, 0.11e probe gather and
sort, 0.11e cubic probe falloff over the source skylight, remains sun
sentinel plus F0=1 local diffuse, encoded local lights, Radar cone opacity
and the 0.20e ambient adapter. `--probe-specular` adds the opt-in source 0.11e
probe specular producer (an A/B candidate, not part of either look).

Every input is pinned by SHA-256; shaders are compiled from committed
sources with FXC (/WX /O3 ps_4_0). Nothing outside OUTPUT is written.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

SHADERS = Path(__file__).resolve().parents[1]/'render-compare/shaders'
SNAPSHOT_SUMS = 'SHA256SUMS'
SKYLIGHT = Path(__file__).resolve().parent/'source-skylight'
SKYLIGHT_SHA256 = {  # skylight_parameters0..7 captured from the original 0.11e skylight draw (day weather)
    'chemical-011e-day': '5bdf6987edd1465d0dccb3eddf194a271541fd84d3822502f34231b284759b5d',
    'radar-011e-day': 'c440f33e565ef28dc19d3aae3f52b10eb5b93fbf6f5461ec7c976da6fd126bcf',
}
SSAO_011E = {  # donor 0.11e archive entries (quality 3 accumulation, upsample, rotation)
    'old-ssao-3.dxbc': ('shaders/sm_4_0/ssao_accumulation.ps/_3__________________________________________________________________', '0420583bf092cbf6d2cda26fa446e58be777980f32c5bc7af40649317a562f85'),
    'old-ssao-upsample.dxbc': ('shaders/sm_4_0/ssao_filter_upsample.ps/____________________________________________________________________', 'd52c8337fe60dbbd1894fd2166a1281e8bd77af5b053cb5dd23e5694ca608e4e'),
}
SSAO_NOISE = ('textures/engine/ssao_rotate.dds', '0a1ee4b719ca93154d04b86b3f76f74d884c7a6a695ffaea95c896a7551938e7')
SSAO_PARAMETERS = {'chemical-011e-day': '1.3,1.3', 'radar-011e-day': '1.5,1.5'}  # post_process level_04/03_day
WEATHER = {'chemical-011e-day': 'LEGACY_WEATHER_CHEMICAL_DAY', 'radar-011e-day': 'LEGACY_WEATHER_DAY'}
PINNED = {
    'cone/cone.dxbc': 'c6f5566ccfc47c9fbba5eb94e2385aef465b97c78797dde619f27b2aea75c896',
    'cone/old-spot_falloff.dds': '2aaabea0da1d8e5402ca72ebc1dd45af278580c361af8213ef03ab2d7382d50b',
    'cone/old-sphere_falloff.dds': '7cb7da5821e77a34e0b0f579ad08c169f20a392b5459d26a154632b49ba8413f',
    'ambient-020e.ps': '94b548fecc000ea0b7cbf04eeeb1678afbefcfe5dce699fa51b6cbbd4942f07e',
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def dxbc(data):
    offset = data.index(b'DXBC')
    return data[offset:offset+int.from_bytes(data[offset+24:offset+28], 'little')]


class Builder:
    def __init__(self, args):
        self.args = args
        self.compiled = {}

    def fxc(self, source, defines):
        key = (source, tuple(defines))
        if key in self.compiled:
            return self.compiled[key]
        out = self.args.output/'.shaders'/(Path(source).stem+''.join('-'+d.split('=')[0] for d in defines)+'.dxbc')
        out.parent.mkdir(parents=True, exist_ok=True)
        win = lambda p: 'Z:'+str(Path(p).resolve()).replace('/', '\\')
        command = [str(self.args.wine), str(self.args.fxc), '/nologo', '/WX', '/T', 'ps_4_0', '/E', 'main', '/O3']
        for define in defines:
            command += ['/D', define]
        command += ['/Fo', win(out), win(SHADERS/source)]
        subprocess.run(command, env=self.args.env, check=True, stdout=subprocess.DEVNULL)
        self.compiled[key] = out
        return out

    def extract(self, resource, expected, destination, shader):
        temporary = destination.with_suffix('.raw')
        subprocess.run([str(self.args.porter), 'extract', '--db', str(self.args.donor011e), '--path', resource,
                        '--output', str(temporary)], check=True, stdout=subprocess.DEVNULL)
        data = temporary.read_bytes()
        temporary.unlink()
        data = dxbc(data) if shader else data[128:192] if resource.endswith('.dds') else data
        if hashlib.sha256(data).hexdigest() != expected:
            raise ValueError(f'{resource}: donor bytes differ from the pinned asset')
        destination.write_bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--snapshot', type=Path, required=True, help='legacy look snapshot directory')
    parser.add_argument('--hook', type=Path, required=True)
    parser.add_argument('--donor011e', type=Path, required=True, help='0.11e resources.db')
    parser.add_argument('--porter', type=Path, required=True)
    parser.add_argument('--pinned', type=Path, required=True, help='directory holding the PINNED assets')
    parser.add_argument('--local-fresnel-one', type=Path, required=True, help='generated F0=1 shader directory')
    parser.add_argument('--wine', type=Path, required=True)
    parser.add_argument('--fxc', type=Path, required=True)
    parser.add_argument('--wineprefix', type=Path, required=True)
    parser.add_argument('--probe-specular', action='store_true')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output exists')
    args.env = {'WINEPREFIX': str(args.wineprefix), 'WINEDEBUG': '-all', 'DISPLAY': ':0',
                'WINEDLLOVERRIDES': 'winemenubuilder.exe=d;mscoree,mshtml=', 'PATH': '/run/current-system/sw/bin'}
    for line in (args.snapshot/SNAPSHOT_SUMS).read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        if sha(args.snapshot/name.lstrip('*').lstrip('./')) != digest:
            raise ValueError(f'snapshot file changed: {name}')
    for name, digest in PINNED.items():
        if sha(args.pinned/name) != digest:
            raise ValueError(f'pinned asset changed: {name}')
    builder = Builder(args)
    common = args.output/'common'
    (common/'ssao-011e').mkdir(parents=True)
    for name, (resource, digest) in SSAO_011E.items():
        builder.extract(resource, digest, common/'ssao-011e'/name, True)
    builder.extract(SSAO_NOISE[0], SSAO_NOISE[1], common/'ssao-011e/old-ssao-noise.bgra', False)
    shutil.copyfile(args.snapshot/'presets/radar-011e-day/assets/SRP_LEGACY_SSAO_DIR/fullscreen-vs.dxbc',
                    common/'ssao-011e/fullscreen-vs.dxbc')
    shutil.copytree(args.pinned/'cone', common/'cone')
    shutil.copytree(args.local_fresnel_one, common/'local-fresnel-one')
    shutil.copyfile(args.pinned/'ambient-020e.ps', common/'ambient-020e.ps')
    shutil.copyfile(args.hook, common/'camera_hook.dll')
    looks = ['source', 'legacy-010b-atmosphere']
    record = {'looks': {}, 'probe_specular': args.probe_specular}
    for look in looks:
        for key in ('chemical-011e-day', 'radar-011e-day', 'radar-020e-day'):
            directory = args.output/look/'presets'/key
            directory.mkdir(parents=True)
            snapshot = args.snapshot/'presets'/key
            shutil.copyfile(snapshot/'user.cfg', directory/'user.cfg')
            rel = lambda p: str(Path(p).relative_to(args.output/look)) if Path(p).is_relative_to(args.output/look) else None
            preset = {'environment': {}, 'assets': {}, 'hook': f'presets/{key}/camera_hook.dll',
                      'config': f'presets/{key}/user.cfg'}
            shutil.copyfile(common/'camera_hook.dll', directory/'camera_hook.dll')
            assets = directory/'assets'
            assets.mkdir()

            def put(name, source):
                target = assets/name
                if Path(source).is_dir():
                    shutil.copytree(source, target)
                else:
                    shutil.copyfile(source, target)
                preset['assets'][name] = f'presets/{key}/assets/{name}'

            if key == 'radar-020e-day':
                preset['environment']['SRP_LEGACY_020E_AMBIENT'] = '1'
                put('SRP_LEGACY_020E_AMBIENT_APPLY', common/'ambient-020e.ps')
            else:
                old = json.loads((snapshot/'preset.json').read_text())
                for name in old['assets']:
                    put(name, args.snapshot/old['assets'][name])
                weather = WEATHER[key]
                # Renderer contract, all looks: 0.11e probes use cubic falloff
                # (011e DXBC) and lerp over the source skylight, which the hook
                # writes encoded before the first probe. The inherited 011a
                # exponential falloff let the global sphere and box probes cover
                # pixels the original leaves to the skylight.
                probe_defines = ['CONFIG_SOURCE_PROBE_SPECULAR=1'] if args.probe_specular else []
                put('SRP_LEGACY_PROBE_DIFFUSE', builder.fxc('legacy_probe_011e_falloff.hlsl', probe_defines))
                put('SRP_LEGACY_PROBE_DIFFUSE_BOX', builder.fxc('legacy_probe_011e_falloff_box.hlsl', probe_defines))
                underlay = args.output/'.shaders'/f'skylight-underlay-{key}'
                underlay.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(builder.fxc('legacy_skylight_underlay.hlsl', []), underlay/'skylight-underlay.dxbc')
                # The 32 floats 0.11e uploads as skylight_parameters0..7 for this
                # map's day weather, captured from the original skylight draw.
                constants = SKYLIGHT/f'{key}.bin'
                if sha(constants) != SKYLIGHT_SHA256[key]:
                    raise SystemExit(f'{constants}: pinned skylight constants changed')
                shutil.copyfile(constants, underlay/'source-skylight.bin')
                put('SRP_LEGACY_SKYLIGHT_UNDERLAY', underlay)
                put('SRP_LEGACY_COMPOSITION', builder.fxc('legacy_composition_sun_sentinel.hlsl', []))
                put('SRP_LOCAL_FRESNEL_ONE_DIR', common/'local-fresnel-one')
                # Renderer contract, all looks: local lights add in sqrt-encoded space.
                put('SRP_LEGACY_ENCODED_LIGHTS', builder.fxc('legacy_encoded_lights.hlsl', []))
                shutil.rmtree(assets/'SRP_LEGACY_SSAO_DIR')
                put('SRP_LEGACY_SSAO_DIR', common/'ssao-011e')
                preset['environment']['SRP_LEGACY_SSAO_PARAMETERS'] = SSAO_PARAMETERS[key]
                preset['environment']['SRP_LEGACY_PROBE_SORT_011A'] = '1'
                preset['environment']['SRP_CAPTURE_BACKGROUND_RENDER'] = '1'
                if key == 'radar-011e-day':
                    put('SRP_LEGACY_VOLUME_CONE', common/'cone/cone.dxbc')
                    put('SRP_LEGACY_VOLUME_ASSETS', common/'cone')
                if look == 'source':
                    put('SRP_LEGACY_ATMOSPHERE_LUT', builder.fxc('legacy_atmosphere_lut.hlsl', ['LEGACY_SOURCE_011E=1']))
                    put('SRP_LEGACY_GEOMETRY_FOG', builder.fxc('legacy_geometry_fog.hlsl', ['LEGACY_SOURCE_011E=1']))
                    put('SRP_LEGACY_RAIN_CLOUDS', builder.fxc('legacy_rain_clouds.hlsl',
                        [weather+'=1', 'LEGACY_SOURCE_011E_CLOUDS=1']))
                    post = ['LEGACY_WEATHER_CHEMICAL_DAY=1'] if key == 'chemical-011e-day' else []
                    put('SRP_LEGACY_RAIN_TONEMAP', builder.fxc('legacy_011e_post_tail.hlsl', post))
                elif key == 'radar-011e-day':
                    preset['environment']['SRP_LEGACY_DISABLE_GOD_RAYS'] = '1'
            (directory/'preset.json').write_text(json.dumps(preset, indent=2)+'\n')
        files = {str(p.relative_to(args.output/look)): sha(p)
                 for p in sorted((args.output/look).rglob('*')) if p.is_file()}
        (args.output/look/'manifest.json').write_text(json.dumps(files, indent=2)+'\n')
        record['looks'][look] = len(files)
    shutil.rmtree(args.output/'.shaders')
    record['inputs'] = {'hook': sha(args.hook), 'snapshot': str(args.snapshot), 'donor011e': str(args.donor011e)}
    (args.output/'build.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps(record, indent=2))


if __name__ == '__main__':
    main()
