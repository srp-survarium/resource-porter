#!/usr/bin/env python3
"""peer_scale.py: a ported gun's balance on the live scale, by a peer both builds ship.

usage: python3 -I peer_scale.py --donor <gun.leaves> --donor-peer <peer.leaves> --live-peer <peer.leaves>
           [--keep key ...] > set.json

Each .leaves is `vostok-port dump-db-config --leaves` of a weapon config. For every numeric
parameters.*, recoil.* and dispersion.* leaf the three have, the gun's value is
live_peer x donor / donor_peer (the owner's rule, 2026-11-01: the donor's own stats scaled by a peer
gun both builds share), so the gun differs from its peer as it did in its build. Where the peer is 0
in the donor, the donor's value if it has one, else the live peer's. A min/max pair (recoil kicks)
of recoil is scaled by one factor, the ratio of the pair's sums, so min stays below max. Only what
differs from the donor's own value is written (the port takes the donor's otherwise). Durability, repair,
levels, faction and drops are not balance (the native systems' and the port's own): left out, as
the keys of --keep. Rounded: distances and rates to integers, the rest to 0.01 (0.001 under 0.1).
"""
import argparse, json, re, sys

SKIP = {'parameters.degrade_per_match', 'parameters.degrade_per_use', 'parameters.repair_cost_ratio',
        'parameters.item_level', 'parameters.match_making_level', 'parameters.drop_weight',
        'parameters.faction', 'parameters.min_damage_amount', 'parameters.reliability'}
INTEGER = re.compile(r'(distance|rounds_per_minute|magazine_capacity|stamina_damage)$')

def load(p):
    r = {}
    for l in open(p):
        k, _, v = l.rstrip('\n').partition('\t')
        if v.startswith(('f32:', 'i32:')) and '[' not in k:
            r[k] = float(v[4:])
    return r

def rnd(k, v):
    if INTEGER.search(k):
        return int(round(v))
    return round(v, 3 if abs(v) < 0.1 else 2)

a = argparse.ArgumentParser()
a.add_argument('--donor', required=True)
a.add_argument('--donor-peer', required=True)
a.add_argument('--live-peer', required=True)
a.add_argument('--keep', nargs='*', default=[])
o = a.parse_args()
d, dp, lp = load(o.donor), load(o.donor_peer), load(o.live_peer)
keys = sorted(k for k in d if re.match(r'(parameters|recoil|dispersion)\.', k) and k in lp
              and k not in SKIP and k not in o.keep)
out, done = {}, set()
for k in keys:
    if k in done:
        continue
    pair = None
    m = re.match(r'(recoil\.)(min|max)_(.*)$', k)
    if m:
        other = f"{m.group(1)}{'max' if m.group(2) == 'min' else 'min'}_{m.group(3)}"
        if other in d and other in dp and other in lp:
            pair = other
    if pair:
        sd, sl = dp.get(k, 0) + dp[pair], lp[k] + lp[pair]
        f = sl / sd if sd else 1
        for kk in (k, pair):
            out[kk] = rnd(kk, d[kk] * f if sd else lp[kk])
            done.add(kk)
        continue
    if k in dp and dp[k] != 0:
        v = lp[k] * d[k] / dp[k]
    elif d[k] != 0:
        v = d[k]
    else:
        v = lp[k]
    out[k] = rnd(k, v)
out = {k: v for k, v in out.items() if abs(v - d[k]) > 1e-6}
json.dump(out, sys.stdout, indent=1, sort_keys=True)
print()
