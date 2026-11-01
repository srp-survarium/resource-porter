#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Write ports/muzzle-devices-069d.json from 0.69d0's game.options and our dictionary.

    muzzle-devices-manifest.py <game.options leaves> <our dictionary leaves> <guns.tsv> <devices.tsv> <out.json> [<gun>...]

<game.options leaves>  `vostok-port dump-db-config --leaves` of 0.69d0's gameplay/game.options.
<our dictionary leaves> the same of the players' gameplay/db_static_dictionaries.
<guns.tsv>             our guns: item name, 0.69d0 weapon, its built-in module (or -), the 0.69d0
                       weapon whose fits it takes (or -), the fit tolerance in mm (or -), a note.
<devices.tsv>          the devices' dict ids and icon cells (module, dict id, row, col): kept as they are;
                       a device not in it yet is appended (the next id and cell), so ids never move.
<gun>...               only these guns (by item name), and the devices that fit them.

The devices are every 0.69d0 module players could get (it has drop infos) that attaches to a
muzzle slot, is no suppressor (by its name, or a suppressor's shot effect) and fits one of the guns (module_to_item_links). New ones take
dict ids from 2601 and icon cells of LobbyMenu_AmmoIcons column 5, then 6, in devices.tsv.
Prices: a brake 6000, a flash hider 4500 [srp].
"""
import json
import re
import sys

LEAVES, DICT, GUNS, TABLE, OUT = sys.argv[1:6]
only = set(sys.argv[6:])
FIRST_DICT = 2601
FIRST_COL, ROWS = 5, 23
PRICE = {'brake': 6000, 'flash_hider': 4500}
GROUPS = {'compensator_h', 'compensator_u', 'compensator_v', 'flash_hider', 'firearrestor', 'muzzle'}


def value(v):
    v = v.split(':', 1)[1] if ':' in v else v
    return v.strip('"')


modules, links, infos = {}, {}, set()
for line in open(LEAVES, encoding='utf-8'):
    k, _, v = line.rstrip('\n').partition('\t')
    m = re.match(r'weapon_modules\.([^.]+)\.(.*)', k)
    if m:
        modules.setdefault(m.group(1), {})[m.group(2)] = value(v)
        continue
    m = re.match(r'module_to_item_links\[(\d+)\]\.(\w+)', k)
    if m:
        links.setdefault(m.group(1), {})[m.group(2)] = value(v)
        continue
    m = re.match(r'modules_drop_options\.modules_additional_infos\[(\d+)\]\.dict_id', k)
    if m:
        infos.add(value(v))
fits = {}
for l in links.values():
    fits.setdefault(l['module'], set()).add(l['item'])

items = {}
for line in open(DICT, encoding='utf-8'):
    k, _, v = line.rstrip('\n').partition('\t')
    m = re.match(r'items_dict\[(\d+)\]\.(\w+)', k)
    if m:
        items.setdefault(m.group(1), {})[m.group(2)] = value(v)
config_of = {r['name']: r['cfg_name'] for r in items.values() if 'name' in r}

guns = []
for line in open(GUNS, encoding='utf-8'):
    if not line.strip() or line.startswith('#'):
        continue
    name, donor, stock, like, tol, note = (line.rstrip('\n').split('\t') + [''] * 6)[:6]
    guns.append(dict(name=name, donor=donor, stock=None if stock == '-' else stock,
                     like=None if like in ('-', '') else like, tol=None if tol in ('-', '') else float(tol),
                     note=note or None))


def devices_for(gun_list):
    keys = {g['like'] or g['donor'] for g in gun_list}
    out = []
    for mid, m in sorted(modules.items()):
        slot = m.get('slots_to_attach_to[0].slot', '')
        if not slot.endswith('_muzzle') or m.get('group') not in GROUPS:
            continue
        if re.search(r'silencer|suppressor|pbs', mid + m.get('model', '')):
            continue
        # A device 0.69d0 fires with a suppressor's effect is one; so is every Zenit DTK-4 (a
        # sound moderator, a closed can, even where 0.69d0 calls one a flash hider).
        if 'suppr' in m.get('attach_params.effects_to_override.particles.shoot', '') or 'dtk4' in mid:
            continue
        if m.get('parameters.dict_id') not in infos:
            continue
        if fits.get(mid, set()) & keys:
            out.append(mid)
    return out


everything = devices_for(guns)
ids, cells = {}, {}
try:
    for line in open(TABLE, encoding='utf-8'):
        if line.startswith('#') or not line.strip():
            continue
        mid, d, r, c = line.rstrip('\n').split('\t')
        ids[mid], cells[mid] = int(d), [int(r), int(c)]
except FileNotFoundError:
    pass
for mid in everything:
    if mid not in ids:
        n = len(ids)
        assert n < 2 * ROWS, 'columns 5 and 6 are full'
        ids[mid], cells[mid] = FIRST_DICT + n, [1 + n % ROWS, FIRST_COL + n // ROWS]
with open(TABLE, 'w', encoding='utf-8') as f:
    f.write('# Muzzle devices: 0.69d0 module, our dict id, its LobbyMenu_AmmoIcons cell (row, col). Append only.\n')
    for mid in sorted(ids, key=ids.get):
        f.write(f'{mid}\t{ids[mid]}\t{cells[mid][0]}\t{cells[mid][1]}\n')
chosen = [g for g in guns if not only or g['name'] in only]
assert not only or len(chosen) == len(only), only - {g['name'] for g in chosen}
devices = devices_for(chosen)


def kind(mid):
    m = modules[mid]
    return 'flash_hider' if m.get('group') in ('flash_hider', 'firearrestor') or 'firearrestor' in mid else 'brake'


manifest = {
    'format_version': 1,
    'id': 'muzzle-devices',
    'flash_hider_shoot': 'fx/weapons/9x39',
    'devices': [{'module': mid, 'dict_id': ids[mid], 'base_cost': PRICE[kind(mid)], 'icon': cells[mid]}
                for mid in devices],
    'guns': [],
}
for g in chosen:
    e = {'config': config_of[g['name']], 'donor': g['donor']}
    if g['stock']:
        e['stock'] = g['stock']
    if g['like']:
        e['fits_like'] = g['like']
    if g['tol'] is not None:
        e['fit_tolerance_mm'] = g['tol']
    if g['note']:
        e['note'] = g['note']
    manifest['guns'].append(e)
open(OUT, 'w').write(json.dumps(manifest, indent=1, ensure_ascii=False) + '\n')
print(f'{len(manifest["devices"])} devices of {len(everything)}, {len(manifest["guns"])} guns', file=sys.stderr)
