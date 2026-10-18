#!/usr/bin/env python3
"""0.11e loading screen and minimaps in the 0.26g movies (tag level).

Usage: v011e_ui_swf.py tags DONOR_MATCH_MAKING DONOR_HUD LOBBY_MENU HUD OUT_DIR
       v011e_ui_swf.py code OUT_DIR

0.11e shows one loading picture for every map: match_making.swf's
`LevelBack` sprite, which draws bitmap 82 (a DefineBitsJPEG3). Its minimaps
are hud.swf's `minimap_zavod` (Chemical Plant, level_04) and `minimap_base`
(Radar, level_03), both DefineBitsJPEG3, with their own Minimap settings
(level_04: size 140, posX 23.302, posY 43.049; level_03: size 100, posX
14.23, posY -42.196). 0.26g redrew both minimaps and has no 0.11e picture.

`tags` adds to the 0.26g movies, before the class that uses them, for each
picture: the donor's JPEG3 tag under a new character id (the loading picture
re-encoded first, see below), a bitmap class (a
copy of a 0.26g bitmap class's DoABC2 tag renamed to a name of the same
length, so no ABC offset moves; its default width/height constants patched
to the picture's, same u30 length) and a SymbolClass entry binding them:
- lobby_menu.swf: `Level_v011e` (from `Level9_rain`), before `LobbyMenu`;
- hud.swf: `minimap_zv11e` (from `minimap_zavod`) and `minimap_bv11`
  (from `minimap_base`), before `survarium/controls/Minimap`.
Nothing else in the movies changes.

The loading picture: 0.11e stretched its 1280x720 picture over the whole
stage (LevelBack.width/height = stage size). 0.26g's LobbyGameLoad draws a
loading bitmap unscaled at 1920 wide (scale max(stage width / 1920, band /
height)) in the band between its overlays; a native picture is 1920x798.
So `Level_v011e` gets the 0.11e picture as 0.11e showed it at 1920x1080
(scaled 1.5x, Lanczos), cropped to its middle 1920x798, JPEG quality 92
(Pillow, which `tags` needs on PYTHONPATH), with `Level9_rain`'s 1920x798
defaults unchanged. Shipped unscaled it would sit 1280 wide between 320 px
black bars.

`code` then edits the two classes in OUT_DIR's movies (abc_patch.py:
constants appended, one method body each, nothing else touched):
`LobbyMenu`'s `_load_images` gets `Level_v011e` as its 30th entry
(loading_screen_id 29), `BottomPanel.matchesNameTable()` levels 9-13 (the Matches window), and `Minimap` gets the map keys `v011e_level_04` /
`v011e_level_03` with the 0.11e bitmaps and settings.
"""
import struct, sys, zlib
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import abc_patch


def read(path):
    data = Path(path).read_bytes()
    assert data[:3] in (b'CWS', b'FWS'), path
    version = data[3]
    body = zlib.decompress(data[8:]) if data[:3] == b'CWS' else data[8:]
    nbits = body[0] >> 3
    head = (5 + 4 * nbits + 7) // 8 + 4
    tags, p = [], head
    while p < len(body):
        h = struct.unpack_from('<H', body, p)[0]; p += 2
        code, length = h >> 6, h & 0x3f
        if length == 0x3f:
            length = struct.unpack_from('<I', body, p)[0]; p += 4
        tags.append([code, body[p:p + length]]); p += length
        if code == 0:
            break
    return version, body[:head], tags


def write(path, version, head, tags):
    out = bytearray(head)
    for code, payload in tags:
        if len(payload) < 0x3f and code not in (6, 21, 35, 36, 90):
            out += struct.pack('<H', (code << 6) | len(payload))
        else:
            out += struct.pack('<HI', (code << 6) | 0x3f, len(payload))
        out += payload
    body = bytes(out)
    Path(path).write_bytes(b'CWS' + bytes([version]) + struct.pack('<I', len(body) + 8) + zlib.compress(body, 9))


def symbols(tags):
    out = {}
    for code, b in tags:
        if code == 76:
            n = struct.unpack_from('<H', b)[0]; q = 2
            for _ in range(n):
                i = struct.unpack_from('<H', b, q)[0]; q += 2
                e = b.index(0, q); out[b[q:e].decode()] = i; q = e + 1
    return out


CHARACTER_TAGS = {2, 6, 7, 10, 11, 14, 20, 21, 22, 32, 34, 35, 36, 37, 39, 46, 48, 60, 62, 75, 83, 84, 87, 88, 90, 91}


def max_id(tags):
    return max(struct.unpack_from('<H', b)[0] for c, b in tags if c in CHARACTER_TAGS and len(b) >= 2)


def image_tag(tags, cid):
    found = [t for t in tags if t[0] in (6, 20, 21, 35, 36, 90) and struct.unpack_from('<H', t[1])[0] == cid]
    assert len(found) == 1 and found[0][0] == 35, f'character {cid} is not one DefineBitsJPEG3'
    return found[0]


def doabc(tags, name):
    found = [i for i, (c, b) in enumerate(tags) if c == 82 and b[4:4 + len(name) + 1] == name.encode() + b'\0']
    assert len(found) == 1, f'no single DoABC2 {name}'
    return found[0]


def u30(n):
    out = bytearray()
    while True:
        b = n & 0x7f; n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def bitmap_class(tags, template, name, size_from, size_to):
    """A copy of bitmap class `template`'s DoABC2 tag as `name` (same length)."""
    assert len(name) == len(template)
    payload = tags[doabc(tags, template)][1]
    assert payload.count(template.encode()) >= 2
    payload = payload.replace(template.encode(), name.encode())
    for old, new in zip(size_from, size_to):
        a, b = u30(old), u30(new)
        assert len(a) == len(b) and payload.count(a) >= 1, (old, new)
        # The int pool sits right after the ABC version: patch its first occurrence there.
        at = payload.index(a, 4 + len(name) + 1)
        payload = payload[:at] + b + payload[at + len(a):]
    return [82, payload]


def cover(body, size):
    """A DefineBitsJPEG3 body (after the id) as a JPEG3 body of `size`: scaled to cover it and centre-cropped,
    its alpha channel scaled and cropped with it."""
    import io
    from PIL import Image
    n, = struct.unpack_from('<I', body, 2)
    img = Image.open(io.BytesIO(body[6:6 + n])).convert('RGB')
    alpha = Image.frombytes('L', img.size, zlib.decompress(body[6 + n:]))
    w, h = size
    scale = max(w / img.width, h / img.height)
    sw, sh = round(img.width * scale), round(img.height * scale)
    x, y = (sw - w) // 2, (sh - h) // 2
    img, alpha = (i.resize((sw, sh), Image.LANCZOS).crop((x, y, x + w, y + h)) for i in (img, alpha))
    out = io.BytesIO(); img.save(out, 'JPEG', quality=92)
    jpeg = out.getvalue()
    return body[:2] + struct.pack('<I', len(jpeg)) + jpeg + zlib.compress(alpha.tobytes(), 9)


def add_pictures(target, donor, pictures, before):
    """pictures: (donor character id, class name, template class, template size, picture size[, cover size])."""
    version, head, tags = read(target)
    _, _, dtags = read(donor)
    nid = max_id(tags) + 1
    new = []
    for cid, name, template, size_from, size_to, *fit in pictures:
        code, body = image_tag(dtags, cid)
        if fit:
            body = cover(body, fit[0])
        new.append([code, struct.pack('<H', nid) + body[2:]])
        new.append(bitmap_class(tags, template, name, size_from, size_to))
        new.append([76, struct.pack('<HH', 1, nid) + name.encode() + b'\0'])
        nid += 1
    at = doabc(tags, before)
    tags[at:at] = new
    return version, head, tags


def tags_command(match_making, donor_hud, lobby, hud, out_dir):
    out = Path(out_dir); out.mkdir(parents=True, exist_ok=True)
    mm_symbol = 82                     # LevelBack (symbol 84) draws shape 83 filled with bitmap 82
    write(out / 'lobby_menu.swf', *add_pictures(lobby, match_making,
          [(mm_symbol, 'Level_v011e', 'Level9_rain', (), (), (1920, 798))], 'LobbyMenu'))
    donor = symbols(read(donor_hud)[2])
    write(out / 'hud.swf', *add_pictures(hud, donor_hud,
          [(donor['minimap_zavod'], 'minimap_zv11e', 'minimap_zavod', (458,), (512,)),
           (donor['minimap_base'], 'minimap_bv11', 'minimap_base', (512,), (512,))],
          'survarium/controls/Minimap'))


def class_abc(tags, name):
    i = doabc(tags, name)
    payload = tags[i][1]
    head = 4 + len(name) + 1
    return i, payload[:head], abc_patch.Abc(payload[head:])


def qname(abc, name):
    """The multiname QName(PackageNamespace(""), name), added if missing."""
    return abc.multiname(0x07, abc.package_namespace(), abc.string(name))


def find(abc, body, op, operand, after=None):
    """Offset of the instruction `op` with first operand `operand` (after `after`)."""
    hits = [i for i in abc_patch.instructions(bytes(body['code'])) if i[1] == op and i[2][:1] == [operand] and (after is None or i[0] > after)]
    assert hits, (hex(op), operand)
    return hits[0][0]


def code_command(out_dir):
    """Adds the classes' uses to LobbyMenu and Minimap in OUT_DIR's movies (after `tags`)."""
    U = abc_patch.u30
    out = Path(out_dir)
    # lobby_menu.swf: _load_images gets Level_v011e as its 30th entry.
    version, head, tags = read(out / 'lobby_menu.swf')
    i, prefix, abc = class_abc(tags, 'LobbyMenu')
    level9 = abc.multiname(0x07, abc.package_namespace(), abc.find_string('Level9_rain'))
    body = abc.body_with(lambda b: bytes([0x66]) + U(level9) + bytes([0x56]) + U(29) in b['code'])
    at = find(abc, body, 0x56, 29, find(abc, body, 0x66, level9))
    new = qname(abc, 'Level_v011e')
    insert = bytes([0x5d]) + U(new) + bytes([0x66]) + U(new)
    abc.insert(body, at, insert, 2)
    at += len(insert); assert body['code'][at:at + 2] == bytes([0x56, 29]); body['code'][at + 1] = 30
    tags[i][1] = prefix + abc.bytes()
    matches_levels(tags)
    write(out / 'lobby_menu.swf', version, head, tags)
    # hud.swf: Minimap gets the 0.11e maps under v011e_level_04 / v011e_level_03.
    version, head, tags = read(out / 'hud.swf')
    i, prefix, abc = class_abc(tags, 'survarium/controls/Minimap')
    map09 = abc.multiname(0x07, abc.package_namespace(), abc.find_string('Map_09'))
    body = abc.body_with(lambda b: bytes([0x4a]) + U(map09) + U(0) + bytes([0x55]) + U(12) in b['code'])
    at = find(abc, body, 0x55, 12, find(abc, body, 0x4a, map09))
    zv, bv = qname(abc, 'minimap_zv11e'), qname(abc, 'minimap_bv11')
    k04, k03 = abc.string('v011e_level_04'), abc.string('v011e_level_03')
    insert = (bytes([0x2c]) + U(k04) + bytes([0x5d]) + U(zv) + bytes([0x4a]) + U(zv) + U(0)
              + bytes([0x2c]) + U(k03) + bytes([0x5d]) + U(bv) + bytes([0x4a]) + U(bv) + U(0))
    abc.insert(body, at, insert, 4)
    at += len(insert); assert body['code'][at:at + 2] == bytes([0x55, 12]); body['code'][at + 1] = 14
    # The settings: after `_minimapProps.level_04 = {...}`, the same for the two keys.
    ins = abc_patch.instructions(bytes(body['code']))
    set04 = [x for x in ins if x[1] == 0x61 and abc.multiname_name(x[2][0]) == 'level_04']
    assert len(set04) == 1
    raw04 = abc.multiname_raw(set04[0][2][0]); assert raw04[0] == 0x09
    _, q = abc_patch.read_u30(raw04, 1); nsset, _ = abc_patch.read_u30(raw04, q)
    k = ins.index(set04[0])
    props = ins[k - 8]; assert props[1] == 0x66 and ins[k - 9][1] == 0xd0   # getlocal0; getproperty _minimapProps
    size, posx, posy = abc.find_string('size'), abc.find_string('posX'), abc.find_string('posY')
    def entry(key, s, x, y):
        return (bytes([0xd0, 0x66]) + U(props[2][0]) + bytes([0x2c]) + U(size) + bytes([0x25]) + U(s)
                + bytes([0x2c]) + U(posx) + bytes([0x2f]) + U(abc.double(x)) + bytes([0x2c]) + U(posy) + bytes([0x2f]) + U(abc.double(y))
                + bytes([0x55]) + U(3) + bytes([0x61]) + U(abc.multiname(0x09, key, nsset)))
    abc.insert(body, set04[0][0] + set04[0][3], entry(k04, 140, 23.302, 43.049) + entry(k03, 100, 14.23, -42.196), 0)
    tags[i][1] = prefix + abc.bytes()
    write(out / 'hud.swf', version, head, tags)


# The Matches window (srp's lobby movie, BottomPanel.matchesNameTable()) names a match's map by its level
# (match_making_params.map_id) from a table of levels 1-8 in its five languages (English, Ukrainian,
# Russian, German, Polish); any other level shows as "#<level>". Levels 9-13 as the loading screen
# names them (st_level_<n>_name: the native 0.26 name, the port's source version in brackets).
MATCHES_LEVELS = [
    ('9', ['London', 'Лондон', 'Лондон', 'London', 'Londyn']),
    ('10', ['Vostok Radar Station (v0.21)', 'РЛС "Восток" (v0.21)', 'РЛС "Восток" (v0.21)', 'RS Vostok (v0.21)', 'Stacja Radarowa "Vostok" (v0.21)']),
    ('11', ['Mamayev Kurgan (v0.21)', 'Мамаїв курган (v0.21)', 'Мамаев курган (v0.21)', 'Mamaew kurgan (v0.21)', 'Kurhan Mamaja (v0.21)']),
    ('12', ['Chemical Plant (v0.11)', 'Хімзавод (v0.11)', 'Химзавод (v0.11)', 'Chemiebetrieb (v0.11)', 'Zakłady chemiczne (v0.11)']),
    ('13', ['Vostok Radar Station (v0.11)', 'РЛС "Восток" (v0.11)', 'РЛС "Восток" (v0.11)', 'RS Vostok (v0.11)', 'Stacja Radarowa "Vostok" (v0.11)']),
]


def matches_levels(tags):
    """BottomPanel.matchesNameTable(): the `levels` object literal (`"8":[Cologne x5]` then `newobject 8`)
    gets MATCHES_LEVELS before its newobject, whose count becomes 13. One method body, constants appended."""
    U = abc_patch.u30
    i, prefix, abc = class_abc(tags, 'survarium/controls/BottomPanel')
    cologne = abc.find_string('Most w Kolonii')
    body = abc.body_with(lambda b: any(x[1] == 0x2c and x[2][0] == cologne for x in abc_patch.instructions(bytes(b['code']))))
    ins = abc_patch.instructions(bytes(body['code']))
    k = [j for j, x in enumerate(ins) if x[1] == 0x2c and x[2][0] == cologne]
    assert len(k) == 1 and ins[k[0] + 1][1] == 0x56 and ins[k[0] + 1][2] == [5], 'Cologne is not the table\'s last row'
    new = ins[k[0] + 2]
    if new[1] == 0x2c:      # rows after Cologne: a movie that has them already (revision 20's lobby_menu.swf)
        found, j = [], k[0] + 2
        while ins[j][1] == 0x2c:
            row = [abc.strings[ins[j + i][2][0] - 1] for i in range(6)]
            found.append((row[0][1:].decode(), [r[1:].decode() for r in row[1:]]))   # skip each string's length byte
            assert ins[j + 6][1] == 0x56 and ins[j + 6][2] == [5], 'unexpected Matches table'
            j += 7
        assert ins[j][1] == 0x55 and ins[j][2] == [8 + len(found)], 'unexpected Matches table'
        want = [(key, names) for key, names in MATCHES_LEVELS]
        assert found == want, f'the movie has other Matches rows: {found}'
        print('Matches window: levels 9-13 already there, unchanged')
        return
    assert new[1] == 0x55 and new[2] == [8], 'the levels table is not 8 rows'
    code = b''
    for key, names in MATCHES_LEVELS:
        assert len(names) == 5
        code += bytes([0x2c]) + U(abc.string(key))
        for n in names:
            code += bytes([0x2c]) + U(abc.string(n))
        code += bytes([0x56]) + U(5)
    abc.insert(body, new[0], code, 2 * len(MATCHES_LEVELS))
    at = new[0] + len(code)
    assert body['code'][at:at + 2] == bytes([0x55, 8])
    body['code'][at + 1] = 8 + len(MATCHES_LEVELS)
    tags[i][1] = prefix + abc.bytes()


if __name__ == '__main__':
    command, args = sys.argv[1], sys.argv[2:]
    if command == 'tags':
        tags_command(*args)
    elif command == 'code':
        code_command(*args)
    else:
        raise SystemExit(__doc__)
