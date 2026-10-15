#!/usr/bin/env python3
"""Pin what a fidelity run compared: databases, executables, hooks, presets,
poses and both sides' graphics settings (as written to the profiles).

    manifest.py --harness DIR --fidelity DIR --output JSON
                [--ref011e-set LABEL=RECIPE_DIR=MERGED_DIR ...]

Each --ref011e-set pins one set of original 0.11e references: its recipe,
profile settings, the profiles written back by every capture launch, and the
sha256 of every merged reference image. Without it, the single set
FIDELITY/ref011e + FIDELITY/ref-merged is pinned as before.
"""
import argparse
import hashlib
import json
from pathlib import Path

KEYS = ('r_graphics_quality', 'r_shading_quality', 'r_lighting_quality', 'r_shadow_quality', 'r_texture_quality',
        'r_post_process_quality', 'r_ambient_occlusion_quality', 'r_antialiasing_method', 'r_geometry_quality',
        'r_decorations_quality', 'r_ssao_screen_ratio', 'r_ssdo', 'r_resolution', 'r_fullscreen', 'r_max_anisotropic',
        'r_use_shader_lods', 'r_motion_blur_quality', 'r_gamma_correction_factor', 'r_particles_quality',
        'r_use_motion_blur', 'r_use_motion_vectors_in_taa', 'r_use_temporal_antialiasing')


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        while block := f.read(8 << 20):
            h.update(block)
    return h.hexdigest()


def settings(path):
    out = {}
    for line in Path(path).read_text(encoding='cp1251', errors='replace').splitlines():
        parts = line.split(' ', 1)
        if parts[0] in KEYS:
            out[parts[0]] = parts[1] if len(parts) > 1 else ''
    return out


def reference_set(recipe_dir, merged):
    """Pin one original 0.11e reference set: inputs, settings and every merged image."""
    recipe = json.loads((recipe_dir/'recipe.json').read_text())
    record = {'recipe_dir': str(recipe_dir), 'merged_dir': str(merged),
              'source_version': recipe['source_version'], 'executable_sha256': recipe['source_executable_sha256'],
              'database': recipe['database'], 'pinned_files': recipe['pinned_files'], 'capture': recipe.get('capture'),
              'settings': settings(recipe_dir/'reference-user.cfg'), 'profile_changes': recipe.get('profile_changes'),
              'profiles_after': {}, 'images': {}}
    for side in [merged/m for m in ('chemical', 'radar') if (merged/m).is_dir()]:
        images = sorted(side.glob('*.bmp'))
        record['images'][side.name] = {i.name: sha(i) for i in images}
        for capture in sorted({i.resolve().parent.parent for i in images}):
            after = capture/'profile-after.cfg'
            if after.exists():
                record['profiles_after'][str(capture)] = settings(after)
    return record


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--harness', type=Path, required=True)
    ap.add_argument('--fidelity', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--arms', default='fidelity.json', help='arms file under HARNESS/arms that produced the port captures')
    ap.add_argument('--ref011e-set', action='append', default=[], metavar='LABEL=RECIPE_DIR=MERGED_DIR')
    a = ap.parse_args()
    h, f = a.harness, a.fidelity
    record = json.loads((h/'harness.json').read_text())
    port = {
        'database_sha256': record['database_sha256'],
        'client_exe_sha256': sha(h/'client/binaries/x86/survarium-capture.exe'),
        'server_sha256': sha(h/'server/match-server-026'),
        'arms': {},
    }
    port['arms_file'] = a.arms
    for arm in json.loads((h/'arms'/a.arms).read_text())['arms']:
        preset = json.loads((h/'arms'/arm['preset']).read_text())
        port['arms'][arm['name']] = {
            'map': arm['map'],
            'preset_sha256': sha(h/'arms'/arm['preset']),
            'hook_sha256': sha(h/preset['hook']),
            'assets_sha256': {k: sha(h/v) if (h/v).is_file() else 'directory' for k, v in preset['assets'].items()},
            'settings': settings(h/preset['config']),
        }
    sets = [s.split('=', 2) for s in a.ref011e_set] or [['default', str(f/'ref011e'), str(f/'ref-merged')]]
    refs011e = {label: reference_set(Path(recipe_dir), Path(merged)) for label, recipe_dir, merged in sets}
    prepare = json.loads(Path('/tmp/factory-020e-reference-ready/prepare.json').read_text())
    ref020e = {'executable_sha256': prepare.get('exe_sha256'), 'hook_sha256': prepare.get('hook_sha256'),
               'source_game': prepare.get('source_game')}
    during = f/'ref020e/profile-during.cfg'
    if during.exists():
        ref020e['settings'] = settings(during)
    r20 = f/'ref-merged/r20'
    if r20.is_dir():
        ref020e['images'] = {i.name: sha(i) for i in sorted(r20.glob('*.bmp'))}
    poses = {p.stem: sha(p) for p in sorted((f/'poses').glob('*.json'))}
    ref011e = refs011e[sets[0][0]] if len(refs011e) == 1 else refs011e
    out = {'port': port, 'original_011e': ref011e, 'original_020e': ref020e, 'poses_sha256': poses,
           'frames_per_pose': 5, 'resolution': '1280x720 (the user plays -next at 1280x720 in a 1296x749 window)',
           'grain': 'not disabled on either side; removed by the per-pixel median of 5 frames',
           'sky': 'clouds are not phase-synchronised; sky pixels are flagged and excluded from the non-sky score',
           'settings_alignment': 'texture 2 on both; 0.11e post 3->2, decorations 2->1, motion blur 3->2, preset 4->5 (custom) to match the port '
                                 '(a set whose settings say r_use_motion_blur off also disables 0.11e motion blur, which scales the frame by (N+1)/N); '
                                 '0.20e texture 2 (it rejects 3)'}
    a.output.write_text(json.dumps(out, indent=1)+'\n')
    print(json.dumps({k: (v if k != 'port' else {'database': v['database_sha256'][:12]}) for k, v in out.items() if k in ('port', 'frames_per_pose')}))


if __name__ == '__main__':
    main()
