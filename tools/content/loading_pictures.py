#!/usr/bin/env python3
"""The map loading pictures a 0.26g lobby_menu.swf shows, as PNGs.

Usage: loading_pictures.py LOBBY_MENU_SWF OUT_DIR NAME=ID [NAME=ID ...]

Follows the movie's own chain: LobbyMenu._load_images (the classes before
the array's `newarray`), entry ID -> class name -> SymbolClass character ->
image tag (DefineBits with JPEGTables, DefineBitsJPEG2/3, DefineBitsLossless/2). Writes per NAME:
- NAME.png: the picture as LobbyGameLoad.placeImages() puts it on a
  1920x1080 stage (scale max(1, band/height), centred; the band between the
  176 px top and 251 px bottom overlays), cropped to the 1920x798 rows a
  native picture fills (stage y 103.5-901.5); anything the bitmap does not
  cover is black;
- NAME.source.png: the bitmap itself.
Needs Pillow (PYTHONPATH). Prints one JSON line per picture.
"""
import io, json, struct, sys, zlib
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import abc_patch
from v011e_ui_swf import read, symbols, class_abc
from PIL import Image


def load_images(tags):
    _, _, abc = class_abc(tags, 'LobbyMenu')
    body = abc.body_with(lambda b: abc.find_string('Level1_day') is not None and bytes([0x56]) in b['code']
                         and any(i[1] == 0x66 and abc.multiname_name(i[2][0]) == 'Level1_day' for i in abc_patch.instructions(bytes(b['code']))))
    ins = abc_patch.instructions(bytes(body['code']))
    for k, x in enumerate(ins):
        if x[1] == 0x56:                                  # newarray N: the N getproperty class reads before it
            names = [abc.multiname_name(y[2][0]) for y in ins[:k] if y[1] == 0x66][-x[2][0]:]
            if names and names[0] == 'Level1_day':
                return names
    raise SystemExit('no _load_images array in LobbyMenu')


def bitmap(tags, cid):
    for code, p in tags:
        if code in (6, 20, 21, 35, 36, 90) and struct.unpack_from('<H', p)[0] == cid:
            if code == 6:                                 # DefineBits: the scan, with the movie's JPEGTables
                clean = lambda d: d[4:] if d[:4] == b'\xff\xd9\xff\xd8' else d
                tables = [clean(t) for c, t in tags if c == 8 and t]
                d = clean(p[2:])
                if not tables:                            # empty JPEGTables: the scan is a whole JPEG
                    return Image.open(io.BytesIO(d)).convert('RGB'), 'DefineBits (whole JPEG)'
                assert len(tables) == 1, 'several JPEGTables'
                return Image.open(io.BytesIO(tables[0][:-2] + d[2:])).convert('RGB'), 'DefineBits+JPEGTables'
            if code == 21:
                return Image.open(io.BytesIO(p[2:])).convert('RGB'), 'DefineBitsJPEG2'
            if code == 35:
                n, = struct.unpack_from('<I', p, 2)
                img = Image.open(io.BytesIO(p[6:6 + n])).convert('RGB')
                alpha = zlib.decompress(p[6 + n:])
                if any(a != 255 for a in alpha):
                    img = img.convert('RGBA'); img.putalpha(Image.frombytes('L', img.size, alpha))
                return img, 'DefineBitsJPEG3'
            if code in (20, 36):
                fmt, w, h = struct.unpack_from('<BHH', p, 2)
                assert fmt == 5, f'lossless format {fmt}'
                argb = zlib.decompress(p[7:])
                img = Image.frombytes('RGBA', (w, h), argb, 'raw', 'ARGB')
                return img, 'DefineBitsLossless' + ('2' if code == 36 else '')
            raise SystemExit(f'character {cid}: tag {code} not handled')
    raise SystemExit(f'no image tag for character {cid}')


def staged(img):
    """placeImages() at a 1920x1080 stage, cropped to stage rows 103.5..901.5 (1920x798)."""
    w, h = img.size
    top, bottom = 176, 251
    band = 1080 - top - bottom
    scale = max(1.0, band / h)
    sw, sh = round(w * scale), round(h * scale)
    y = top + (band - sh) / 2
    if top < y or y + sh < 1080 - bottom:
        y = top
    x = 1920 / 2 - sw / 2
    stage = Image.new('RGB', (1920, 1080), 'black')
    pic = img.resize((sw, sh), Image.LANCZOS) if scale != 1 else img
    stage.paste(pic.convert('RGB'), (round(x), round(y)))
    return stage.crop((0, 103, 1920, 901)), dict(x=x, y=y, scale=scale, shown_width=sw, shown_height=sh)


def main():
    swf, out, pairs = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3:]
    out.mkdir(parents=True, exist_ok=True)
    _, _, tags = read(swf)
    classes = load_images(tags)
    ids = symbols(tags)
    for pair in pairs:
        name, i = pair.split('='); i = int(i)
        cls = classes[i]; cid = ids[cls]
        img, kind = bitmap(tags, cid)
        img.save(out / f'{name}.source.png')
        shown, place = staged(img)
        shown.save(out / f'{name}.png')
        print(json.dumps(dict(name=name, loading_screen_id=i, entries=len(classes), class_name=cls, character=cid, tag=kind,
                              size=list(img.size), placed_on_1920x1080=place)))


if __name__ == '__main__':
    main()
