#!/usr/bin/env python3
"""The exoskeletons' oxygen slot in hud.swf shows the exo's helmet.

Usage: exo_oxygen_hud_swf.py HUD_SWF LOBBY_MENU_SWF OUT_HUD_SWF

How the 0.26g HUD draws an oxygen slot. The game fills the quick slot of
the item that carries the oxygen with that item's `ui_desc.icon` as
`icon_row` / `icon_column` (1-based). `QuickSlots` turns them into an icon
id, `(icon_column - 1) * 22 + icon_row - 1`, and looks the id up in its
`hudIcons` table: the position in the table is the icon's cell in the HUD's
own atlas, the `hudItems` bitmap (cells of 171x83, column by column, 24 to a
column). An id that is not in the table gets index -1: no icon, only the
hotkey and the seconds. `HUD.init` makes grey copies of cells 0-9 (shown
while a slot is empty or cooling down), and `QuickItemRenderer.updateSlot`
keeps the grey copy of an empty oxygen tank on screen for the ids 13, 42,
67 and 74: the four native tanks (scavenger_oxygen_1 [14, 1], edge_back_3
[21, 2], ren_back_5 [2, 4], ren_back_3 [9, 4]), drawn in cells 6-9.

The exoskeletons carry their oxygen in the body (torso slot), whose icon is
not in the table: Duty 1805 [1, 6] is id 110, Freedom 1807 [4, 6] is id 113.
This tool, over any hud.swf with the native code (it checks every byte it
relies on):
- draws the helmets' lobby icons (Duty 1800 [3, 6], Freedom 1808 [5, 6],
  171x83 cells of lobby_menu.swf's LobbyMenu_EquipmentIcons) into the empty
  hudItems cells 28 and 29. The atlas is a DefineBitsJPEG3 (premultiplied
  colour, zlib alpha); it becomes a DefineBitsLossless2 (format 5,
  premultiplied ARGB, as the lobby atlas) of the same pixels under the same
  character id, so cells 0-27 keep what the JPEG decodes to;
- `hudIcons` gets -1 for the unlisted cells 23-27 (no id is negative) and
  110, 113 for cells 28, 29;
- `HUD.init` makes the grey copies of cells 28 and 29 too (a second copy of
  its loop over 28 <= i < 30);
- `updateSlot`'s tank test also holds for 110 and 113.
Nothing else in the movie changes. Branch offsets across an insertion are
re-targeted; the tool refuses any code it does not recognise.
"""
import struct, sys, zlib, io
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import abc_patch
from abc_patch import u30, read_u30, instructions

ROWS = 22                      # QuickSlots.otherRows
HUD_COLUMN = 24                # hudItems cells per column (2048 // 83)
CELL_W, CELL_H, LOBBY_PITCH = 171, 83, 91
NATIVE_HUD_ICONS = [17, 18, 19, 20, 21, 22, 13, 42, 67, 74, 14, 15, 9, 23, 12, 49, 50, 16, 10, 11, 51, 52, 53]
NATIVE_TANKS = [13, 42, 67, 74]
NATIVE_GREY = 10               # HUD.init greys cells 0..9
# (body that carries the oxygen [row, col], helmet whose icon it shows [row, col], hudItems cell)
EXOS = [
    ('duty 1805 -> helmet 1800', (1, 6), (3, 6), 28),
    ('freedom 1807 -> helmet 1808', (4, 6), (5, 6), 29),
]


def icon_id(row, col):
    return (col - 1) * ROWS + row - 1


# SWF tags ----------------------------------------------------------------
def read(path):
    data = Path(path).read_bytes()
    assert data[:3] in (b'CWS', b'FWS'), path
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
    return data[3], body[:head], tags


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


def image_index(tags, cid):
    found = [i for i, (c, b) in enumerate(tags) if c in (6, 20, 21, 35, 36, 90) and struct.unpack_from('<H', b)[0] == cid]
    assert len(found) == 1, f'character {cid}: {len(found)} image tags'
    return found[0]


def lossless2(payload):
    """(width, height, premultiplied ARGB) of a format-5 DefineBitsLossless2 payload."""
    fmt, w, h = payload[2], *struct.unpack_from('<HH', payload, 3)
    assert fmt == 5, f'DefineBitsLossless2 format {fmt}'
    argb = bytearray(zlib.decompress(payload[7:]))
    assert len(argb) == 4 * w * h
    return w, h, argb


def jpeg3(payload):
    """(width, height, premultiplied ARGB) of a DefineBitsJPEG3 payload. Its colour is stored premultiplied
    (no colour channel exceeds alpha beyond the JPEG's noise); a channel above alpha is clamped to it."""
    from PIL import Image
    n, = struct.unpack_from('<I', payload, 2)
    img = Image.open(io.BytesIO(payload[6:6 + n])).convert('RGB')
    w, h = img.size
    alpha = zlib.decompress(payload[6 + n:])
    assert len(alpha) == w * h
    rgb = img.tobytes()
    argb = bytearray(4 * w * h)
    for i in range(w * h):
        a = alpha[i]
        argb[4 * i] = a
        argb[4 * i + 1] = min(rgb[3 * i], a)
        argb[4 * i + 2] = min(rgb[3 * i + 1], a)
        argb[4 * i + 3] = min(rgb[3 * i + 2], a)
    return w, h, argb


def get_cell(w, argb, x, y):
    return b''.join(bytes(argb[4 * ((y + r) * w + x):4 * ((y + r) * w + x + CELL_W)]) for r in range(CELL_H))


def put_cell(w, argb, x, y, pixels):
    for r in range(CELL_H):
        at = 4 * ((y + r) * w + x)
        argb[at:at + 4 * CELL_W] = pixels[4 * CELL_W * r:4 * CELL_W * (r + 1)]


# ABC ---------------------------------------------------------------------
def doabc(tags, name):
    found = [i for i, (c, b) in enumerate(tags) if c == 82 and b[4:4 + len(name) + 1] == name.encode() + b'\0']
    assert len(found) == 1, f'no single DoABC2 {name}'
    return found[0]


def class_abc(tags, name):
    i = doabc(tags, name)
    payload = tags[i][1]
    head = 4 + len(name) + 1
    return i, payload[:head], abc_patch.Abc(payload[head:])


def branch_targets(code):
    """(instruction start, operand positions, base, old targets) of every branch of `code`."""
    out = []
    for start, op, args, length in instructions(code):
        if op in range(0x0c, 0x1b):
            out.append((start, [start + 1], start + length, [start + length + args[0]]))
        elif op == 0x1b:
            default, cases = args[0]
            _, p = read_u30(code, start + 4)
            out.append((start, [start + 1] + [p + 3 * c for c in range(len(cases))], start,
                        [start + default] + [start + c for c in cases]))
    return out


def insert(body, offset, code, stack_extra):
    """Insert `code` at instruction boundary `offset` of `body`, re-targeting every branch so that it reaches
    the instruction it reached before; a forward branch to `offset` itself reaches the inserted code (a
    backward one is refused). Exception ranges move with the code."""
    old = bytes(body['code'])
    ins = instructions(old)
    assert offset in {i[0] for i in ins} | {len(old)}, 'not an instruction boundary'
    n = len(code)
    moved = lambda pos: pos + n if pos > offset else pos
    shifted = lambda pos: pos + n if pos >= offset else pos
    new = bytearray(old)
    expected = []
    for start, positions, base, targets in branch_targets(old):
        for pos, target in zip(positions, targets):
            assert not (target == offset and start >= offset), 'a backward branch reaches the insertion point'
            delta = moved(target) - (base + n if start >= offset else base)
            assert -(1 << 23) <= delta < (1 << 23)
            new[pos:pos + 3] = (delta & 0xffffff).to_bytes(3, 'little')
        expected.append((shifted(start), [moved(t) for t in targets]))
    new[offset:offset] = code
    body['code'] = new
    for e in body['exceptions']:
        for k in (0, 1, 2):
            if e[k] >= offset:
                e[k] += n
    body['fields'][1] += stack_extra
    # The old instructions, moved, with every branch reaching its old instruction (or the inserted code).
    after = {i[0]: i for i in instructions(bytes(new))}
    for start, op, args, length in ins:
        assert shifted(start) in after and after[shifted(start)][1] == op, 'instruction stream changed'
    found = {s: t for s, _, _, t in branch_targets(bytes(new))}
    for start, targets in expected:
        assert found[start] == targets, f'branch at {start} retargeted wrongly'


def pushbyte(v):
    return bytes([0x24, v & 0xff])


def patch_quick_slots(tags):
    i, prefix, abc = class_abc(tags, 'survarium/controls/QuickSlots')
    native = b''.join(pushbyte(v) for v in NATIVE_HUD_ICONS) + bytes([0x56]) + u30(len(NATIVE_HUD_ICONS))
    body = abc.body_with(lambda b: native in b['code'])
    assert bytes(body['code']).count(native) == 1
    at = bytes(body['code']).index(native) + len(native) - 1 - len(u30(len(NATIVE_HUD_ICONS)))
    table = list(NATIVE_HUD_ICONS)
    for _, body_icon, _, cell in sorted(EXOS, key=lambda e: e[3]):
        assert cell >= len(table)
        table += [-1] * (cell - len(table)) + [icon_id(*body_icon)]
    assert all(-128 <= v < 128 for v in table)
    extra = table[len(NATIVE_HUD_ICONS):]
    insert(body, at, b''.join(pushbyte(v) for v in extra), len(extra))
    at += 2 * len(extra)
    assert body['code'][at:at + 1 + len(u30(len(NATIVE_HUD_ICONS)))] == bytes([0x56]) + u30(len(NATIVE_HUD_ICONS))
    assert len(u30(len(table))) == len(u30(len(NATIVE_HUD_ICONS)))
    body['code'][at + 1:at + 1 + len(u30(len(table)))] = u30(len(table))
    tags[i][1] = prefix + abc.bytes()
    return table


def patch_hud_init(tags, first, end):
    """A second copy of HUD.init's grey-icon loop, over first <= i < end."""
    i, prefix, abc = class_abc(tags, 'HUD')
    gray = abc.find_string('createGray::')
    body = abc.body_with(lambda b: bytes([0x2c]) + u30(gray) in b['code'])
    code = bytes(body['code'])
    ins = instructions(code)
    by_start = {x[0]: k for k, x in enumerate(ins)}
    marker = [x for x in ins if x[1] == 0x2c and x[2] == [gray]]
    assert len(marker) == 1
    # The loop's back edge: `getscopeobject 1; getslot S; pushbyte 10; iflt <body>` with the body before the marker.
    loops = [x for x in ins if x[1] == 0x15 and x[0] + x[3] + x[2][0] < marker[0][0] < x[0]]
    assert len(loops) == 1, 'grey-icon loop not found'
    back = loops[0]; k = by_start[back[0]]
    cond = ins[k - 3:k]
    assert [c[1] for c in cond] == [0x65, 0x6c, 0x24] and cond[0][2] == [1] and cond[2][2] == [NATIVE_GREY], 'loop condition'
    slot = cond[1][2][0]
    # The entry: `getscopeobject 1; pushbyte 0; convert_u; setslot S; jump <condition>`.
    jumps = [x for x in ins if x[1] == 0x10 and x[0] + x[3] + x[2][0] == cond[0][0]]
    assert len(jumps) == 1
    j = by_start[jumps[0][0]]
    entry = ins[j - 4:j]
    assert [e[1] for e in entry] == [0x65, 0x24, 0x74, 0x6d] and entry[0][2] == [1] and entry[1][2] == [0] and entry[3][2] == [slot], 'loop entry'
    start, end_at = entry[0][0], back[0] + back[3]
    copy = bytearray(code[start:end_at])
    # Inside the copy every branch is relative and stays inside it.
    for x in instructions(bytes(copy)):
        if x[1] in range(0x0c, 0x1b):
            assert 0 <= x[0] + x[3] + x[2][0] < len(copy)
        assert x[1] != 0x1b
    p0 = entry[1][0] - start; assert copy[p0:p0 + 2] == pushbyte(0); copy[p0:p0 + 2] = pushbyte(first)
    p1 = cond[2][0] - start; assert copy[p1:p1 + 2] == pushbyte(NATIVE_GREY); copy[p1:p1 + 2] = pushbyte(end)
    assert all(end_at not in t for _, _, _, t in branch_targets(code)), 'a branch reaches the end of the loop'
    insert(body, end_at, bytes(copy), 0)
    tags[i][1] = prefix + abc.bytes()


def patch_tank_test(tags, ids):
    """`icon_id != 13 && icon_id != 42 && icon_id != 67 && icon_id != 74` gets `&& icon_id != <id>` per id."""
    i, prefix, abc = class_abc(tags, 'survarium/controls/QuickItemRenderer')
    tail = pushbyte(NATIVE_TANKS[-1]) + bytes([0xab, 0x96, 0x76])        # pushbyte 74; equals; not; convert_b
    body = abc.body_with(lambda b: tail in b['code'])
    code = bytes(body['code'])
    assert code.count(tail) == 1
    ins = instructions(code)
    k = {x[0]: j for j, x in enumerate(ins)}[code.index(tail)]
    # `<icon_id> pushbyte 13 equals not convert_b`, then per further id
    # `dup convert_b iffalse <next> pop <icon_id> pushbyte <id> equals not convert_b`, then `iffalse <else>`.
    read_id = None
    for j, tank in enumerate(NATIVE_TANKS):
        at = k - 11 * (len(NATIVE_TANKS) - 1 - j)
        assert [x[1] for x in ins[at - 3:at + 4]] == [0x5d, 0x66, 0x66, 0x24, 0xab, 0x96, 0x76] and ins[at][2] == [tank], f'tank {tank}'
        this_read = code[ins[at - 3][0]:ins[at][0]]
        assert read_id in (None, this_read); read_id = this_read
        if j:
            link = ins[at - 7:at - 3]
            assert [x[1] for x in link] == [0x2a, 0x76, 0x12, 0x29], f'tank {tank} link'
            next_at = ins[at + 4][0]
            assert link[2][0] + link[2][3] + link[2][2][0] == next_at, f'tank {tank} short circuit'
    end = ins[k + 4]
    assert end[1] == 0x12, 'tank test end'
    insertion = b''
    for v in ids:
        rest = bytes([0x29]) + read_id + pushbyte(v) + bytes([0xab, 0x96, 0x76])
        insertion += bytes([0x2a, 0x76, 0x12]) + len(rest).to_bytes(3, 'little', signed=True) + rest
    insert(body, end[0], insertion, 0)
    tags[i][1] = prefix + abc.bytes()


def draw_icons(tags, lobby_tags):
    hs, ls = symbols(tags), symbols(lobby_tags)
    hi = image_index(tags, hs['hudItems'])
    code, payload = tags[hi]
    assert code == 35, 'hudItems is not the native DefineBitsJPEG3'
    w, h, argb = jpeg3(payload)
    assert (w, h) == (2 * CELL_W, 2048), (w, h)
    li = image_index(lobby_tags, ls['LobbyMenu_EquipmentIcons'])
    assert lobby_tags[li][0] == 36
    lw, lh, largb = lossless2(lobby_tags[li][1])
    used = []
    for cell in range(2 * HUD_COLUMN):
        x, y = CELL_W * (cell // HUD_COLUMN), CELL_H * (cell % HUD_COLUMN)
        if any(get_cell(w, argb, x, y)[0::4]):
            used.append(cell)
    assert used == list(range(28)), f'hudItems cells in use: {used}'
    for name, _, (row, col), cell in EXOS:
        icon = get_cell(lw, largb, CELL_W * (col - 1), LOBBY_PITCH * (row - 1))
        assert any(icon[0::4]), f'{name}: lobby cell [{row}, {col}] is empty'
        put_cell(w, argb, CELL_W * (cell // HUD_COLUMN), CELL_H * (cell % HUD_COLUMN), icon)
    tags[hi] = [36, payload[:2] + bytes([5]) + struct.pack('<HH', w, h) + zlib.compress(bytes(argb), 9)]


def main(hud, lobby, out):
    version, head, tags = read(hud)
    _, _, lobby_tags = read(lobby)
    draw_icons(tags, lobby_tags)
    table = patch_quick_slots(tags)
    cells = sorted(e[3] for e in EXOS)
    assert cells == list(range(cells[0], cells[-1] + 1))
    patch_hud_init(tags, cells[0], cells[-1] + 1)
    patch_tank_test(tags, [icon_id(*e[1]) for e in EXOS])
    write(out, version, head, tags)
    print(f'hudIcons: {table}')
    for name, body_icon, helmet, cell in EXOS:
        print(f'{name}: icon id {icon_id(*body_icon)} -> hudItems cell {cell} (lobby icon {list(helmet)})')


if __name__ == '__main__':
    main(*sys.argv[1:])
