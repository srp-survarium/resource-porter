#!/usr/bin/env python3
"""Static render budget of one 0.26 client project.

    map_budget.py --porter VOSTOK DB PROJECT [--cache DIR]

Counts, over every render visual (as if all were in view):
instances, surfaces (draw calls with instancing off, the shipped user.cfg),
triangles at the first and last LOD, instances with LODs or custom LOD
distances, sun-shadow casters (surfaces whose material sets
g_stage.is_cast_shadow) and the texture bytes the visible materials bind
(DDS sizes, every mip). Grass is reported from the vegetation layers' .veg
instance counts: the grass system draws, LODs and culls it separately.
"""
import argparse
import collections
import json
import os
import re
import struct
import subprocess
import tempfile


class Db:
    def __init__(self, porter, db, cache):
        self.porter, self.db, self.cache = porter, db, cache
        os.makedirs(cache, exist_ok=True)

    def _run(self, *args):
        return subprocess.run([self.porter, *args, '--db', self.db], capture_output=True, text=True)

    def leaves(self, path):
        out = os.path.join(self.cache, path.replace('/', '_') + '.leaves')
        if not os.path.exists(out):
            r = self._run('dump-db-config', '--path', path, '--leaves')
            if r.returncode:
                return None
            open(out, 'w').write(r.stdout)
        return dict(line.split('\t', 1) for line in open(out).read().splitlines() if '\t' in line)

    def read(self, path):
        out = os.path.join(self.cache, path.replace('/', '_'))
        if not os.path.exists(out):
            if subprocess.run([self.porter, 'extract', '--db', self.db, '--path', path, '--output', out],
                              capture_output=True).returncode:
                return None
        return open(out, 'rb').read()

    def size(self, path):
        r = self._run('stat', '--path', path)
        m = re.search(r'uncompressed_size=(\d+)', r.stdout)
        return int(m.group(1)) if m else None


def triangles(blob):
    i = 0
    while blob and i + 8 <= len(blob):
        cid, sz = struct.unpack_from('<II', blob, i)
        if cid == 4:
            return struct.unpack_from('<I', blob, i + 8)[0] // 3
        i += 8 + sz
    return 0


def string(value):
    return value.split(':', 1)[1].strip('"') if value and value.startswith('string:') else None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--porter', required=True)
    ap.add_argument('db')
    ap.add_argument('project')
    ap.add_argument('--cache', default=os.path.join(tempfile.gettempdir(), 'map-budget-cache'))
    a = ap.parse_args()
    db = Db(a.porter, a.db, a.cache)
    project = db.leaves(a.project)
    instances = collections.Counter(string(v) for k, v in project.items()
                                    if re.fullmatch(r'render_visuals\[\d+\]\.lib_name', k))
    totals = collections.Counter()
    textures, materials_seen, per_model = set(), {}, {}
    for name, count in instances.items():
        base = name.replace('\\', '/').removeprefix('resources/')
        model = base if base.endswith('.model') else f'models/{base}.model'
        settings = db.leaves(model + '/settings') or {}
        lods = collections.defaultdict(list)
        for k, v in settings.items():
            m = re.fullmatch(r'lod_hierrarchy\.(LOD\d)\.surfaces\[\d+\]', k)
            if m:
                lods[m.group(1)].append(string(v))
        levels = sorted(l for l in lods if lods[l])
        if not levels:
            totals['instances_without_surfaces'] += count
            continue
        tris = {l: sum(triangles(db.read(f'{model}/render/{s}/converted_model')) for s in lods[l]) for l in levels}
        shadow_surfaces = 0
        for s in lods[levels[0]]:
            material = string(settings.get(f'material_settings.{s}.material_name'))
            if not material:
                continue
            if material not in materials_seen:
                ml = db.leaves(f'material_instances/{material}.material') or {}
                materials_seen[material] = ml
                for k, v in ml.items():
                    if 'texture' in k and k.endswith('.value') and string(v):
                        textures.add(string(v))
            if materials_seen[material].get('material.g_stage.is_cast_shadow.value') == 'bool:true':
                shadow_surfaces += 1
        totals['instances'] += count
        totals['draw_calls_lod0'] += count * len(lods[levels[0]])
        totals['triangles_lod0'] += count * tris[levels[0]]
        totals['triangles_last_lod'] += count * tris[levels[-1]]
        totals['shadow_casting_draws'] += count * shadow_surfaces
        if len(levels) > 1:
            totals['instances_with_lods'] += count
        if settings.get('lod_switching.default_params') == 'bool:false':
            totals['instances_custom_lod_distances'] += count
        per_model[name] = {'instances': count, 'triangles': [tris[l] for l in levels],
                           'surfaces': len(lods[levels[0]]), 'shadow_surfaces': shadow_surfaces}
    texture_bytes = 0
    for t in sorted(textures):
        size = db.size(f'textures/{t}.dds')
        texture_bytes += size or 0
    grass = collections.Counter()
    for k, v in project.items():
        m = re.fullmatch(r'client_objects\[\d+\]\.layers\[\d+\]\.filename', k)
        if m:
            root = '/'.join(a.project.split('/')[:2])
            blob = db.read(f'{root}/{string(v)}')
            i, n = 0, 0
            while blob and i + 8 <= len(blob):
                cid, sz = struct.unpack_from('<II', blob, i)
                if cid == 0x58:
                    n = sz // 12
                i += 8 + sz
            grass[string(v)] = n
    heavy = sorted(per_model.items(), key=lambda kv: -kv[1]['instances'] * kv[1]['triangles'][0])[:8]
    print(json.dumps({'project': a.project, **totals, 'unique_models': len(instances),
                      'materials': len(materials_seen), 'textures': len(textures),
                      'texture_mib': round(texture_bytes / 2**20, 1),
                      'grass_instances': sum(grass.values()), 'grass_layers': dict(grass),
                      'heaviest_models': dict(heavy)}, indent=1))


if __name__ == '__main__':
    main()
