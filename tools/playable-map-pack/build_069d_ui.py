#!/usr/bin/env python3
"""UI bundle for a 0.69d map ported to 0.26g: its own loading screen and minimap.

    build_069d_ui.py --porter VOSTOK_PORT --magick MAGICK --target-db 026.db \
        --donor-db donor.split --donor-mode gameplay/game_modes/level_11_day_tdm.options \
        --output BUNDLE [--loading-class Level8_night] [--minimap-slot level_09]

0.26g has no data-driven loading screens or minimaps: both live in Flash.

* Loading screens are the bitmap classes listed in LobbyMenu._load_images
  (lobby_menu.swf); `mode.ui.loading_screen_id` indexes that list. The
  donor's loading image (flash_movies/textures/ui_loading/<level>_<weather>.png)
  replaces the JPEG of one slot no 0.26 mode uses (index 24, Level8_night:
  only the level_08 night modes name it, and they have no project). The
  image is centre-cropped to the 1920x798 of the level 1-6 screens.
* Minimaps are bitmap classes of hud.swf; Minimap.setProps sends HUD keys it
  does not know to "level_09", whose Map_09 no 0.26 map uses. The donor's
  minimap (flash_movies/textures/ui_hud_maps/<ui.minimap_image>.png, placed
  by ui.minimap_props: centre `position` x/z, half extents `scale` x/z) is
  resampled into 0.26's square frame (half extent `size`, centre posX/posY;
  0.26 draws world -z downwards, as 0.69d does) and the level_09 props in
  Minimap's constructor are rewritten in place (same code length; the
  constructor's double constants are Minimap's own).

Writes BUNDLE/overlay/flash_movies/{lobby_menu,hud}.swf and
BUNDLE/port-manifest.json for `vostok-port apply`. Donor files are data.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import zlib

NOP, PUSHBYTE, PUSHSTRING, PUSHDOUBLE = 0x02, 0x24, 0x2C, 0x2F


# --- SWF ---------------------------------------------------------------------
def swf_load(data):
    if data[:3] == b'CWS':
        return data[:8], zlib.decompress(data[8:])
    if data[:3] == b'FWS':
        return data[:8], data[8:]
    raise ValueError('not a SWF')


def swf_tags(body):
    nbits = body[0] >> 3
    o = (5 + 4 * nbits + 7) // 8 + 4
    head, tags = body[:o], []
    while o < len(body):
        code_len = struct.unpack_from('<H', body, o)[0]
        code, length, start = code_len >> 6, code_len & 0x3F, o + 2
        if length == 0x3F:
            length = struct.unpack_from('<I', body, start)[0]
            start += 4
        tags.append([code, body[start:start + length]])
        o = start + length
        if code == 0:
            break
    return head, tags


def swf_save(header, head, tags):
    out = bytearray(head)
    for code, payload in tags:
        if len(payload) < 0x3F and code not in (6, 21, 35, 36, 20, 82, 87):
            out += struct.pack('<H', code << 6 | len(payload))
        else:
            out += struct.pack('<HI', code << 6 | 0x3F, len(payload))
        out += payload
    body = bytes(out)
    return b'CWS' + header[3:4] + struct.pack('<I', len(body) + 8) + zlib.compress(body, 9)


def symbol_ids(tags):
    ids = {}
    for code, p in tags:
        if code == 76:
            q, count = 2, struct.unpack_from('<H', p, 0)[0]
            for _ in range(count):
                cid = struct.unpack_from('<H', p, q)[0]
                e = p.index(b'\0', q + 2)
                ids[p[q + 2:e].decode('latin1')] = cid
                q = e + 1
    return ids


def jpeg_size(b):
    i = 2
    while i < len(b) - 9:
        if b[i] == 0xFF and b[i + 1] in (0xC0, 0xC1, 0xC2):
            h, w = struct.unpack_from('>HH', b, i + 5)
            return w, h
        if b[i] == 0xFF and b[i + 1] not in (0x00, 0xD8, 0x01) and not 0xD0 <= b[i + 1] <= 0xD7:
            i += 2 + struct.unpack_from('>H', b, i + 2)[0]
        else:
            i += 1
    raise ValueError('no JPEG frame header')


# --- AVM2 ----------------------------------------------------------------------
def u30(b, o):
    value = shift = 0
    while True:
        byte = b[o]
        o += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, o


def enc_u30(v):
    out = bytearray()
    while True:
        byte = v & 0x7F
        v >>= 7
        out.append(byte | (0x80 if v else 0))
        if not v:
            return bytes(out)


def abc_pools(abc):
    """Constant pools: doubles [(index, offset, value)], strings [index] -> str,
    multiname index -> name string (None for runtime names)."""
    o = 4
    count, o = u30(abc, o)
    for _ in range(max(count - 1, 0)):
        _, o = u30(abc, o)
    count, o = u30(abc, o)
    for _ in range(max(count - 1, 0)):
        _, o = u30(abc, o)
    count, o = u30(abc, o)
    doubles = []
    for i in range(1, count):
        doubles.append((i, o, struct.unpack_from('<d', abc, o)[0]))
        o += 8
    count, o = u30(abc, o)
    strings = [None]
    for i in range(1, count):
        n, o = u30(abc, o)
        strings.append(abc[o:o + n].decode('utf-8', 'replace'))
        o += n
    count, o = u30(abc, o)
    for _ in range(max(count - 1, 0)):
        o += 1
        _, o = u30(abc, o)
    count, o = u30(abc, o)
    for _ in range(max(count - 1, 0)):
        n, o = u30(abc, o)
        for _ in range(n):
            _, o = u30(abc, o)
    count, o = u30(abc, o)
    multinames = [None]
    for _ in range(max(count - 1, 0)):
        kind = abc[o]
        o += 1
        name = None
        if kind in (0x07, 0x0D):
            _, o = u30(abc, o)
            name, o = u30(abc, o)
        elif kind in (0x0F, 0x10):
            name, o = u30(abc, o)
        elif kind in (0x11, 0x12):
            pass
        elif kind in (0x09, 0x0E):
            name, o = u30(abc, o)
            _, o = u30(abc, o)
        elif kind in (0x1B, 0x1C):
            _, o = u30(abc, o)
        elif kind == 0x1D:
            _, o = u30(abc, o)
            n, o = u30(abc, o)
            for _ in range(n):
                _, o = u30(abc, o)
        else:
            raise ValueError(f'unknown multiname kind {kind:#x}')
        multinames.append(strings[name] if name else None)
    return doubles, strings, multinames


def push_number(value, slots):
    if value == int(value) and -128 <= value <= 127:
        return bytes([PUSHBYTE, int(value) & 0xFF]), None
    if not slots:
        raise ValueError(f'no double constant left for {value}')
    slot = slots.pop(0)
    return bytes([PUSHDOUBLE]) + enc_u30(slot[0]), (slot, value)


def patch_minimap_props(abc, key, size, pos_x, pos_y):
    """Rewrite `_minimapProps.<key> = {size, posX, posY}` in place: the same
    code length (padded with nop) and the block's own double constants."""
    doubles, strings, multinames = abc_pools(abc)
    names = [strings.index(n) for n in ('size', 'posX', 'posY')]
    blocks = []
    for start in range(len(abc) - 16):
        o, used = start, []
        for name in names:
            if abc[o] != PUSHSTRING:
                break
            idx, o = u30(abc, o + 1)
            if idx != name:
                break
            if abc[o] == PUSHBYTE:
                o, d = o + 2, None
            elif abc[o] == PUSHDOUBLE:
                d, o = u30(abc, o + 1)
            else:
                break
            used.append(d)
        else:
            # newobject 3; setproperty <key>
            if abc[o] == 0x55 and abc[o + 1] == 3 and abc[o + 2] == 0x61:
                mn, _ = u30(abc, o + 3)
                if multinames[mn] == key:
                    blocks.append((start, o, used))
    if len(blocks) != 1:
        raise ValueError(f'expected one {key} props block, found {len(blocks)}')
    start, end, used = blocks[0]
    # Doubles used here and nowhere else in the code (the pool index appears once as pushdouble).
    slots = []
    for index, offset, old in doubles:
        if index in used:
            uses = abc.count(bytes([PUSHDOUBLE]) + enc_u30(index))
            if uses == 1:
                slots.append((index, offset, old))
    code, writes = bytearray(), []
    for name, value in zip(names, (size, pos_x, pos_y)):
        code += bytes([PUSHSTRING]) + enc_u30(name)
        push, write = push_number(float(value), slots)
        code += push
        if write:
            writes.append(write)
    if len(code) > end - start:
        raise ValueError('patched props block is longer than the original')
    abc = bytearray(abc)
    abc[start:end] = code + bytes([NOP]) * (end - start - len(code))
    for (index, offset, old), new in writes:
        struct.pack_into('<d', abc, offset, new)
    return bytes(abc), [(index, old, new) for (index, _, old), new in writes]


# --- images --------------------------------------------------------------------
def magick(tool, args, data):
    return subprocess.run([tool, *args], input=data, capture_output=True, check=True).stdout


def loading_jpeg(tool, png, width=1920, height=798):
    return magick(tool, ['png:-', '-resize', f'{width}x', '-gravity', 'center', '-extent', f'{width}x{height}',
                         '-strip', '-interlace', 'none', '-sampling-factor', '2x2', '-quality', '92', 'jpg:-'], png)


def minimap_jpeg3(tool, png, half_x, half_z, size, pixels=1024):
    w, h = round(pixels * half_x / size), round(pixels * half_z / size)
    rgba = magick(tool, ['png:-', '-alpha', 'on', '-resize', f'{w}x{h}!', '-background', 'none', '-gravity', 'center',
                         '-extent', f'{pixels}x{pixels}', 'png:-'], png)
    jpeg = magick(tool, ['png:-', '-background', 'black', '-alpha', 'remove', '-strip', '-interlace', 'none',
                         '-sampling-factor', '2x2', '-quality', '92', 'jpg:-'], rgba)
    alpha = magick(tool, ['png:-', '-alpha', 'extract', '-depth', '8', 'gray:-'], rgba)
    if len(alpha) != pixels * pixels:
        raise ValueError('alpha plane size')
    return jpeg, zlib.compress(alpha, 9)


# --- porter --------------------------------------------------------------------
def extract(porter, db, path, out):
    subprocess.run([porter, 'extract', '--db', str(db), '--path', path, '--output', str(out)], check=True,
                   capture_output=True)
    return Path(out).read_bytes()


def leaves(porter, path):
    text = subprocess.run([porter, 'dump-config', '--file', str(path), '--leaves'], check=True,
                          capture_output=True, text=True).stdout
    out = {}
    for line in text.splitlines():
        k, _, v = line.partition('\t')
        out[k] = v
    return out


def value(v):
    kind, _, raw = v.partition(':')
    if kind == 'string':
        return json.loads(raw)
    if kind.startswith('float'):
        return [float(x) for x in raw.strip('[]').split(',')]
    return float(raw)


def main():
    a = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    a.add_argument('--porter', required=True)
    a.add_argument('--magick', required=True)
    a.add_argument('--target-db', required=True, type=Path)
    a.add_argument('--donor-db', required=True, type=Path)
    a.add_argument('--donor-mode', required=True)
    a.add_argument('--output', required=True, type=Path)
    a.add_argument('--loading-class', default='Level8_night')
    a.add_argument('--minimap-slot', default='level_09')
    a.add_argument('--minimap-pixels', type=int, default=1024)
    args = a.parse_args()
    out = args.output
    (out/'overlay'/'flash_movies').mkdir(parents=True, exist_ok=False)
    records, notes = [], []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        extract(args.porter, args.donor_db, args.donor_mode, tmp/'mode')
        mode = leaves(args.porter, tmp/'mode')
        level, weather = value(mode['mode.level_name']), value(mode['mode.weather'])
        loading_path = f'flash_movies/textures/ui_loading/{level}_{weather}.png'
        minimap_path = f'flash_movies/textures/ui_hud_maps/{value(mode["mode.ui.minimap_image"])}.png'
        centre = value(mode['mode.ui.minimap_props.position'])
        scale = value(mode['mode.ui.minimap_props.scale'])
        loading_png = extract(args.porter, args.donor_db, loading_path, tmp/'loading.png')
        minimap_png = extract(args.porter, args.donor_db, minimap_path, tmp/'minimap.png')

        # Loading screen.
        header, body = swf_load(extract(args.porter, args.target_db, 'flash_movies/lobby_menu.swf', tmp/'lobby.swf'))
        head, tags = swf_tags(body)
        cid = symbol_ids(tags)[args.loading_class]
        jpeg = loading_jpeg(args.magick, loading_png)
        hits = [t for t in tags if t[0] == 6 and struct.unpack_from('<H', t[1], 0)[0] == cid]
        if len(hits) != 1:
            raise SystemExit(f'{args.loading_class}: expected one DefineBits tag, found {len(hits)}')
        old = jpeg_size(hits[0][1][2:])
        hits[0][1] = struct.pack('<H', cid) + jpeg
        lobby = swf_save(header, head, tags)
        notes.append(f'lobby_menu.swf {args.loading_class} (character {cid}, was {old[0]}x{old[1]}): {loading_path} as {jpeg_size(jpeg)[0]}x{jpeg_size(jpeg)[1]} JPEG')

        # Minimap.
        header, body = swf_load(extract(args.porter, args.target_db, 'flash_movies/hud.swf', tmp/'hud.swf'))
        head, tags = swf_tags(body)
        size = max(scale[0], scale[2])
        bitmap = {'level_09': 'Map_09'}[args.minimap_slot]
        cid = symbol_ids(tags)[bitmap]
        jpeg, alpha = minimap_jpeg3(args.magick, minimap_png, scale[0], scale[2], size, args.minimap_pixels)
        hits = [t for t in tags if t[0] == 35 and struct.unpack_from('<H', t[1], 0)[0] == cid]
        if len(hits) != 1:
            raise SystemExit(f'{bitmap}: expected one DefineBitsJPEG3 tag, found {len(hits)}')
        hits[0][1] = struct.pack('<HI', cid, len(jpeg)) + jpeg + alpha
        abc_tags = [t for t in tags if t[0] == 82 and b'survarium.controls:Minimap' in t[1] and b'_minimapProps' in t[1]]
        if len(abc_tags) != 1:
            raise SystemExit(f'expected one Minimap ABC, found {len(abc_tags)}')
        payload = abc_tags[0][1]
        name_end = payload.index(b'\0', 4) + 1
        abc, writes = patch_minimap_props(payload[name_end:], args.minimap_slot, size, centre[0], centre[2])
        abc_tags[0][1] = payload[:name_end] + abc
        hud = swf_save(header, head, tags)
        notes.append(f'hud.swf {bitmap} (character {cid}): {minimap_path} resampled to {args.minimap_pixels}x{args.minimap_pixels}; '
                     f'Minimap {args.minimap_slot} props size {size}, posX {centre[0]}, posY {centre[2]} (double constants {writes})')

    for name, data in [('flash_movies/lobby_menu.swf', lobby), ('flash_movies/hud.swf', hud)]:
        (out/'overlay'/name).write_bytes(data)
        records.append({'path': name, 'source': '0.26g Flash movie with the donor map\'s image', 'size': len(data),
                        'sha256': hashlib.sha256(data).hexdigest()})
    (out/'port-manifest.json').write_text(json.dumps({'files': records, 'compatibility_notes': notes}, indent=2)+'\n')
    print('\n'.join(notes))


if __name__ == '__main__':
    main()
