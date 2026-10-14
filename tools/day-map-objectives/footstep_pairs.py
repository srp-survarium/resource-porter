import re, collections, json, sys
S = sys.argv[1]
def table(path, rx):
    m = collections.defaultdict(dict)
    for line in open(path):
        k, _, v = line.rstrip('\n').partition('\t')
        g = re.match(rx, k)
        if g: m[int(g.group(1))][g.group(2)] = v.split(':', 1)[1].strip('"')
    return m
mats = table(f'{S}/gm_026.leaves', r'materials\[(\d+)\]\.(id|name|physic\.base_material|physic\.walkable|deleted)$')
byid = {int(d['id']): d for d in mats.values()}
name = {i: d['name'] for i, d in byid.items()}
ids = {d['name']: i for i, d in byid.items()}
pairs = {}
for p in table(f'{S}/pairs026.leaves', r'pairs\[(\d+)\]\.(mtrl_1_id|mtrl_2_id|collision_sound)$').values():
    a, b = int(p['mtrl_1_id']), int(p['mtrl_2_id'])
    pairs[(a, b)] = pairs[(b, a)] = p['collision_sound']
actors = [i for i, n in name.items() if n.startswith('actor')]
def step(mid):
    d = byid.get(mid, {})
    base = d.get('physic.base_material')
    out = {name[a]: pairs.get((a, mid)) for a in actors}
    return d.get('name'), base, out
if __name__ == '__main__':
    print('actors', {i: name[i] for i in actors})
    for mid in sorted(byid):
        n, base, out = step(mid)
        if n.startswith('actor'): continue
        print(mid, n, 'base=', base, {k: (v.split('/')[-1] if v else v) for k, v in out.items()})
