# Factory and Chemical Plant 0.11e day bundle

`ports/survarium-011e-day-to-026g.json` builds both maps in one additive bundle:
Chemical Plant **194** and Factory **195**, sharing `ported/v011e` resources.
The 0.11e build 884 archive is the geometry, lighting, material and texture donor.
No 0.10b weather recipe or blanket 0.11a visual overlay is applied.

```sh
python3 tools/chemical-plant/rebuild.py \
  --porter /path/to/vostok-port \
  --manifest ports/survarium-011e-day-to-026g.json \
  --donor-db /path/to/011e/resources.db \
  --server-donor-db /path/to/011a/resources.db \
  --target-db /path/to/original-026g/resources.db \
  --output /new/011e-day-candidate --bundle-only
```

## Explicit Factory gameplay fallback

The 0.11e archive contains no Factory server project. Its binary and Lua client
projects contain no battery spawners, and no alternate server project exists in
the archive. The manifest therefore explicitly pins the 0.11a build 816 server
project: payload SHA-256
`a8c6ca4f156b11dad06697d52d6768d0f4351a60545abf9db16dfa0ed98f7d11`,
from database SHA-256
`f59650ee69e834f7757461c54affd93b6fbb767ab841d70468e85019b059c4f6`.

The converter verifies the database and payload hashes and refuses this fallback
unless all **43 player respawns, two collectors and 47 interaction zones** exactly
match the primary 0.11e client. Those checks pass. Factory's **21 battery
candidates are authored 0.11a data**, not claimed as recovered 0.11e server data.
All candidate coordinates remain unchanged; nested battery origins receive the
native collision-centre offset. Factory's collision and render geometry still
come from 0.11e, not the fallback server's obsolete collision array.

## CPU result and remaining runtime checks

The combined bundle contains **8,585 files / 1,209,865,537 bytes**, 333 converted
materials and 162 shared native dependencies. Dependency closure is complete,
every output hash passed, and Chemical's 33 battery transforms and distinct
collector zones pass the existing audit. No donor shader is imported.

`audit_factory_011e_spawns.py` reproduces the Factory fallback/transform checks and
compares initial native battery boxes with the actual 0.11e collision resources.
No collision payload required by the converted scene is missing. Nineteen
candidates have exact triangle or box support within 0.2 metres of the body
centre. Candidates **3 and 19** are among barrels: cylinder contact is deliberately
not inferred from conservative bounding boxes, and their nearest proven ground
surface is about one metre below. They need native settling checks.

Initial shallow contact at candidates **14, 17 and 18** reaches approximately
2.6–4.5 cm against a table or sofa. Candidates **0, 2 and 8** have nearby
non-box primitives that this CPU audit does not fully resolve. No body centre is
inside an exact box or below an upward collision triangle. These are initial
support/overlap checks, not proof of final settling or pickup accessibility.
No position was relocated.

Factory source `antenna_tree.model/walker_collision` is absent, but its original
ordinary collision payload exists and the converter's documented fallback keeps
it. `soviet_comp_02.model/collision` is absent without a walker counterpart;
this explicitly listed source omission is recorded for both maps.

The original 0.11e sphere shaders and native CPU upload agree on radial falloff;
explicit donor widths are retained, including authored reversed intervals.
Full probe appearance and selection remain separate parity questions. See
`chemical-plant-011e.md` and
`investigation/chemical-011e-probe-falloff-20261014.md`.

Artifacts: `/tmp/011e-day-candidate/bundle`, `chemical-cpu-audit.json` and
`factory-cpu-audit.json` beside it. This supersedes the Chemical-only bundle.
No full database or shared runtime resource was changed. Existing regression
functions cover the pinned fallback rejection and large config serialization;
no additional test functions were introduced.

## Native finish and native textures (revision 18, `maps-v011e-2`)

`ports/survarium-011e-v011-release-to-026g.json` sets `native_finish` and
`native_resources`. 0.11e tuned its finish for its own renderer: constant
Fresnel 1.0 on `props/electric/electrical_cabinet_02`, gloss 0.8 / Fresnel 0.7
on `tiled/metal/metal_solanka` (gate, tower cabins), Fresnel 0.7 on
`fuse_boxes1`, `electric_mechanism`, `oxygen_cylinder`, Fresnel 1.0 on
`roof_ruberoid` and the remains. Under 0.26g's environment probes cabinets,
booths and lockers reflected like mirrors.

* `native_finish`: a donor material 0.26g ships under the same path keeps the
  converter's template and switches (an early donor's), and takes the native
  material's finish values and maps (`material_finish::adopt_native_finish_with`).
  Where the template reads a map the native material does not, min = max = the
  native constant; where it reads a constant the native material takes from a
  map, the constant is the native value at the map's mean (its smallest mip,
  `native_equivalence::texture_mean`). 323 of 333 materials have a namesake;
  their effective gloss and Fresnel now equal native, except where a native
  map's name is also a donor texture (one name is one texture; the donor's art
  stays) or retail lacks the map: 12 materials, none mirror-like.
* `native_resources`: a donor texture byte-identical to the native one, or
  with the same full-resolution level, colour-space byte and `.options`
  (`native_equivalence::texture_match`), is referenced, not imported: 190.
  The native finish maps resolve to the target's textures (two dependency
  scans).

Against `maps-v011e-1`, only finish values and maps of 310 materials and the
texture names of 10 `export_properties` differ; no shader switch, model,
project, gameplay fill, map id, name, loading picture or minimap changes.
672 texture files are no longer imported: the pack is 857 MB instead of
1,279 MB. `PACK_VERSION=2 tools/content/build_v011e_packs.sh OUT_DIR` builds it.

`maps-v011e-3` adds two reviewed changes over `-2`:

* `finish_exceptions` (`material_finish::FinishException`): the owner's choice
  of 2026-10-17 between the native finish and a ported one (gallery
  rev17-metal-fix, column v0.26-ported): `electrical_cabinet_01`,
  `electrical_cabinet_02` and `mech_cabinet_02` keep Fresnel 0.05 (min = max =
  constant) over the native 0 / 0.03, whose dark rooms hid them.
* the team 1 ammo crate on Chemical Plant (194/206/207) at (66.8, 3.879, 62.9),
  yaw 0.6727, pitch -0.0963, roll 0.0574 (`ports/gameplay-fill/chemical-011e-resupply.json`).

`SKHID_PATCHES` selects the directory of the released patches 12 and 14.

## Growing track fog to the end of the tracks, capped damage (revision 20)

The owner's requests of 2026-10-17 for Chemical Plant (194/206/207), made by
`plan_track_anomaly.py` (parameters in its log) into
`ports/gameplay-fill/chemical-011e-track-anomaly.json`:

* The fog grows to the end of the tracks: the central factory wall at
  z 131.9 (rails, dead end and buffer to z 131). 25 stages, z 10 ... 130,
  from 150 s every 15 s, so the old stages keep their times (z 20 at 180 s)
  and the fog is full at 510 s of the 900 s match.
* The clear patch between the moss field's rust fog (clouds to z 7) and the
  first track cloud (z 20) is closed by stages at z 10 and 15. Every
  corridor point (x 9.9 ... 21.6, z 0 ... 131.9) is within the 5 m
  rust_smoke radius of a cloud; neighbouring clouds are 5 m apart.
* The damage hits only near the track bed. 26 segments (about 5 m, ends on
  every step's end, 0.25 m overlap) of five lane boxes across the tracks
  (x 9.9 / 11.4 / 14.4 / 17.1 / 20.1 / 21.6, 0.1 m overlap). Each box runs
  from 0.3 m below the lowest to 2.2 m above the highest track bed under it
  (terrain, moss and rail collision, sampled every 0.25 m), so a prone,
  crouched or standing face anywhere on the bed is inside (the fruit cloud
  zone filters "face") and wagon roofs (7.2 m), the loading-station
  walkways and the old box's 7.58 m top are out. The segments are shared:
  step j's one zone holds segments 1 ... n_j, so overlapping boxes hit
  once per interval. The steps keep the times and the lag: 240 / 300 /
  360 / 420 / 480 s (0.06 / 0.08 / 0.10 / 0.12 / 0.12) end 4.5 m behind the
  newest cloud, and the last one (540 s, 0.12) reaches the wall, 30 s after
  the last cloud. The damage starts at z 3, inside the native moss zone
  (always on, 0.10), so there is no gap at the junction.
* No spawn, container or crate of 206 or 207 is near: the nearest, the
  team 1 container, is 39 m from the fog.

`tools/day-map-objectives/apply_track_fog_rev20.sh PACK_DIR` applies it to
a built maps-v011e pack (`track-fog-rev20.plan.json` through
`examples/edit-built-pack.rs`: the -4 objects checked by count and hash,
replaced, the result checked by hash, nothing else may change).
`verify_track_fog.py` and `render_track_fog.py` are the offline checks and
pictures.

## Surfaces under 0.26's lighting (revision 20)

With the probe fixes the lighting reads right, but some surfaces did not:
Chemical's team-0 hall floor and boilers, Radar's hangar floor.

* Vertex-blended materials on 0.26's height-and-mask blend (technique 1, tiled
  mask) kept 0.11e's `constant_vertex_blended_falloff` 1 where 0.26 has 10 or
  15: the second layer was smeared over whole floors in the mask's noise,
  mottled and whitish. A material taking the native finish now also takes the
  native blend tuning (falloff, strength, factor, tiled-mask scale, rotation and
  texture) where it blends by the native technique and mask: 34 materials of
  both maps (`floor_trash_dirt_vc`, `concrete_green_moh_vc`,
  `concrete_column_trash_vc`, `trash_concrete_vc`, `concrete_green_trash_dirt_vc`, ...).
* `reviewed_native_textures`: textures that decode as 0.26's own art, which
  0.26 re-made: eight normal maps 1.5 to 10 times flatter in 0.26
  (`metal_rust_02_nmap` 10x: the boilers' white blotches; `metal_column_06`,
  `stucco_ceiling`, `stucco_08`, `stucco_05`, `bricks_brick`, `table_stool`,
  `door_gate`), and four colour maps that 0.26 gave the alpha heights its blend
  reads (`floor_trash`, `trash`, `brick_02`, `briks_trash_02`). They are
  referenced as the native textures.
