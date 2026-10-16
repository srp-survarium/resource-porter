# S.T.A.L.K.E.R. deathmatch maps in 0.26g

Five X-Ray multiplayer maps run in Survarium **0.26g0 build 2777** as team
deathmatch (native mode 4: tickets, two teams, no batteries or chargers).
0.26g has no free-for-all mode. Mode 2 and mode 3 are the battery modes.

| Map ID | Recipe | Source | Notes |
| ---: | --- | --- | --- |
| 197 | `pool-soc-dm` | SoC `testers_mp_pool` | wooden plank ramps and covers in place of the 7 ladders |
| 198 | `training-camp-cs-dm` | CS `mp_training_camp` + summer meadow and forest | foliage through 0.26's own systems |
| 199 | `rembasa-cs-dm` | CS `mp_rembasa` | 2 ladders to empty ledges, blocked |
| 200 | `sport-center-cs-dm` | CS `mp_sport_center` | wooden plank ramps out of the basin and up to the galleries |
| 201 | `firestation-cs-dm` | CS `mp_firestation` | one hidden ramp to the west roof; the tower and the small roofs are blocked |

IDs 152–193 and 194–196 are in use by the other imported maps. 197 is the
first free ID after them.

## Route

There is no direct X-Ray-to-0.26 converter. The established path goes
through v0.100b:

1. `build-soc-map` converts each X-Ray level into v0.100b. This is the
   [existing SoC path](soc-map-porting.md). The Training Camp meadow addon
   is built against it.
2. The bundles are composed into one v0.100b donor `resources.db`.
3. `build-survarium-maps-026`, the early-map importer
   ([survarium-early-to-026](survarium-early-to-026.md)), imports the
   donor's projects with the `0.10b` source contract. It uses
   `ports/soc-cs-dm-to-026g.json`.

`scripts/build-soc-dm-026.sh` runs the whole recipe and pins both retail
inputs. The donor fingerprint is pinned in the port spec, so a rebuild that
changes any v0.100b bundle must update `donor_sha256` and rerun
`tools/playable-map-pack/xray_material_templates.py`, described below.

### What the early importer gained

All of it is opt-in per map and off for the existing 0.10b–0.11e maps:

- **`donor_project`** imports a project that is not a retail
  `{level}{weather}` scene. `level` (here `level_03`) then only selects the
  native template project, the environment template and the game-mode
  options. The HUD minimap key is the template level's.
- **`mode: 4`** registers `{level}_{weather}_4.options`. The template is the
  `day_4` project, and the probe variant key is `day_4`. Each converted map
  gets its own mode file: `v010b_<project>_level_03_day_4.options`.
- **`deathmatch`** lists the source respawns, `[x, y, z, yaw]`, taken from
  the source `level.game` (team 0 with the deathmatch bit). It replaces
  every source gameplay group. Each point is registered once per team and
  cloned from the native mode-4 project's respawn point. It therefore carries
  0.26's protection (6 s) and spawn-check radii, and the source's ally and
  enemy zone links are dropped. A mode-4 map is validated for two teams and
  no battery objectives.
- **`height_ambient`** keeps 0.26's height-based ambient on. The early
  maps turn it off so that the encoded-probe decode runs. These maps have no
  environment probes, and their source skylight is exactly what
  height-based ambient replaced.
- **`walker_ramps`** and **`walker_boxes`** are invisible, walker-only box
  primitives (cmask 4, cgroup 2), in one generated model per map that is
  cloned from native `level/04/level_border`. A ramp is a slab whose top
  face runs from `from` to `to`, at no more than 40° (the sport-center
  planks are 30° or less). A box is a lid over a pit or a slab over the
  rungs of a ladder that is not replaced. With **`planks: true`**, native
  `props/cover/plywood_04` boards (0.28 × 2 m) are laid on the top face as
  render-only props. A ramp gets 3 boards side by side (and a lid as many as
  cover it), chained end to end, slightly turned, shifted and stacked where
  they overlap. The slab stays the walking surface, and bullets pass the
  boards.
- **Generated materials** (`material_instances/ported/...`) have no path
  domain, so the importer cannot choose a template for them by itself.
  `xray_material_templates.py` matches each one to the v0.100b family
  template it was cloned from: every non-texture value must be identical.
  It then declares that template's 0.26 namesake in the spec's
  `material_templates`.

## Training Camp foliage, optimised

The v0.100b meadow drew 50,000 grass tufts as 378 merged static meshes. It
drew 33,292 conifers as 2,172 merged 27- or 9-tree cluster meshes, plus
4,798 single trees. The merged meshes had no per-tree LOD, a wind weight
that swayed whole clusters, and roughly 31 million LOD0 triangles.
`build-soc-map-addon --native-foliage` keeps the same planted positions,
with the same placement code and the same counts, and emits them through
0.26's own mechanisms:

- **Grass and flowers** become two vegetation layers (`.veg` instances of
  the same grass models, at 1.35× scale with ±15% random scale) on the
  grass system. The grass system draws them in patches, switches their LODs
  at `r_grass_lod1_distance`/`r_grass_lod2_distance`, culls them past 100 m,
  and follows `r_decorations_quality`.
- **Near forest:** the 1,200 trees closest to the fence are single instances
  of the LOD'd `flora/elka_green_0x` models. These are native 0.10b models,
  imported with all three LODs and default switching (20/40/240 m, ×0.75 at
  `r_geometry_quality 0`), and they are culled past 240 m. They cast sun
  shadows like native trees.
- **Far forest:** 2,300 of the remaining trees, evenly thinned, are merged
  per 50 m tile into lowest-LOD meshes, using a no-shadow copy of the elka
  material. The rest are dropped.
- The victory items, the base containers and the server-project changes are
  left out (`--deathmatch`).

The budgets below are static counts over every visual, as if all were in
view (`map_budget.py`). With the shipped `r_instancing off`, each surface is
a draw call.

| Project | Instances | Draw calls | Triangles LOD0 / last LOD | Sun-shadow draws | Grass instances | Textures |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| v0.100b meadow (before) | 7,754 | 7,754+ | about 31 M / 5.3 M | all | 50,000 as 378 meshes | — |
| **training-camp-cs-dm** | 1,388 | 1,388 | 1.76 M / 0.82 M | 1,311 | 50,283 | 120 MiB |
| native level_07 day_4 (most flora) | 1,585 | 2,829 | 2.09 M / 0.84 M | 2,385 | 7,379 | 719 MiB |
| native level_06 day_4 | 870 | 1,960 | 1.91 M / 0.68 M | 1,764 | 69,626 | 505 MiB |
| native level_03 day_4 (most grass) | 845 | 1,402 | 1.82 M / 0.40 M | 1,344 | 90,879 | 613 MiB |
| pool-soc-dm | 165 | 227 | 0.07 M | 225 | 0 | 42 MiB |
| rembasa-cs-dm | 35 | 172 | 0.33 M | 172 | 0 | 137 MiB |
| sport-center-cs-dm | 44 | 157 | 0.23 M | 153 | 0 | 80 MiB |
| firestation-cs-dm | 33 | 263 | 0.28 M | 262 | 0 | 136 MiB |

Training Camp is at or under level_07 on every static count, and its grass
stays under level_03. The forest keeps 3,500 of the 33,292 planned trees
(10.5%): 1,200 near-belt trees at about 946/471/158 triangles per LOD, plus
2,300 far trees in 77 tiles (364,402 triangles in total). Grass casts sun
shadows like native `flora/grass_l03_alt`, and the near trees cast like
native elka. The far tiles cast none, and they do not sway (`wind_motion 0`;
a tile is one instance). The near trees have the same geometry as native
0.26 `elka_green_0x` (byte-identical) and keep its wind.

Low settings reduce the cost. `r_geometry_quality 0` scales every LOD
distance by 0.75, which also brings the near-tree cull (240 m) closer.
`r_shadow_quality` and `r_shadow_last_lod` set the shadow cascades and the
LOD that shadows use, and `r_decorations_quality` together with
`r_grass_lod1/2_distance` set the grass. These are 0.26's own option names (checked in the 0.26g executable). Their
effect was read from the engine source and has not been measured: the
static frame-count run (`capture_dm_gallery.py --quality low|medium|high
--frame-rate 0 --fps-seconds 6`) has not been done yet, because other
clients were running.

## Ladders

0.26g has no ladder gameplay. In v0.100b, `player::use_ladder` is empty (see
[soc-ladder-animation](soc-ladder-animation.md)), and none of the climb
work applies. Each ladder was traced with the X-Ray collision mesh. Its
`climable_object` box is centred on the spawn position with half extents
given by the shape matrix, and its landings were mapped. Reachability on a
0.25 m grid with 0.45 m steps confirmed every replacement.

| Map | Ladder | Leads | Replacement |
| --- | --- | --- | --- |
| pool | clmbl#6 (12.2, -37.1) | pool bottom −2.93 → deck 0.0 | plank ramps `pool_exit_west` and `pool_exit_east`, 30° |
| pool | clmbl#4/#5 (16.9, −47.0 / −27.2) | hatch pits −2.6 → deck, tunnel ring | plank covers over both hatches at deck level; the tunnel opens into the pool |
| pool | clmbl#0–#3 (corners) | 7.8 m shafts, tunnel → balcony 5.35 | no plank run fits a 1.6 × 2 m shaft, so these keep the replacement design: two plank ramps from the deck to the balcony (30°, 10.7 m) over the deck strips beside the pool, and plank covers over the shafts at balcony level (no 8 m drop) |
| training camp | clmbl#0 (21.8, 3.2) | 16.4 → 53 m² walkway at 20.1, no spawns | not replaced (a beam leaves no room); slab over the rungs |
| rembasa | clmbl#0 (−4.8, −33.8) | 15.5 m to a 14 m² ledge | slab over the rungs; no spawns there |
| rembasa | clmbl#1 (42.1, 18.9) | 3.2 m to a 15 m² ledge | slab over the rungs; no spawns there |
| sports center | clmbl#7/#6, #4/#5 | basin −1.24/−4.3 → rim 0.44 | plank ramps `basin_exit_west` and `basin_exit_east` (29°); the deep end walks up the basin slope |
| sports center | clmbl#2/#3 | gallery 4.16 → tribune 9.2 (upper level, 2,684 m², 4 spawns) | plank ramp `stand_ramp`, **39.4°**: the tribune joists every 2.3 m leave no room for a shallower run |
| sports center | clmbl#0/#1 | 7.68 pads → upper level | none; the upper level is reached by `stand_ramp` |
| fire station | clmbl#1 (and #0) | 6.66 → west roof 11.23 (678 m², 3 spawns) | hidden ramp `roof_west_ramp`, 34° |
| fire station | clmbl#2 | 7.5 → 14.6 roof pocket (97 m²) | blocked (rung slab); 1 spawn dropped |
| fire station | clmbl#3 | 5.4 → 11.2 | none needed: reachable on foot |
| fire station | clmbl#7–#10, #6 | the 17.5 m tower chain, a stairwell ledge and an unresolved ladder to the tower top (118 m²) | blocked (rung slabs on #7 and #9); 2 tower spawns dropped |
| fire station | clmbl#4/#5 | 23.2 → 30.7 high ledges (chain tops) | unreachable already; no spawns |
| fire station | — | 37 m² structure above the west roof | unreachable without the ladder; 1 spawn dropped |

## Checks

- `verify_shader_variants.py --templates`: 751 materials proven against
  their native templates, 0 missing shader variants, 0 new shadow casters,
  0 unprovable.
- `verify_material_textures.py`: 751 materials, 0 sampled or inert missing
  textures.
- Spawn audit (`tools/day-map-objectives/audit_respawns.py`): support,
  standing capsule and facing for every point of every map. After the
  adjustments recorded in the spec, 0 points are flagged (35, 14, 19, 32
  and 43 points). The audit's "closer to the other team centroid" note is
  expected: both teams share every point.
- Reachability (X-Ray cform, 0.25 m grid, 0.45 m step) with the ramps:
  the pool's deck, basin and balcony form one area of 1,545 m², and the
  sports center's ground, basin and upper level one area of 6,474 m².
- 314 porter tests and 8 script tests.

## Known issues

- Nothing has been walked in game yet. Only static, input-free checks
  were run. The ramp slopes (30°, 34° and 39.4°) and the step height need a
  walk test, the 39.4° sports-center ramp most of all.
- The respawn yaw is copied from X-Ray `level.game`. Whether players face
  the authored direction is not verified.
- The minimap, loading screen and mode name come from the native
  `level_03` (day, mode 4) template.
- Team deathmatch has 100 tickets and runs 15 minutes. Both teams spawn at
  every point (the source's deathmatch points), and protection is 0.26's
  6 s.
- Ramp collision is invisible and walker-only, and the planks are
  render-only, so bullets pass through ramps, planks and covers. The plank
  covers over the pool's hatches and shafts can be walked over.
- Training Camp is rebuilt from the installed Clear Sky. It is no longer
  the carried-forward release map, so it now has the foliage-wind and
  normal-map fixes, and it may look slightly different from the v0.100b
  package. The forest is a tenth as dense, and its far tiles show their
  lowest LOD to a player who walks out of the camp. The planted trees have
  no collision, as in v0.100b.

## Build and play

```sh
TARGET010=/path/to/retail-0100b/resources.db TARGET026=/path/to/retail-026g/resources.db \
OUT=/tmp/soc-dm VP=target/release/vostok-port nix develop -c bash scripts/build-soc-dm-026.sh
python3 tools/playable-map-pack/build_day_presets.py ... \
  --maps pool-soc-dm,training-camp-cs-dm,rembasa-cs-dm,sport-center-cs-dm,firestation-cs-dm --output PRESETS
python3 tools/playable-map-pack/prepare_dm_package.py --base-package THREE_DAY_PACKAGE \
  --presets PRESETS --bundle /tmp/soc-dm/bundles/soc-cs-dm-026g --output PACKAGE   # PACKAGE holds resources.db
PACKAGE/play-maps start pool-soc-dm --port 2297      # one client, UI on, runtime /tmp/soc-dm-maps-runtime
PACKAGE/play-maps stop
```

`--quality low|medium|high` applies a whole graphics preset over the
recipe's `user.cfg`, and `--frame-rate 0` lifts the 60 fps cap.
`capture_dm_gallery.py` takes the static gallery on its own Xvfb display.
