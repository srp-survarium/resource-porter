#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""The 0.69d0 files the ported weapons draw, as a list of paths.

    weapons-069d-donor.py <vostok-port with split support> <0.69d0 .split> <paths069.txt> <out> <weapon key>..

Reads `gameplay/game.options` (each weapon's model, skeleton, bundled and
default modules, clips and sounds) and follows models -> materials ->
textures and composite sounds -> collections -> single sounds. Writes
<out> (one path a line; 0.69d's base, antidote, bandage and medkit clips and
per-location reverbs left out, as the port drops them), for

    vostok-port extract-bundle --db <split> --prefix <each path> --output TREE
    (cp gameplay/game.options in) ; cargo run --example pack-donor-tree -- TREE/overlay donor.db
"""
import collections
import re
import subprocess
import sys

P, SPLIT, PATHS, OUT = sys.argv[1:5]
guns = sys.argv[5:]
paths = set(open(PATHS).read().split('\n'))


def dump(p):
    r = subprocess.run([P, 'dump-db-config', '--db', SPLIT, '--path', p, '--leaves'], capture_output=True, text=True)
    d = {}
    for line in r.stdout.splitlines():
        k, _, v = line.partition('\t')
        d[k] = v
    return d


game = dump('gameplay/game.options')
cfg = collections.defaultdict(dict)
mods = collections.defaultdict(dict)
for k, v in game.items():
    if k.startswith('weapons.'):
        n, _, r = k[8:].partition('.')
        if n in guns:
            cfg[n][r] = v
    elif k.startswith('weapon_modules.'):
        n, _, r = k[15:].partition('.')
        mods[n][r] = v


def sv(v):
    return v.split(':', 1)[1].strip('"') if ':' in v else v


need = {'gameplay/game.options'}


def add(p):
    if p in paths:
        need.add(p)
        return True
    return False


seen = set()


def material(mt):
    if mt in seen:
        return
    seen.add(mt)
    p = f'material_instances/{mt}.material'
    if not add(p):
        return
    for v in dump(p).values():
        if v.startswith('string:'):
            t = sv(v)
            if '/' in t and add(f'textures/{t}.dds'):
                add(f'textures/{t}.options')


def model(m):
    for kind in ('skinned_model', 'model'):
        root = f'models/{m}.{kind}'
        files = [p for p in paths if p.startswith(root + '/')]
        if files:
            need.update(files)
            for k, v in dump(f'{root}/settings').items():
                if k.endswith('.material_name'):
                    material(sv(v))
            return
    sys.exit(f'no model {m}')


def sound(kind, name):
    if (kind, name) in seen:
        return
    seen.add((kind, name))
    if kind == 256:
        for ext in ('high.ogg', 'high.rms', 'raw_options'):
            add(f'sounds/single/{name}.{ext}')
    elif kind == 258:
        p = f'sounds/collections/{name}.sound_collection'
        if add(p):
            for k, v in dump(p).items():
                if k.endswith('.filename'):
                    sound(256, sv(v))
    elif kind == 257:
        p = f'sounds/composite/{name}.composite_sound'
        if add(p):
            items = collections.defaultdict(dict)
            for k, v in dump(p).items():
                m = re.search(r'sound_items\[(\d+)\]\.(filename|resource_type)$', k)
                if m:
                    items[m.group(1)][m.group(2)] = sv(v)
            for it in items.values():
                if not it['filename'].startswith('weapons/environments/'):
                    sound(int(it.get('resource_type', '258')), it['filename'])


for g in guns:
    c = cfg[g]
    model(sv(c['object.model']))
    # The kill log icon of its ui_desc.combat_log_icon (icon_<n + 1>.png).
    add(f"flash_movies/textures/ui_icons/log/icon_{int(sv(c['ui_desc.combat_log_icon'])) + 1}.png")
    add(f"animations/skeletons/{sv(c['object.skeleton'])}.skeleton")
    for k, v in c.items():
        if re.match(r'(bundled|default)_modules\[\d+\]\.module$', k):
            md = mods[sv(v)]
            for mk in [k for k in md if k == 'model' or k == 'aimed_model' or k == f'exceptions.{g}.model']:
                model(sv(md[mk]))
        s = sv(v)
        if s.startswith('resources/animations/single/') and not re.search(r'/(base|antidote|bandage|medkit_injector)/', s):
            add(s[len('resources/'):] + '.b-spline')
            add(s[len('resources/'):] + '.bones_names')
    pairs = collections.defaultdict(dict)
    for k, v in c.items():
        m = re.match(r'(.*\.sounds\.[a-z_]+\[\d+\])\[(0|1)\]$', k)
        if m:
            pairs[m.group(1)][m.group(2)] = sv(v)
    for pr in pairs.values():
        if '0' in pr and '1' in pr and 'silencer' not in pr['1']:
            sound(int(pr['0']), pr['1'])
open(OUT, 'w').write('\n'.join(sorted(need)) + '\n')
print(f'{len(need)} files')
