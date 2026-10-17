#!/usr/bin/env python3
"""Plan the 194 growing track anomaly: a gameplay-fill pack of cloned native objects.

Usage: plan_track_anomaly.py LEVEL_02_PROJECT BUNDLE_OVERLAY OUTPUT_PACK [options]

Native template: the level_02 fruit anomaly, a generic_anomaly_core whose
states switch zone groups (dummy zone, toxic cloud, fx carrier zones with an
idle_particle at effect_zones offsets). Its state machine runs in the client
simulation: select_state takes the last enabled state whose energy_threshold
is reached; a state with active_time_sec sets energy to its energy_on_exit
when the time runs out; changing state deactivates the old state's zones and
activates the new state's.

Fog: one controller per stage (a state change would restart the fog of the
zones it keeps): state 0 holds an underground dummy zone for --start +
(k-1) * --interval seconds and exits with energy 1; state 1 (threshold 1,
permanent) holds the stage's fog carrier, whose rust_smoke clouds (level_03
rust fog: 100 particles, 15 s life, so a cloud fades in over ~7-15 s) sit
--cloud-height above the ground on each track. Players never stand in the
carrier's underground geometry, so its particle stays the idle one.

Damage: one controller with a timed state chain, idle until the first step,
then one face toxic zone per --damage step (time:max_hit). Step j's box
covers both tracks from the first cloud back to --lag metres behind the
newest cloud started by then; only one step is active at a time.
With --damage-cap H the step is instead one box per stage segment (--spacing
long, overlapping its neighbours by --overlap), each from --damage-depth below
to H above that segment's own track-bed ground (the lowest of the ground under
the tracks and between them at its ends and middle): a damage zone hits any
body part inside it (0.10b damage_zone_core: a contact test against the
character's bone collision), so players more than H above the bed (wagon
roofs, bridges, walkways) are outside it. The step's state holds all its
segments.

Timers count from the controllers' first tick (anomaly activation); the
calibration run tells whether that is map load or match start.
Placement is checked: every cloud must have a rail of the 194 railroad
collision within 1 m across and ground under it; clouds over a wagon are
listed. Prints the object counts.
"""
import argparse, collections, hashlib, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_respawns import World, config
from audit_collision_region import collision_parts

TEMPLATE = 'projects/level_02/day_2/client_project'
CORE, DUMMY = 'anomaly/fruit/fruit_anomaly', 'anomaly/fruit/fruit_toxic_cloud_dummy'
FOG, DAMAGE = 'anomaly/fruit/fx_locator/fruit_toxic_cloud_lvl1_fx', 'anomaly/fruit/fruit_toxic_cloud_lvl1'
GEOMETRY = 'anomaly/fruit/fruit_toxic_cloud_collision_lvl1'
ROOT = 'logic/fill/track_anomaly'
PARTICLE = 'fx/level_3_anomaly/rust_smoke'

def args():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('level_02', type=Path); ap.add_argument('overlay', type=Path); ap.add_argument('output', type=Path)
    ap.add_argument('--start', type=int, default=180, help='seconds before stage 1 (default 180)')
    ap.add_argument('--interval', type=int, default=15, help='seconds between stages (default 15)')
    ap.add_argument('--stages', type=int, default=16)
    ap.add_argument('--first-z', type=float, default=20.0, help='z of stage 1, at the moss-field end of the tracks')
    ap.add_argument('--spacing', type=float, default=5.0, help='metres between stages along the tracks')
    ap.add_argument('--tracks', default='12.9,18.6', help='x of the track centre lines')
    ap.add_argument('--cloud-height', type=float, default=0.85, help='cloud above the ground (level_03 median)')
    ap.add_argument('--damage', default='240:0.06,300:0.08,360:0.10,420:0.12', help='time_sec:max_hit steps')
    ap.add_argument('--lag', type=float, default=4.5, help='damage ends this far behind the newest cloud')
    ap.add_argument('--zone-margin', type=float, default=3.0, help='damage box beyond the outer tracks')
    ap.add_argument('--damage-cap', type=float, default=None, help='per-segment damage boxes up to this height above the local ground')
    ap.add_argument('--damage-depth', type=float, default=0.3, help='how far below the local ground those boxes reach')
    ap.add_argument('--overlap', type=float, default=0.25, help='each segment box reaches this far into its neighbours')
    ap.add_argument('--rails-from-z', type=float, default=None, help='clouds before this z bridge to another fog (no rail check; ground only)')
    ap.add_argument('--also-map-ids', default='', help='comma-separated further map ids the pack applies to')
    return ap.parse_args()

def main():
    a = args()
    tracks = [float(v) for v in a.tracks.split(',')]
    steps = [(int(t), float(h)) for t, h in (s.split(':') for s in a.damage.split(','))]
    assert steps == sorted(steps) and all(0 < h <= 1 for _, h in steps)
    level_02 = a.level_02.read_bytes()
    native = {o['full_name']: o for o in config(level_02)['client_objects'] if 'full_name' in o}
    for name in (CORE, DUMMY, FOG, DAMAGE, GEOMETRY): assert name in native, name
    project = config((a.overlay/'projects/v011e_level_04/day_2/client_project').read_bytes())
    world = World(a.overlay, project)
    rails = []
    for o in project['collision_objects']:
        if 'props/railroad/railroad' in o['lib_name'] and 'traffic' not in o['lib_name']:
            try: rails.extend(q['bounds'] for q in collision_parts(a.overlay, o))
            except FileNotFoundError: pass
    wagons = [v['position'] for v in project['render_visuals'] if v['lib_name'].endswith('vehicle/train/hopper_wagon')]

    def ground(x, z):
        f = world.floor(x, 2.4, z, below=1.5, above=0.7)
        assert f, f'no ground under ({x}, {z})'
        return f[0]
    def rail_near(x, z):
        return any(lo[0] - 1 <= x <= hi[0] + 1 and lo[2] - 2.5 <= z <= hi[2] + 2.5 for lo, hi, in (b[:2] for b in rails))

    clones, notes = [], collections.Counter()
    def clone(source, full_name, **fields):
        clones.append(dict(source=source, full_name=full_name, set=fields))
    group = lambda zones: dict(zones=zones, enabled=True, keep_all_zones_active=True, sequential_activation=False,
                               min_active_time_sec=0, max_active_time_sec=0, recharge_time_sec=20, max_charged_count=255)
    quiet = dict(energy_enabled=False, energy_initial=0, energy_maximum=0, energy_decrease_speed=0, energy_af_container_use=0,
                 **{f'energy_on_{k}': 0 for k in ('walk', 'run', 'sprint', 'jump', 'shoot', 'explosion', 'character_hit', 'character_kill')},
                 artefacts_enabled=False, max_artefacts_count=0, artefact_containers=[])
    def state(threshold, seconds, exit_energy, zones):
        return dict(enabled=True, energy_threshold=threshold, active_time_sec=seconds, energy_on_exit=exit_energy,
                    shoot_trigger=False, zone_activity_trigger=False, groups=[group(zones)])
    cx = sum(tracks) / len(tracks)
    stage_z, stage_t, report = [], [], []
    for k in range(1, a.stages + 1):
        z = a.first_z + a.spacing * (k - 1); t = a.start + a.interval * (k - 1)
        stage_z.append(z); stage_t.append(t)
        p = f'{ROOT}/stage_{k:02d}'
        heights = []
        for x in tracks:
            assert (a.rails_from_z is not None and z < a.rails_from_z) or rail_near(x, z), f'stage {k}: no rail near x {x} z {z}'
            heights.append(ground(x, z))
            over = [w for w in wagons if math.hypot(w[0] - x, w[2] - z) < 3.0]
            if over: report.append(f'stage {k:02d} cloud at ({x}, {z}) is beside a hopper wagon at {[round(v, 1) for v in over[0]]}')
        gy = round(sum(heights) / len(heights), 3)
        clone(GEOMETRY, f'{p}/carrier_geometry', position=[cx, round(gy - 20, 3), z],
              meshes=[dict(type=0, position=[0, 0, 0], rotation=[0, 0, 0], scale=[0.1, 0.1, 0.1])])
        carrier = [dict(name=f'{p}/carrier_geometry')]
        clone(DUMMY, f'{p}/idle_zone', position=[cx, gy, z], collision_geometries=carrier)
        clone(FOG, f'{p}/fog_zone', position=[cx, gy, z], max_hit=0, idle_particle=PARTICLE, collision_geometries=carrier,
              effect_zones=[dict(position=[round(x - cx, 3), round(h + a.cloud_height - gy, 3), 0], rotation=[0, 0, 0]) for x, h in zip(tracks, heights)])
        clone(CORE, f'{p}/controller', position=[cx, gy, z], **quiet,
              states=[state(0, t, 1, [f'{p}/idle_zone']), state(1, 0, 0, [f'{p}/fog_zone'])])
    # Damage: one controller, a timed chain idle -> step 1 -> ... -> last step.
    p = f'{ROOT}/damage'
    gy = ground(cx, stage_z[0])
    clone(GEOMETRY, f'{p}/carrier_geometry', position=[cx, round(gy - 20, 3), stage_z[0]],
          meshes=[dict(type=0, position=[0, 0, 0], rotation=[0, 0, 0], scale=[0.1, 0.1, 0.1])])
    clone(DUMMY, f'{p}/idle_zone', position=[cx, gy, stage_z[0]], collision_geometries=[dict(name=f'{p}/carrier_geometry')])
    half_x = (max(tracks) - min(tracks)) / 2 + a.zone_margin
    states, damage = [state(0, steps[0][0], 1, [f'{p}/idle_zone'])], []
    for j, (t, hit) in enumerate(steps, 1):
        newest = max((z for z, s in zip(stage_z, stage_t) if s <= t), default=None)
        assert newest is not None, f'damage step at {t} s precedes the first fog stage'
        z0, z1 = stage_z[0] - a.spacing, newest - a.lag
        assert z1 > z0, f'damage step {j} would be empty'
        nxt = steps[j][0] - t if j < len(steps) else 0
        if a.damage_cap is None:
            clone(GEOMETRY, f'{p}/zone_{j}_geometry', position=[cx, round(gy + 2.0, 3), round((z0 + z1) / 2, 3)],
                  meshes=[dict(type=1, position=[0, 0, 0], rotation=[0, 0, 0], scale=[round(half_x, 3), 2.5, round((z1 - z0) / 2, 3)])])
            clone(DAMAGE, f'{p}/zone_{j}', position=[cx, gy, round((z0 + z1) / 2, 3)], max_hit=hit,
                  collision_geometries=[dict(name=f'{p}/zone_{j}_geometry')])
            states.append(state(j, nxt, j + 1 if nxt else 0, [f'{p}/zone_{j}']))
            damage.append(dict(step=j, at_sec=t, max_hit=hit, z=[round(z0, 2), round(z1, 2)], newest_cloud_z=newest))
            continue
        zones, segs, za = [], [], z0
        while za < z1 - 1e-6:
            zb = min(za + a.spacing, z1)
            g = min(ground(x, zz) for x in tracks + [cx] for zz in (za, (za + zb) / 2, zb))
            lo, hi = g - a.damage_depth, g + a.damage_cap
            s0, s1 = za - a.overlap if za > z0 else za, zb + a.overlap if zb < z1 else zb
            k = len(segs) + 1
            clone(GEOMETRY, f'{p}/zone_{j}_{k:02d}_geometry', position=[cx, round((lo + hi) / 2, 3), round((s0 + s1) / 2, 3)],
                  meshes=[dict(type=1, position=[0, 0, 0], rotation=[0, 0, 0], scale=[round(half_x, 3), round((hi - lo) / 2, 3), round((s1 - s0) / 2, 3)])])
            clone(DAMAGE, f'{p}/zone_{j}_{k:02d}', position=[cx, round(g, 3), round((s0 + s1) / 2, 3)], max_hit=hit,
                  collision_geometries=[dict(name=f'{p}/zone_{j}_{k:02d}_geometry')])
            zones.append(f'{p}/zone_{j}_{k:02d}'); segs.append(dict(z=[round(s0, 2), round(s1, 2)], ground=round(g, 2), y=[round(lo, 2), round(hi, 2)]))
            za = zb
        gaps = [round(b['z'][0] - c['z'][1], 2) for c, b in zip(segs, segs[1:]) if b['z'][0] > c['z'][1]]
        assert not gaps, f'damage step {j}: gaps {gaps}'
        states.append(state(j, nxt, j + 1 if nxt else 0, zones))
        damage.append(dict(step=j, at_sec=t, max_hit=hit, z=[round(z0, 2), round(z1, 2)], newest_cloud_z=newest, segments=len(segs),
                           ground=[min(q['ground'] for q in segs), max(q['ground'] for q in segs)]))
    clone(CORE, f'{p}/controller', position=[cx, gy, stage_z[0]], **quiet, states=states)
    pack = dict(name='chemical-011e growing track anomaly', map_id=194, template_project=TEMPLATE,
                template_sha256=hashlib.sha256(level_02).hexdigest(), clones=clones)
    if a.also_map_ids:
        pack = dict(pack, also_map_ids=[int(v) for v in a.also_map_ids.split(',')])
        pack = {k: pack[k] for k in ('name', 'map_id', 'also_map_ids', 'template_project', 'template_sha256', 'clones')}
    a.output.write_text(json.dumps(pack, indent=2) + '\n')
    kinds = collections.Counter(native[c['source']]['game_object_type'] for c in clones)
    print(json.dumps(dict(parameters={k: v for k, v in vars(a).items() if k not in ('level_02', 'overlay', 'output')},
                          stages=[dict(stage=k + 1, at_sec=t, z=z) for k, (t, z) in enumerate(zip(stage_t, stage_z))][::5],
                          full_at_sec=stage_t[-1], damage=damage, objects=dict(kinds), clouds=a.stages * len(tracks),
                          wagons=report), indent=1))

if __name__ == '__main__':
    main()
