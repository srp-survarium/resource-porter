#!/usr/bin/env python3
"""Give converted maps their own loading screens in the 0.26g lobby.

    lobby_loading.py --bundle BUNDLE --spec ports/soc-cs-dm-to-026g.json \
        --target-db RETAIL_026G/resources.db --ffdec FFDEC_DIR \
        [--java java --javac javac] [--work DIR] [--no-grade]

The engine reads mode.ui.loading_screen_id from the map's game mode and
calls root.set_loading_level(id) in flash_movies/lobby_menu.swf, whose
LobbyMenu shows _load_images[id] (26 native images, ids 0-25). Every spec
map with a `loading_image` gets id NATIVE_IMAGES + k, k counting those maps
in spec order; build-survarium-maps-026 writes the same id into the map's
mode options.

Each image is prepared like the native ones: centre-cropped to 1920:798,
resized to 1920 x 798, and (unless --no-grade) tone-matched to the native
day images: luminance p5/p50/p95 to 0.09/0.30/0.61 with a highlight
shoulder at 0.80, mean saturation towards 0.27 and a mild unsharp mask. No
vignette or text: the SWF draws the LoadingTop/LoadingBottom frames (logo,
grunge edges) and the map/mode names over the image.

The retail lobby_menu.swf (build 2777, ab6f22d8...; the .orig beside it is
not loaded) is patched with hud_minimaps/PatchLobbyLoading.java (JPEXS
FFDec). Every other tag stays byte-identical, and every native method body
except LobbyMenu's constructor. The result goes into the bundle overlay and
its port-manifest files, so `vostok-port verify` checks it.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image, ImageFilter
from scipy.interpolate import PchipInterpolator

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LOBBY = 'flash_movies/lobby_menu.swf'
LOBBY_SHA256 = 'ab6f22d80d32fa4928403d19e3e054bd5316b2370e2550cefec0c9d22b959850'   # 0.26g build 2777
NATIVE_IMAGES = 26          # LobbyMenu._load_images: Level1_day .. Level8_prerain
WIDTH, HEIGHT = 1920, 798   # native levels 1-6 (7-8: 1920 x 680)
SOURCE = 'srp-lobby-loading'
LUMA = np.array([0.2126, 0.7152, 0.0722], np.float32)
# Native day images (sRGB): luminance percentiles and mean HSV saturation.
TARGET = {'p5': 0.09, 'p50': 0.30, 'p95': 0.61, 'white': 0.80, 'saturation': 0.27}


def loading_ids(spec):
    """{map id: loading_screen_id} for the spec maps with a loading_image."""
    ids, k = {}, 0
    for m in spec['maps']:
        if m.get('loading_image'):
            ids[m['id']] = NATIVE_IMAGES + k
            k += 1
    return ids


def crop(img):
    """Centre crop to WIDTH:HEIGHT, then resize to WIDTH x HEIGHT."""
    w, h = img.size
    if w * HEIGHT > h * WIDTH:
        cw = round(h * WIDTH / HEIGHT)
        box = ((w - cw) // 2, 0, (w - cw) // 2 + cw, h)
    else:
        ch = round(w * HEIGHT / WIDTH)
        box = (0, (h - ch) // 2, w, (h - ch) // 2 + ch)
    return img.crop(box).resize((WIDTH, HEIGHT), Image.LANCZOS)


def grade(img):
    """Deterministic native-like grade of an sRGB image (see module doc)."""
    a = np.asarray(img.convert('RGB'), np.float32) / 255
    lum = a @ LUMA
    p5, p50, p95 = np.percentile(lum, [5, 50, 95])
    if not 0 <= p5 < p50 < p95 < 1:
        return img.convert('RGB')                    # flat image: nothing to match
    xs = [0.0, p5, p50, p95, 1.0]
    ys = [0.0, TARGET['p5'], TARGET['p50'], TARGET['p95'], TARGET['white']]
    curve = PchipInterpolator(xs, ys)
    out = np.clip(curve(lum), 0, 1).astype(np.float32)
    ratio = np.where(lum > 1e-4, out / np.maximum(lum, 1e-4), 0)[..., None]
    b = a * ratio + (lum <= 1e-4)[..., None] * out[..., None]
    mx, mn = b.max(-1), b.min(-1)
    saturation = float(np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-6), 0).mean())
    s = float(np.clip(TARGET['saturation'] / max(saturation, 1e-3), 0.8, 1.3))
    y = (b @ LUMA)[..., None]
    b = np.clip(y + (b - y) * s, 0, 1)
    graded = Image.fromarray((b * 255).round().astype(np.uint8), 'RGB')
    return graded.filter(ImageFilter.UnsharpMask(radius=1.2, percent=40, threshold=2))


def prepare(path, out, do_grade=True):
    img = crop(Image.open(path).convert('RGB'))
    if do_grade:
        img = grade(img)
    img.save(out, 'JPEG', quality=92, subsampling=0, optimize=False, progressive=False)
    return img


def read_db(db, path, vostok_port):
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / 'file'
        subprocess.run([vostok_port, 'extract', '--db', str(db), '--path', path, '--output', str(out)],
                       check=True, stdout=subprocess.DEVNULL)
        return out.read_bytes()


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
    ap.add_argument('--no-grade', action='store_true', help='images are final: crop and resize only')
    a = ap.parse_args()
    overlay = a.bundle / 'overlay'
    manifest_path = a.bundle / 'port-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    if any(f['path'] == LOBBY for f in manifest['files']):
        raise SystemExit('bundle already carries a lobby')
    spec = json.loads(a.spec.read_text())
    ids = loading_ids(spec)
    if not ids:
        raise SystemExit('no spec map declares a loading_image')
    # The importer wrote the same ids into the mode options.
    recorded = {m['id']: m.get('loading_image') for m in manifest['maps']}
    if any(not recorded.get(i) for i in ids):
        raise SystemExit('the bundle was built from a spec without these loading images')
    lib = a.ffdec / 'lib' / 'ffdec_lib.jar'
    if not lib.is_file():
        raise SystemExit(f'no FFDec library at {lib}')
    work = a.work or Path(tempfile.mkdtemp(prefix='lobby-loading-'))
    work.mkdir(parents=True, exist_ok=True)
    original = read_db(a.target_db, LOBBY, a.vostok_port)
    if hashlib.sha256(original).hexdigest() != LOBBY_SHA256:
        raise SystemExit('target lobby_menu.swf is not the 0.26g build 2777 original')
    (work / 'lobby_menu.swf').write_bytes(original)
    rows, records = [], []
    for m in spec['maps']:
        if m['id'] not in ids:
            continue
        key = m.get('hud_key') or f"map_{m['id']}"
        jpeg = work / f'{key}.jpg'
        prepare(ROOT / m['loading_image'], jpeg, not a.no_grade)
        alpha = work / f'{key}.alpha'
        alpha.write_bytes(b'\xff' * (WIDTH * HEIGHT))
        cls = 'SrpLoading_' + key
        rows.append('\t'.join([cls, str(jpeg), str(alpha), str(WIDTH), str(HEIGHT)]))
        records.append({'map_id': m['id'], 'loading_screen_id': ids[m['id']], 'class': cls,
                        'source': m['loading_image'], 'prepared': str(jpeg)})
    (work / 'entries.tsv').write_text('\n'.join(rows) + '\n')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    subprocess.run([a.javac, '-encoding', 'UTF-8', '-cp', str(lib), '-d', str(classes),
                    str(HERE / 'hud_minimaps' / 'PatchLobbyLoading.java')], check=True)
    patched = work / 'lobby_menu-patched.swf'
    result = subprocess.run([a.java, '-Djava.awt.headless=true', f'-Duser.home={work}',
                             '-cp', f'{classes}{os.pathsep}{a.ffdec / "lib" / "*"}', 'PatchLobbyLoading',
                             str(work / 'lobby_menu.swf'), str(patched), str(work / 'entries.tsv'),
                             str(NATIVE_IMAGES)], check=True, capture_output=True, text=True)
    assigned = [line.split()[1] for line in result.stdout.splitlines() if line.startswith('loading_screen_id ')]
    if assigned != [str(r['loading_screen_id']) for r in records]:
        raise SystemExit(f'patcher assigned ids {assigned}, expected {[r["loading_screen_id"] for r in records]}')
    sys.path.insert(0, str(HERE / 'hud_minimaps'))
    import swf_tags
    data = patched.read_bytes()
    missing, added = swf_tags.compare(original, data)
    lobby_abc = lambda c, p: c == swf_tags.DO_ABC2 and p[4:].startswith(b'LobbyMenu\0')
    if sorted(c for c, _ in missing) != [swf_tags.SYMBOL_CLASS, swf_tags.DO_ABC2] \
            or not any(lobby_abc(c, p) for c, p in missing) \
            or sorted(c for c, _ in added) != sorted([swf_tags.DEFINE_BITS_JPEG3] * len(records)
                                                     + [swf_tags.SYMBOL_CLASS, swf_tags.DO_ABC2]):
        raise SystemExit('the lobby patch changed more than the LobbyMenu ABC and the SymbolClass')
    target = overlay / LOBBY
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    manifest['files'].append({'path': LOBBY, 'source': SOURCE, 'size': len(data),
                              'sha256': hashlib.sha256(data).hexdigest()})
    manifest.setdefault('compatibility_notes', []).append(
        f'{LOBBY}: own loading screens ' + ', '.join(f"map {r['map_id']} = id {r['loading_screen_id']}"
                                                     for r in records)
        + f'; native ids 0-{NATIVE_IMAGES - 1} unchanged')
    manifest['lobby_loading'] = records
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({'lobby': LOBBY, 'sha256': hashlib.sha256(data).hexdigest(), 'maps': records}, indent=2))


if __name__ == '__main__':
    main()
