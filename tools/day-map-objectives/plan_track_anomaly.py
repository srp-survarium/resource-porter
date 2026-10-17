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
With --damage-cap H the damage is instead capped near the ground. The track,
from --damage-from-z to --end-z (the playable end), is cut into segments of
--spacing (their ends fall on every step's end), each one collision_geometry
whose meshes are one box per lane (--lanes, x edges across the tracks); each
box runs from --damage-depth below the lowest to H above the highest track-bed
ground under it (--bed-models: terrain, moss, rails; sampled every --grid m),
so a standing player's face anywhere on the bed is inside it and players on
wagon roofs, bridges and walkways are not. Neighbouring boxes overlap by
--overlap (along) and --lane-overlap (across). A damage zone hits a body part
in its filter inside any of its geometries (0.10b damage_zone_core: a contact
test against the character's bone collision; the fruit cloud filters "face"),
once per interval however many boxes hold it. Step j's zone holds the segments
up to --lag behind the newest cloud started by then; the last step reaches
--end-z. The geometries are shared by the steps; only one step is active.

Timers count from the controllers' first tick (anomaly activation); the
calibration run tells whether that is map load or match start.
Placement is checked: every cloud must have a rail of the 194 railroad
collision within 1 m across and ground under it; clouds over a wagon are
listed. Prints the object counts.
"""
import argparse, collections, hashlib, json, math, re, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_respawns import World, config
from fast_world import FastWorld
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
    ap.add_argument('--damage-cap', type=float, default=None, help='capped damage boxes up to this height above the local track bed')
    ap.add_argument('--damage-depth', type=float, default=0.3, help='how far below the local track bed those boxes reach')
    ap.add_argument('--overlap', type=float, default=0.25, help='each segment box reaches this far into its neighbours along the track')
    ap.add_argument('--lanes', default=None, help='x edges of the box lanes across the tracks (default: one lane, the tracks +- --zone-margin)')
    ap.add_argument('--lane-overlap', type=float, default=0.1, help='each lane box reaches this far into its neighbours across')
    ap.add_argument('--bed-models', default=r'/(terrain\d*|moss|railroad_0[12])\.model/', help='regex of the movement-collision models that make the track bed')
    ap.add_argument('--grid', type=float, default=0.25, help='track-bed sampling step under each box')
    ap.add_argument('--damage-from-z', type=float, default=None, help='capped damage starts here (default: first cloud - spacing)')
    ap.add_argument('--end-z', type=float, default=None, help='the playable end of the track: the last capped step reaches it')
    ap.add_argument('--segments-out', type=Path, default=None, help='write the capped segments, their boxes and track-bed heights (JSON)')
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
    segments, z1_of = [], {}
    if a.damage_cap is not None:
        assert a.end_z is not None, '--damage-cap needs --end-z'
        bed_re = re.compile(a.bed_models)
        bed = FastWorld(a.overlay, parts=[q for q in FastWorld(a.overlay, project).parts if bed_re.search('/' + q[0])])
        def bed_ground(x, z):
            f = bed.floor(x, 2.4, z, below=1.5, above=0.7)
            return f[0] if f else None
        zf = a.damage_from_z if a.damage_from_z is not None else stage_z[0] - a.spacing
        for j, (t, _) in enumerate(steps, 1):
            newest = max((z for z, s in zip(stage_z, stage_t) if s <= t), default=None)
            assert newest is not None, f'damage step at {t} s precedes the first fog stage'
            z1_of[j] = a.end_z if j == len(steps) else newest - a.lag - a.overlap  # its last box, overlap included, ends --lag behind
            assert zf < z1_of[j] <= a.end_z, f'damage step {j} ends at {z1_of[j]}'
        assert stage_z[-1] - a.lag < a.end_z <= stage_z[-1] + a.spacing, f'the last cloud (z {stage_z[-1]}) does not reach --end-z {a.end_z}'
        assert max(s for s in stage_t if s <= steps[-1][0]) == stage_t[-1], 'the last step precedes the last cloud'
        phase = z1_of[1] % a.spacing
        cuts = sorted({zf, a.end_z, *z1_of.values(), *(phase + a.spacing * n for n in range(int(a.end_z / a.spacing) + 2) if zf < phase + a.spacing * n < a.end_z)})
        ends = set(z1_of.values()) | {zf, a.end_z}
        # Every step end is a cut; the --spacing grid through them fills in, a grid cut nearer than 2 m to a kept one dropped.
        merged_cuts = sorted(ends)
        for c in cuts:
            if all(abs(c - k) >= 2.0 for k in merged_cuts): merged_cuts.append(c); merged_cuts.sort()
        edges = [float(v) for v in a.lanes.split(',')] if a.lanes else [cx - half_x, cx + half_x]
        assert edges == sorted(edges) and len(edges) >= 2
        for k, (za, zb) in enumerate(zip(merged_cuts, merged_cuts[1:]), 1):
            s0, s1 = za - a.overlap if za > zf else za, zb + a.overlap if zb < a.end_z else zb
            boxes = []
            for l, (xa, xb) in enumerate(zip(edges, edges[1:])):
                x0, x1 = xa - (a.lane_overlap if l else 0), xb + (a.lane_overlap if l < len(edges) - 2 else 0)
                hs, n = [], 0
                for i in range(int(round((x1 - x0) / a.grid)) + 1):
                    for m in range(int(round((s1 - s0) / a.grid)) + 1):
                        n += 1
                        h = bed_ground(min(x0 + i * a.grid, x1), min(s0 + m * a.grid, s1))
                        if h is not None: hs.append(h)
                assert len(hs) >= 0.9 * n, f'segment {k} lane {l + 1}: track bed under only {len(hs)} of {n} samples'
                lo, hi = min(hs) - a.damage_depth, max(hs) + a.damage_cap
                boxes.append(dict(x=[round(x0, 3), round(x1, 3)], y=[round(lo, 3), round(hi, 3)], bed=[round(min(hs), 3), round(max(hs), 3)]))
            gy_k = round(min(b['y'][0] for b in boxes), 3)
            cz = round((s0 + s1) / 2, 3)
            clone(GEOMETRY, f'{p}/segment_{k:02d}', position=[cx, gy_k, cz],
                  meshes=[dict(type=1, position=[round((b['x'][0] + b['x'][1]) / 2 - cx, 3), round((b['y'][0] + b['y'][1]) / 2 - gy_k, 3), 0],
                               rotation=[0, 0, 0], scale=[round((b['x'][1] - b['x'][0]) / 2, 3), round((b['y'][1] - b['y'][0]) / 2, 3), round((s1 - s0) / 2, 3)])
                          for b in boxes])
            segments.append(dict(segment=k, z=[round(za, 3), round(zb, 3)], box_z=[round(s0, 3), round(s1, 3)], boxes=boxes))
    for j, (t, hit) in enumerate(steps, 1):
        newest = max((z for z, s in zip(stage_z, stage_t) if s <= t), default=None)
        assert newest is not None, f'damage step at {t} s precedes the first fog stage'
        z0, z1 = stage_z[0] - a.spacing, newest - a.lag
        assert a.damage_cap is not None or z1 > z0, f'damage step {j} would be empty'
        nxt = steps[j][0] - t if j < len(steps) else 0
        if a.damage_cap is None:
            clone(GEOMETRY, f'{p}/zone_{j}_geometry', position=[cx, round(gy + 2.0, 3), round((z0 + z1) / 2, 3)],
                  meshes=[dict(type=1, position=[0, 0, 0], rotation=[0, 0, 0], scale=[round(half_x, 3), 2.5, round((z1 - z0) / 2, 3)])])
            clone(DAMAGE, f'{p}/zone_{j}', position=[cx, gy, round((z0 + z1) / 2, 3)], max_hit=hit,
                  collision_geometries=[dict(name=f'{p}/zone_{j}_geometry')])
            states.append(state(j, nxt, j + 1 if nxt else 0, [f'{p}/zone_{j}']))
            damage.append(dict(step=j, at_sec=t, max_hit=hit, z=[round(z0, 2), round(z1, 2)], newest_cloud_z=newest))
            continue
        zones = [f'{p}/zone_{j}']
        clone(DAMAGE, zones[0], position=[cx, round(gy, 3), round((zf + z1_of[j]) / 2, 3)], max_hit=hit,
              collision_geometries=[dict(name=f'{p}/segment_{k:02d}') for k, sg in enumerate(segments, 1) if sg['z'][1] <= z1_of[j] + 1e-6])
        states.append(state(j, nxt, j + 1 if nxt else 0, zones))
        held = [sg for sg in segments if sg['z'][1] <= z1_of[j] + 1e-6]
        damage.append(dict(step=j, at_sec=t, max_hit=hit, z=[held[0]['z'][0], held[-1]['z'][1]], newest_cloud_z=newest, segments=len(held)))
    clone(CORE, f'{p}/controller', position=[cx, gy, stage_z[0]], **quiet, states=states)
    pack = dict(name='chemical-011e growing track anomaly', map_id=194, template_project=TEMPLATE,
                template_sha256=hashlib.sha256(level_02).hexdigest(), clones=clones)
    if a.also_map_ids:
        pack = dict(pack, also_map_ids=[int(v) for v in a.also_map_ids.split(',')])
        pack = {k: pack[k] for k in ('name', 'map_id', 'also_map_ids', 'template_project', 'template_sha256', 'clones')}
    a.output.write_text(json.dumps(pack, indent=2) + '\n')
    kinds = collections.Counter(native[c['source']]['game_object_type'] for c in clones)
    print(json.dumps(dict(parameters={k: v for k, v in vars(a).items() if k not in ('level_02', 'overlay', 'output', 'segments_out')},
                          stages=[dict(stage=k + 1, at_sec=t, z=z) for k, (t, z) in enumerate(zip(stage_t, stage_z))][::5],
                          full_at_sec=stage_t[-1], damage=damage, objects=dict(kinds), clouds=a.stages * len(tracks),
                          wagons=report), indent=1))
    if segments and a.segments_out:
        a.segments_out.write_text(json.dumps(dict(segments=segments, stages=[dict(stage=k + 1, at_sec=t, z=z) for k, (t, z) in enumerate(zip(stage_t, stage_z))],
                                                  steps=[dict(step=j, at_sec=t, max_hit=h, z1=z1_of[j]) for j, (t, h) in enumerate(steps, 1)]), indent=1) + '\n')

if __name__ == '__main__':
    main()
