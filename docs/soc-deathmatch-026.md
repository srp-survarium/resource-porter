# S.T.A.L.K.E.R. deathmatch maps in 0.26g

Five X-Ray multiplayer maps run in Survarium **0.26g0 build 2777** as team
deathmatch (native mode 4: tickets, two teams, no batteries or chargers).
0.26g has no free-for-all mode. Mode 2 and mode 3 are the battery modes.

| Map ID | Recipe | Source | Notes |
| ---: | --- | --- | --- |
| 197 | `pool-soc-dm` | SoC `testers_mp_pool` | wooden plank ramps and covers in place of the 7 ladders |
| 198 | `training-camp-cs-dm` | CS `mp_training_camp` + summer meadow and forest | foliage through 0.26's own systems |
| 199 | `rembasa-cs-dm` | CS `mp_rembasa` | 2 ladders to empty ledges, blocked |
| 200 | `sport-center-cs-dm` | CS `mp_sport_center` | wooden plank ramps out of the basin, up to the galleries and over the steep steel stairs; crates and planks up through the third-floor hatches |
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
  options. The HUD minimap key is the template level's unless the map has
  a `hud_key` (below).
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
  maps turn it off so that the encoded-probe decode runs. In 0.26 it
  multiplies probe light, so on its own it lights nothing: these maps need
  the `skylight_probe` below.
- **`skylight_probe`** adds the map-wide probe that carries the 0.10b
  skylight, plus interior box probes (see [Lighting](#lighting)).
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
- **`props`** places native 0.26g models (`model`, `position`, `yaw`) the
  way native projects do: a render visual plus instances of the model's own
  `collision` (cmask 16, cgroup 8) and `walker_collision` (cmask 4, cgroup
  2). The sports center stacks `props/box/woodbox` crates (0.42 m high) as
  steps up to a plank.
- **`cuts`** removes an axis-aligned box (`centre`, `half`) from the
  converted static geometry under a donor model directory (`models`): every
  render surface and collision mesh there is clipped against the box (split
  triangles keep their attributes and face materials; the surfaces'
  `export_properties` vertex and index counts follow). The converted maps'
  static models sit at the origin, so the box is in map coordinates. Each
  cut must open both render and collision. Edges are left open.
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

The original ladders are still drawn, and 0.26g cannot climb them. Its
character controller steps 0.6 m (`cc_step_height`), walks slopes up to
50 deg and stands on up to 40 deg (`cc_max_slope_angle_cos`,
`cc_idle_max_slope_angle_cos`; defaults read from the executable). Where a
ladder's foot and landing are clear, two planks (a 0.6 m walker ramp) now
lie over the ladder itself, planned from its `climable_object` box against
the X-Ray collision (the planner's reasoning is in each ramp's `reason`):

| Map | Ladder | Ramp | Slope | Replaces |
| --- | --- | --- | ---: | --- |
| fire station | clmbl#0 | `ladder0_planks`, 5.48 → 10.52 roof | 29.9° | new (the roof was reached by `roof_west_ramp`) |

Planks laid over the pool and basin ladders themselves looked wrong next to
the original ladders, so those keep the ramps beside them (`pool_exit_west`,
`basin_exit_west`, `basin_exit_east`).

The sports-center ladders clmbl#0/#1 rise from a stair landing (7.68) to the
third floor (11.61) through a 1.5 m hatch. Each is replaced by a route
without a ladder: the ladder is cut out (`ladder_*_removed`), the hatch is
widened over the whole landing (`hatch_*_opening`, x −20.96..−17.43 by
1.86 m, the slab 11.1..11.8 with its kerb), four columns of native crates
(1 to 4 `woodbox` high: tops 8.10, 8.52, 8.94, 9.36, 0.54 m treads) climb
north along the east wall where the ladder stood, and a two-plank ramp runs
west along the north wall from the top crate to the third floor
(`hatch_*_plank`, 9.36 → 11.63, 34.9°, 2.41 m headroom at the least).

Steep metal stairs. The X-Ray maps lay an invisible inclined plane
(`materials\fake`) over steep steel stairs; 0.26g walks 50° at most, and
those planes are 60°. Every inclined collision plane of 44–76° within 15 m
of the spawns was traced on all five maps: only the sports center's four
flights are stairs (the others are roofs, rubble, rock and terrain). Each
gets a three-plank ramp (0.9 m, the stairs are 1.13 m wide) with its top at
the landing and its foot run out along the floor:

| Ramp | Stair (plane) | Rise | Slope |
| --- | --- | --- | ---: |
| `steel_stair_south` | x −82.68..−80.56, z −59.3, 61.3° | 0 → 4.16 | 30° |
| `steel_stair_north` | x −82.68..−80.56, z −13.8, 61.3° | 0 → 4.16 | 30° |
| `steel_stair_west_lower` | x −81.72..−84.23, z −15.16, 61.0° | 0 → 4.67 | 36° (its foot is under the 4.16 gallery) |
| `steel_stair_west_upper` | x −87.65..−88.87, z −15.16, 59.7° | 4.67 → 6.99 | 30° |

Every other ladder keeps the treatment below: its top is covered, blocked
or reached elsewhere, or there is no room for a ramp (the pool's 7.8 m
shafts and hatches, the sports-center gallery ladders under low joists, the
fire-station tower chain, rembasa and Training Camp).
`check_ladder_ramps.py` walks every ramp's foot, middle and top and every
crate stack's top from the respawns (0.45 m between 0.25 m cells; with
`--target-db` the crates' native collision counts). Each hatch route is
also walked confined to 3 m around its plank, from the lowest crate: it
must reach the landing below and the third floor past the plank (and does
not without the plank). The 0.54 m crate treads alias on the 0.25 m grid,
so with props a floor also counts when it is reachable on the grid shifted
by half a cell. No static collision may remain inside a cut. All pass.


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
| sports center | clmbl#0/#1 | stair landing 7.68 → third floor 11.61 | ladder cut out, hatch widened, crate stack and plank `hatch_south_plank` / `hatch_north_plank`, 34.9° |
| fire station | clmbl#1 (and #0) | 6.66 → west roof 11.23 (678 m², 3 spawns) | hidden ramp `roof_west_ramp`, 34° |
| fire station | clmbl#2 | 7.5 → 14.6 roof pocket (97 m²) | blocked (rung slab); 1 spawn dropped |
| fire station | clmbl#3 | 5.4 → 11.2 | none needed: reachable on foot |
| fire station | clmbl#7–#10, #6 | the 17.5 m tower chain, a stairwell ledge and an unresolved ladder to the tower top (118 m²) | blocked (rung slabs on #7 and #9); 2 tower spawns dropped |
| fire station | clmbl#4/#5 | 23.2 → 30.7 high ledges (chain tops) | unreachable already; no spawns |
| fire station | — | 37 m² structure above the west roof | unreachable without the ladder; 1 spawn dropped |

## Minimaps and names

Each map has its own HUD minimap, rendered from its converted geometry, and
its own name. The spec gives every map a **`hud_key`** (its donor project,
e.g. `cs_mp_sport_center`) and a **`map_name`** (e.g. "Sports Center").

**How 0.26 places the minimap.** `flash_movies/hud.swf`,
`survarium.controls.Minimap.setProps({map: project_name})` looks up
`_minimapProps[key] = {size, posX, posY}` and the bitmap `_levels[key]`; an
unknown key falls back to `level_09`. The bitmap (W × H px) covers 2·size
metres around (posX, posY), and the engine passes the player as (x, −z):

    u = (x − posX + size) · W / (2·size)      v = (posY − z + size) · H / (2·size)

so +x is right and +z is up. This was checked by drawing native project
objects over the native bitmaps: on level_03 (size 100, posX 14, posY
−42.196, 512 px) and level_06 (size 100, posX 30, posY 109) the indoor
furniture lands inside the drawn buildings, the trees outside and the
respawns on the bases; with +z down nothing lines up. The tab map draws the
bitmap 1:1 into a 460 px square framed by `getColorBoundsRect` on RGB, so
outside the play area every channel is 0.

**Style** (measured on the native level_03 bitmap, a premultiplied JPEG +
zlib alpha `DefineBitsJPEG3`): open ground is a desaturated top-down photo at
alpha 87, buildings and blocks are flat (27, 27, 27) at alpha 184 with a
1 px light outline, and the play-area border is a light (110, 110, 107)
line.

**Renderer** (`tools/playable-map-pack/render_minimap.py`). The play area is
what a walker reaches from the respawns over the converted collision,
walker ramps included (0.25 m cells, 0.5 m steps on ≤45° faces, 1.7 m
headroom), with its holes filled. Training Camp has no border (its meadow
runs past any margin), so there the area is cut 45 m around the respawns.
Each cell shows the highest render surface at most 1.2 m above its lowest
reachable floor (interiors show their floor, not the roof; terrain uses the
full colour map), covered floors are darker, and anything unreachable inside
the area is dark. The scale is at most 3.5 px/m and the play area fits
440 px:

| Map | hud_key | size | posX | posY | px/m |
| --- | --- | ---: | ---: | ---: | ---: |
| pool-soc-dm | `soc_mp_pool` | 73.143 | 19.693 | −37.063 | 3.5 |
| training-camp-cs-dm | `cs_training_camp` | 83.055 | 27.712 | 5.788 | 3.08 |
| rembasa-cs-dm | `cs_mp_rembasa` | 179.782 | −2.549 | −103.581 | 1.42 |
| sport-center-cs-dm | `cs_mp_sport_center` | 73.143 | −58.632 | −36.539 | 3.5 |
| firestation-cs-dm | `cs_mp_firestation` | 73.143 | −8.37 | 14.843 | 3.5 |

Rembasa is 224 × 309 m, so its minimap is zoomed out (natives: 2.1–2.8 px/m).

**Wiring.** `build-survarium-maps-026` writes the `hud_key` as the
project's `project_name`, sets the mode's `ui.map_name` to
`st_<hud_key>_name`, and adds that string to all six
`localization/*/localization` tables (an id that exists is an error).
`tools/playable-map-pack/hud_minimaps.py` then renders the maps from the
bundle and patches the retail hud.swf (refused unless it is the build 2777
original, `710c9ca6…`) with `hud_minimaps/PatchHudMinimaps.java` (JPEXS
FFDec): one `DefineBitsJPEG3` + `BitmapData` class per map,
`SrpMinimaps.register(props, levels)` compiled into the Minimap ABC and
called at the end of the Minimap constructor. Every native method body and
every other SWF tag stays byte-identical (checked on save). The patched
hud.swf goes into the bundle and its `files`, so `verify` checks it.

**Loading screens.** The engine reads `mode.ui.loading_screen_id` and calls
`root.set_loading_level(id)` with it unchanged (0.26g 0x2204AE..0x2204EA, an
unsigned argument, no range check). `flash_movies/lobby_menu.swf` (build
2777 `ab6f22d8…`; the `lobby_menu.swf.orig` beside it is never loaded) shows
`LobbyMenu._load_images[id]`: 26 native images, ids 0–25 (level_03 day is
2). They are plain `DefineBits` JPEGs, 1920 × 798 for levels 1–6 and
1920 × 680 for 7 and 8, wide-angle in-game shots with no text, logo or
vignette baked in. `LobbyGameLoad` scales the image to the stage width,
centres it in the band between the `LoadingTop` (1920 × 411, the SURVARIUM
logo) and `LoadingBottom` (1920 × 595) frames, whose alpha edges darken it,
and draws the map name (`ui.map_name`), mode name and description over it.
On the native day images luminance p5/p50/p95 is about 0.09/0.30/0.61
(highlights under 0.8) and mean saturation 0.27.

Every map with a **`loading_image`** gets id 26 + k (k counting those maps
in spec order: 197 → 26 … 201 → 30); `build-survarium-maps-026` writes it
as the mode's `ui.loading_screen_id`. `tools/playable-map-pack/lobby_loading.py`
centre-crops each image to 1920:798, resizes it to 1920 × 798, grades it to
those statistics (unless `--no-grade`) and patches lobby_menu.swf with
`hud_minimaps/PatchLobbyLoading.java`: a `DefineBitsJPEG3` + `BitmapData`
class per image and `SrpLoadingImages.register(this._load_images)` at the end
of the LobbyMenu constructor, which appends them only if the array holds the
26 native images. Every other method body and tag stays byte-identical; the
patched SWF is listed in the bundle files. The images in
`assets/soc-dm-loading/` are placeholders.

## v0.10b against 0.26, by subsystem

The v0.100b builds of these maps are the reference. The table lists what
reached 0.26 before this round and what changed.

| Subsystem | v0.10b | 0.26 before | 0.26 now |
| --- | --- | --- | --- |
| Ambient | skylight stage: ambient cube x skylight colour x SSAO x 2.5 on every pixel | none: height ambient multiplies probe light and there were no probes, so everything unlit was black | `skylight_probe`: a native global sphere probe holding the source cube; native height-ambient tint; interior box probes |
| Sun, sky, fog | template sun, sky, post-process | translated (`transfer_environment`) | unchanged |
| Lamps | none (template lamps dropped) | none | none; the "lamp cone" in the user's screenshot is sunlight through a window with foliage shadows, shown alone because ambient was zero |
| Textures | DDS pitch field = payload, accepted | the same header: 0.26's streamer reads the packed mip tail one byte early, so distant mips of streamed textures render as colour noise (pool `wood_plank7/8`) | native header (0xA100F, payload + colour-space byte) at the source, and repaired by the importer |
| Geometry | parts up to 65,536 vertices | rembasa's 65,238/65,384-vertex terrain parts overflow 0.26's 1 MiB loader stack (crash on load) | parts split at 35,000, refused above 35,003 |
| Tangent frames | X-Ray bake, some parallel | the same | rebuilt (`strict_finite`) |
| Materials | v0.100b family templates | their 0.26 namesakes | unchanged (proven, 0 missing variants) |
| Water, terrain, foliage | water_green, terrain shader, meadow addon | 0.26 namesakes | unchanged |
| Collision, ladders, spawns | X-Ray cform, ladders | cform; ramps and lids | unchanged; audit clean |
| Minimap, name, loading screen | level_03's | level_03's | each map's own (below) |

## Lighting

0.10b lights every pixel with its skylight stage first (`skylight.ps`): the
six axis colours of the post-process (`environment_skycolor_*`) weighted by
the squared world normal, times `pow(skylight_color, 2.2) *
skylight_multiplier`, the height ramp and SSAO x 2.5. 0.26 has no such
stage. Its ambient is environment-probe light, which height-based ambient
then multiplies. These scenes have no probes, so their 0.26 ambient was
zero: the maps were black wherever the sun did not reach.

`skylight_probe` restores it in 0.26's own contract:

- One native global sphere probe, cloned from the template's largest sphere
  probe, centred on the map's render bounds with a radius enclosing them
  (42 m pool to 577 m sports center) and a 1 + 0.02 r falloff width.
- Its cube (64 px, DXT5) and diffuse cube (16 px) hold the source ambient
  cube at every texel direction, times `gain`. 0.26 decodes a probe texel
  as pow(rgb / a, 2.2); a constant non-zero alpha keeps that finite.
- `native_tint` takes the height-ambient colour from the native template
  (warm white) instead of the source skylight. All five maps inherited the
  same cold 0.10b template skylight (0.816, 0.897, 1.0), which reads blue
  under 0.26's grading: blue/red 1.28 against 0.65–0.78 on native level_03.
  With the native tint the sports-center halls measure 0.95–1.04.
- `gain` 0.8 puts the interiors in the native range (static captures:
  native level_03 interiors mean luminance 0.17–0.24, sports-center halls
  0.22–0.29 before the interior probes). Eye adaptation absorbs most of a
  global change; contrast comes from the interior probes.
- Specular 0.3, as on the native global probe.

## Interior probes

0.10b lit the converted rooms like open ground: its skylight is one ambient
cube, darkened only by SSAO. The skylight probe that ports it to 0.26 is
equally flat indoors. Native 0.26 maps darken interiors with box probes, so
`skylight_probe.interiors` lists box probes per map. Each uses the skylight
cube at `diffuse` times the map-wide strength (specular and ambient alike).

- **Box contract:** the box is the unit cube (−1..1) under the probe
  transform, so `scale` holds half extents. The native level_03 `indoor` box
  is a 26 × 12.6 × 43 m hall at scale 13.16, 6.32, 21.71. The inner box is
  the half extents minus `probe_side_width`, and the probe fades smoothly
  between the two. The width is 1 m here, so the transition sits at walls
  and doorways. `radius` only orders the draws (largest first). It is set to
  0.75 × the largest half extent, as 0.10b derives it, so every box draws
  over the map-wide sphere.
- **Proposal:** `tools/playable-map-pack/interior_probes.py` proposes the
  boxes.
  - Sky visibility is measured on every reachable 1 m floor cell: 48 upper
    hemisphere rays through a 0.5 m voxelization of the collision, glass
    faces excluded.
  - Cells with visibility under 0.2 are grouped per storey into boxes that
    are at least 70% covered, at most 12 per map.
  - Boxes run floor to ceiling plus 0.5 m past the covered cells.
  - `diffuse` is 0.45 for a closed room, up to 0.75 as visibility rises, plus
    0.1 for halls over 600 m².
  - Rerun it with `--write` after the geometry changes.

## Rembasa crashed on load

The client log ends `STACK_OVERFLOW` right after rembasa's terrain textures
stream in. The minidump faults in `_alloca_probe` (0x429f47) with ESI
0x13f420: 0.26's static surface loader copies a surface's vertices
through a 20-byte-per-vertex `_alloca` (0x16ebd8..0x16ebe4) on a 1 MiB
thread stack, and rembasa's terrain parts had 65,238 and 65,384 vertices.
Native 0.26 surfaces stop at 35,003 vertices (the largest of 7,854 native
static surfaces). `build-soc-map` takes `max_surface_vertices` from the
manifest (default one u16 index space); the five deathmatch manifests set
35,000, which changes only rembasa's output. The 0.26 importer refuses any
early static surface above 35,003 vertices.

## Texture headers

Every retail v0.100b and 0.26 DDS has flags 0xA100F and
`dwPitchOrLinearSize` = every byte after the header: the mip payload plus
the trailing colour-space byte. `build-soc-map` wrote 0xA1007 and the
payload alone. 0.26's cooker locates the first kept mip from that field
(`texture_cook_wrapper::on_dds_header_ready`, cooker convention total + 1),
so it read the packed tail one byte early. On mp_pool, `wood_plank7` and
`wood_plank8` (1024 px) showed per-block rainbow noise in the albedo
buffer; the same bytes under another path stayed broken, and the same bytes
with the native header rendered correctly. `canonicalize_dds` now writes the
native header (SoC, Clear Sky, F.E.A.R., the meadow addon), the generated
probe cubes count the colour-space byte, and the importer repairs the
off-by-one header of any early DXT texture and refuses any other mismatch.

## Checks

On the final database:

- `verify_shader_variants.py --templates`: 751 materials proven against
  their native templates, 0 missing shader variants, 0 new shadow casters,
  0 unprovable.
- `verify_material_textures.py`: 751 materials, 0 sampled or inert missing
  textures.
- `verify_finite_bundle.py`: 0 defects on every map.
- `verify_probe_textures.py`: every scene has its probes (10–13), and every
  cube and `_diffuse` resolves.
- Spawn audit (`tools/day-map-objectives/audit_respawns.py`, on the
  gameplay-objects branch): support, standing capsule and facing for every
  point of every map, 0 problems (70, 28, 38, 64 and 86 registrations). The
  audit's "closer to the other team centroid" note is expected: both teams
  share every point.
- `vostok-port verify` and `play-maps verify` pass.
- Static, input-free captures of all five maps load and render; rembasa no
  longer crashes. A player-camera capture with the HUD shows the sports
  center's own minimap with the marker on the map.
- Reachability (X-Ray cform, 0.25 m grid, 0.45 m step) with the ramps:
  the pool's deck, basin and balcony form one area of 1,545 m², and the
  sports center's ground, basin and upper level one area of 6,474 m².
- 324 porter tests and 11 script tests.

## Non-finite values

**Probes first.** 0.26g renders NaN where a probe it draws has no cube. On
native Cologne (`level_08/prerain_3`) two probes name cubes the retail
database lacks (`probe_prerain_3_10629444731943501094` and
`_10746810447428071133`); there the game shows a black screen when firing
and flickering squares that spread through TAA, the defect the user saw
("similar to keln"). The deathmatch maps had no probe at all, so forward
passes had no cube either. With the skylight probe every scene has one, and
`tools/playable-map-pack/verify_probe_textures.py` checks that every scene
has an enabled probe and that every probe's cube for the scene's variant
(and its `_diffuse`) exists; it reproduces the two Cologne findings. The
importer refuses a converted scene that ends without a probe. Static
buffer captures (sports center, before and after; native Cologne day) show
no non-finite texel beyond the native `$user$mie_scattering` baseline; the
firing case needs input, so it was not captured.

The spec sets `strict_finite`. The importer then rebuilds static tangent
frames that 0.26 cannot use, and refuses non-finite or NaN-prone output:

- **Tangent frames.** 0.26 interpolates the packed normal, tangent and
  binormal unnormalised and normalises T·x + B·y + N·z per pixel
  (`gbuffer_pass.ps`). The X-Ray tangent bake left 2,596 vertices in 78
  surfaces with a tangent or binormal parallel to the normal or to each
  other (8.1° or closer): pool 564, Training Camp 347, Rembasa 365, Sports
  center 298, Fire station 1,022. They are rebuilt orthonormal around the
  kept normal, with the vertex's own handedness or the model's majority.
  Nothing else in those models changes, and no other file of the bundle
  changes.
- **Refused:** non-finite positions, Inf/NaN half UVs, unusable frames,
  non-finite floats in any emitted config, and environment values a 0.26
  shader turns into NaN. These are an empty height-ambient band or power
  ≤ 0 (`apply_ssao_and_hba.ps` computes exp(log(t)·power)),
  `mie_fog_sun_color_falloff` ≤ 0, and fog far distances ≤ 0.
- `tools/playable-map-pack/verify_finite_bundle.py --bundle B` counts the
  same per map on a built bundle. Before the fix it reports 2,596 frames,
  and after it 0. There are no non-finite positions, UVs or constants, and
  no environment hazard, in either build.

The shader audit (DXBC of the shipped 0.26g permutations, disassembled with
vkd3d-compiler) bounds where a NaN can come from:

- The g-buffer normal is R16G16_FLOAT and can hold a NaN, but every reader
  (composition, probes, sun mask, SSAO) clamps the decoded normal with
  max(-1)/min(1), and D3D min/max (DXVK NMin/NMax) return the non-NaN
  operand. Albedo and parameters are RGBA8 UNORM.
- `composition.ps` reads the light accumulators through max(x, 0), which
  turns NaN into 0 (black, not spreading).
- The probe stage blends each probe over the accumulator by its falloff
  alpha. There is no division by a probe weight sum, so a map without
  probes is black, not NaN.
- TAA (`temporal_antialiasing.ps`) lerps the bilinearly reprojected history
  with no clamp. A NaN that reaches the HDR frame therefore persists and
  grows by a pixel per frame of camera motion. The tonemap's LUT lookup
  shows it as black, and Inf as white.

Therefore the NaN must enter after composition, in a forward or post pass.
The forward water (`forward_simple_water`, pool and Sports center only)
decodes its reflection cube as rgb²/a² without a guard. The shipped cube is
DXT1 with no transparent texel, so a = 1 there. Which pass produces the
NaN still needs a runtime stage capture: a mode-1 pose with buffers, then
`tools/render-compare/scan_nonfinite_buffers.py CAPTURE_DIR`. It lists NaN
and Inf counts per float target and stage. Native 0.26 already has one
finding: `$user$mie_scattering` (256×128) holds NaN in its last two
columns, which lie below the horizon. Its readers cap the lookup at 0.8, so
it is a baseline and not this defect.

## Known issues

- Nothing has been walked in game yet. Only static, input-free checks
  were run. The ramp slopes (30°, 34° and 39.4°) and the step height need a
  walk test, the 39.4° sports-center ramp most of all.
- The respawn yaw is copied from X-Ray `level.game`. Whether players face
  the authored direction is not verified.
- The loading screens are in-game shots (`assets/soc-dm-loading/`,
  poses in `shots.json`); they await the user's approval. A loading screen
  was not captured in game: the Flash overlay is not visible to the
  capture hook.
- Broken window glass on mp_pool shows row-striped alpha at a distance,
  like the parked "stripes on low mips" item of the day maps.
- The NaN reported while firing is attributed to the missing probe (as on
  native Cologne); firing needs input and was not reproduced.
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
OUT=/tmp/soc-dm VP=target/release/vostok-port FFDEC=/path/to/ffdec PYTHON=python3-with-numpy-pillow-scipy \
JAVA=/path/to/java JAVAC=/path/to/javac \
  nix develop -c bash scripts/build-soc-dm-026.sh
python3 tools/playable-map-pack/build_day_presets.py ... \
  --maps pool-soc-dm,training-camp-cs-dm,rembasa-cs-dm,sport-center-cs-dm,firestation-cs-dm --output PRESETS
python3 tools/playable-map-pack/prepare_dm_package.py --base-package THREE_DAY_PACKAGE \
  --presets PRESETS --bundle /tmp/soc-dm/bundles/soc-cs-dm-026g --output PACKAGE   # PACKAGE holds resources.db
PACKAGE/play-maps start pool-soc-dm --port 2297      # one client, UI on, runtime /tmp/soc-dm-maps-runtime
PACKAGE/play-maps stop
```

`--quality low|medium|high` applies a whole graphics preset over the
recipe's `user.cfg`, and `--frame-rate 0` lifts the 60 fps cap.
`capture_dm_gallery.py` takes the static gallery on its own Xvfb display
(`--window 1920x1080 --set r_motion_blur_quality=0` for loading-screen
shots, `--ui on` with a `"mode": 3` pose for the HUD).
