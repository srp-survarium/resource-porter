#!/usr/bin/env python3
"""Put each converted map's own minimap into the 0.26g HUD of a bundle.

    hud_minimaps.py --bundle BUNDLE --spec ports/soc-cs-dm-to-026g.json \
        --target-db RETAIL_026G/resources.db --ffdec FFDEC_DIR \
        [--java java --javac javac] [--work DIR]

1. Renders every spec map that has a `hud_key` with render_minimap.py from
   the bundle's own overlay (the converted geometry and textures).
2. Patches the retail flash_movies/hud.swf (it must be the build 2777
   original) with PatchHudMinimaps.java: one DefineBitsJPEG3 bitmap and one
   _minimapProps/_levels entry per hud_key, native entries untouched.
3. Writes overlay/flash_movies/hud.swf and records it in port-manifest.json
   `files`, so `vostok-port verify` checks the packed copy. The projects'
   project_name is the hud_key (set by build-survarium-maps-026).

Requires a JDK and JPEXS FFDec (its lib/ directory). Every other tag of
hud.swf is checked to stay byte-identical. Inputs are never modified,
except the bundle, which gains the HUD.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import render_minimap  # noqa: E402

HUD = 'flash_movies/hud.swf'
HUD_SHA256 = '710c9ca6b3336190635573ec7ad51bcce93f1d7acebf9f3e3a2f338663a5bf5c'   # 0.26g build 2777
SOURCE = 'srp-hud-minimaps'


def read_db(db, path, vostok_port):
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / 'file'
        subprocess.run([vostok_port, 'extract', '--db', str(db), '--path', path, '--output', str(out)],
                       check=True, stdout=subprocess.DEVNULL)
        return out.read_bytes()


def class_name(key):
    return 'SrpMinimap_' + key


def encode(png, work, key):
    """Premultiplied baseline JPEG + raw alpha, as the native minimap_base."""
    rgba = np.asarray(Image.open(png).convert('RGBA'), np.float32)
    alpha = rgba[..., 3]
    rgb = (rgba[..., :3] * alpha[..., None] / 255).round().clip(0, 255).astype(np.uint8)
    jpeg = work / f'{key}.jpg'
    Image.fromarray(rgb, 'RGB').save(jpeg, 'JPEG', quality=92, subsampling=0, optimize=False,
                                     progressive=False)
    raw = work / f'{key}.alpha'
    raw.write_bytes(alpha.round().astype(np.uint8).tobytes())
    return jpeg, raw


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--target-db', type=Path, required=True)
    ap.add_argument('--ffdec', type=Path, required=True, help='JPEXS FFDec directory (lib/ffdec_lib.jar)')
    ap.add_argument('--java', default='java')
    ap.add_argument('--javac', default='javac')
    ap.add_argument('--vostok-port', default='target/release/vostok-port')
    ap.add_argument('--work', type=Path)
    a = ap.parse_args()
    overlay = a.bundle / 'overlay'
    manifest_path = a.bundle / 'port-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    if any(f['path'] == HUD for f in manifest['files']):
        raise SystemExit('bundle already carries a HUD')
    spec = json.loads(a.spec.read_text())
    maps = [m for m in spec['maps'] if m.get('hud_key')]
    if not maps:
        raise SystemExit('no spec map declares a hud_key')
    work = a.work or Path(tempfile.mkdtemp(prefix='hud-minimaps-'))
    work.mkdir(parents=True, exist_ok=True)
    original = read_db(a.target_db, HUD, a.vostok_port)
    if hashlib.sha256(original).hexdigest() != HUD_SHA256:
        raise SystemExit('target hud.swf is not the 0.26g build 2777 original')
    (work / 'hud.swf').write_bytes(original)
    rows, records = [], []
    for m in maps:
        record = render_minimap.render_map(overlay, m, work / 'render')
        key = m['hud_key']
        jpeg, raw = encode(work / 'render' / f"{record['key']}.png", work, key)
        rows.append('\t'.join([key, class_name(key), str(jpeg), str(raw), str(record['width']),
                               str(record['height']), repr(record['size']), repr(record['posX']),
                               repr(record['posY'])]))
        records.append(dict(record, hud_key=key))
    (work / 'entries.tsv').write_text('\n'.join(rows) + '\n')
    lib = a.ffdec / 'lib' / 'ffdec_lib.jar'
    if not lib.is_file():
        raise SystemExit(f'no FFDec library at {lib}')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    subprocess.run([a.javac, '-encoding', 'UTF-8', '-cp', str(lib), '-d', str(classes),
                    str(HERE / 'hud_minimaps' / 'PatchHudMinimaps.java')], check=True)
    patched = work / 'hud-patched.swf'
    subprocess.run([a.java, '-Djava.awt.headless=true', f'-Duser.home={work}',
                    '-cp', f'{classes}{os.pathsep}{a.ffdec / "lib" / "*"}', 'PatchHudMinimaps',
                    str(work / 'hud.swf'), str(patched), str(work / 'entries.tsv')], check=True)
    sys.path.insert(0, str(HERE / 'hud_minimaps'))
    import swf_tags
    missing, added = swf_tags.compare(original, patched.read_bytes())
    if sorted(c for c, _ in missing) != [swf_tags.SYMBOL_CLASS, swf_tags.DO_ABC2] or \
            sorted(c for c, _ in added) != sorted([swf_tags.DEFINE_BITS_JPEG3] * len(maps)
                                                  + [swf_tags.SYMBOL_CLASS, swf_tags.DO_ABC2]):
        raise SystemExit('the HUD patch changed more than the Minimap ABC and the SymbolClass')
    data = patched.read_bytes()
    target = overlay / HUD
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    manifest['files'].append({'path': HUD, 'source': SOURCE, 'size': len(data),
                              'sha256': hashlib.sha256(data).hexdigest()})
    manifest.setdefault('compatibility_notes', []).append(
        f'{HUD}: own minimaps for ' + ', '.join(f"{r['hud_key']} (map {r['map_id']}: size {r['size']}, "
                                                f"posX {r['posX']}, posY {r['posY']})" for r in records)
        + '; native minimap entries unchanged')
    manifest['hud_minimaps'] = records
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    for r in records:
        shutil.copy2(work / 'render' / f"{r['key']}-preview.png", work / f"{r['hud_key']}-preview.png")
    print(json.dumps({'hud': HUD, 'sha256': hashlib.sha256(data).hexdigest(), 'maps': records}, indent=2))


if __name__ == '__main__':
    main()
