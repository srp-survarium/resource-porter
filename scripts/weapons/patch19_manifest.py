#!/usr/bin/env python3
"""patch19_manifest.py: the port manifest of patch 19's 0.29a3 weapons, read back from patch 19.

usage: python3 -I patch19_manifest.py --donor <leaves dir> --patch19 <leaves dir> --stock <leaves dir>
           --dict-patch19 <leaves> --dict-stock <leaves> > ports/weapons-029a3-patch19.json

Each <leaves dir> holds `vostok-port dump-db-config --leaves` dumps named <config basename>.leaves.
The manifest takes from patch 19 what its porter decided (ids, prices, ammo, the balance and the
icon numbers, as `set`); everything else is the tool's to reproduce. `compare-weapons` then
checks that the tool's port of the donor gives patch 19's files.
"""
import argparse, json, os, re, sys

NATIVE = ('parameters.degrade_per_match', 'parameters.degrade_per_use', 'parameters.repair_cost_ratio',
          'parameters.available_modifications_mask', 'parameters.min_damage_amount', 'particles.shoot_pfx_count')
# the porter's decisions: every leaf under these that patch 19 has otherwise than the donor
DECIDED = re.compile(r'^(parameters\.|recoil\.|dispersion\.|ui_desc\.(icon|text_descriptions)|addons\.rifle_scope_dict_id)')

def load(p):
    r = {}
    for l in open(p, errors='replace'):
        k, _, v = l.rstrip('\n').partition('\t')
        r[k] = v
    return r

def value(v):
    kind, _, x = v.partition(':')
    if kind == 'i32': return int(x)
    if kind == 'f32': return float(x)
    if kind == 'bool': return x == 'true'
    if kind == 'string': return json.loads(x)
    raise ValueError(v)

def rows(p):
    out = {}
    for k, v in load(p).items():
        m = re.match(r'items_dict\[(\d+)\]\.(\w+)$', k)
        if m: out.setdefault(int(m.group(1)), {})[m.group(2)] = value(v)
    return {r['dict_id']: r for r in out.values()}

a = argparse.ArgumentParser()
for f in ('donor', 'patch19', 'stock', 'dict_patch19', 'dict_stock'):
    a.add_argument('--' + f.replace('_', '-'), required=True)
o = a.parse_args()
p19 = rows(o.dict_patch19)
stock = {r['cfg_name']: r for r in rows(o.dict_stock).values()}
compat = {}
for k, v in load(o.dict_patch19).items():
    m = re.match(r'items_compatibility\.(compatibility_\d+)\.(first|second)_item$', k)
    if m: compat.setdefault(m.group(1), {})[m.group(2)] = value(v)
weapons = []
for i in sorted(p19):
    r = p19[i]
    if not (2211 <= i <= 2252): continue
    base = os.path.basename(r['cfg_name'])[:-len('.options')]
    ours, donor = load(f'{o.patch19}/{base}.leaves'), load(f'{o.donor}/{base}.leaves')
    # the template: a stock weapon of the same category whose native values patch 19 has
    best = None
    plain = re.sub(r'(_premium(_2)?|_camo|_special)$', '', base)
    copy = plain != base and f'gameplay/items/weapons/{plain}.options' in stock
    for cfg, s in stock.items():
        if copy and cfg != f'gameplay/items/weapons/{plain}.options':
            continue
        if s['item_category'] != r['item_category'] or not cfg.startswith('gameplay/items/weapons/wpn_'):
            continue
        sl = load(f"{o.stock}/{os.path.basename(cfg)[:-8]}.leaves")
        norm = lambda d: {re.sub(r"\[\d+\]", "[]", k) for k in d if not k.startswith(("states.", "recoil_animations", "user_animations"))}
        miss = sum(sl.get(k) != ours.get(k) for k in NATIVE) * 1000 + len(norm(ours) - norm(sl)) * 100 + len(norm(sl) ^ norm(ours))
        if best is None or miss < best[0]: best = (miss, cfg)
    if best is None:
        sys.exit(f'{base}: no stock weapon of category {r["item_category"]}')
    tl = load(f"{o.stock}/{os.path.basename(best[1])[:-8]}.leaves")
    st = {}
    for k, v in ours.items():
        # the balance: every decided leaf the template has (its others are the tool's)
        if DECIDED.match(k) and k not in NATIVE and (k in tl or k in donor):
            st[re.sub(r'\[(\d+)\]', r'.\1', k)] = value(v)
    weapons.append({
        'name': r['name'], 'dict_id': i, 'donor_item': r['cfg_name'], 'config': r['cfg_name'],
        'template': best[1], 'base_cost': r['base_cost'], 'copy_of_template': copy, 'premium': r['is_premium'],
        'ammo': sorted(c['second'] for c in compat.values() if c.get('first') == i),
        'set': st, 'lobby_icon': None,
        'kill_feed': {'icon': value(ours['ui_desc.combat_log_icon']), 'donor_png': None},
    })
stock_ids = set(rows(o.dict_stock))
ext = sorted({a for w in weapons for a in w['ammo'] if a not in stock_ids})
json.dump({'format_version': 1, 'id': 'weapons-029a3-patch19', 'donor_build': '0.29a3', 'external_ammo': ext,
           'weapons': weapons}, sys.stdout, indent=1, ensure_ascii=False)
print()
