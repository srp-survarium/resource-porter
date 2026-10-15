# Day map calibration, 2026-10-15

Continues [the release notes](day-map-release-20261014.md) for maps 194
(Chemical 0.11e), 195 (Radar 0.11e) and 196 (Radar 0.20e). Evidence is in
`resource-porter/.work/day-map-calibration-20261015/`; its SHA-256 list is
`day-map-calibration-20261015-sha256.txt` beside it.

## Material switch defect (glossy metal, golden remains)

The playtest reports cover a glossy checker-plate walkway, a bluish sheen on
rusty sheet metal and dry grass, glossy electrical cabinets in a dark room, and
golden skeletons on Chemical. These are one converter defect, the same one
the Factory remains patch fixed by hand (see
[factory-day-material-flags](factory-day-material-flags.md)). The converter
copied the donor's scalars and textures, but took every boolean from the
native template. Across the 620 materials of the three maps, 142 gloss switches
differed from their donor:

| Switch (donor → converted) | Count |
| --- | ---: |
| `use_troughness` false → true (constant roughness replaced by the template's textured range) | 54 |
| `use_tfresnel` true → false (Fresnel map lost) | 41 |
| `use_tspecular_intensity` true → false (intensity mask lost) | 29 |
| other gloss directions | 18 |

`audit/feature_audit.py` (in the checkpoint) compares every converted
material with the donor material extracted from the 0.11e/0.20e archive.
`before.json` and `after.json` list every switch difference, including the
non-gloss ones.

### Converter repair (`b1d34c1`)

- Template choice prefers a shipped native material that carries the donor's
  `use_troughness`, `use_tfresnel` and `use_tspecular_intensity`. It switches
  only when the alternative template adds no other feature difference (alpha
  test, normal maps, wind and so on) that the closest template did not
  already have. A first attempt without this guard swapped alpha test and
  wind on 51 materials. That version was discarded.
- Remaining donor switches are restored when a `gbuffer_pass.ps` permutation
  ships for the result at both shading qualities. The key digits come from
  `shaders/masks` membership across all shipped shaders: 5 tdiffuse, 6 nmap,
  7 detail nmap, 8 alpha test, 10 parallax, 11 translucency, 12 specular
  intensity (`CONFIG_TSPECULAR_COLOR`), 13 Fresnel, 14 roughness, 16 detail
  diffuse. All 1,704 native g-stage materials project onto shipped keys.
  Swapping any two of these positions breaks 70 to 1,616 of them. The catalog
  rechecks this on load and disables the restore if the layout ever differs.
- When only textured roughness ships, a donor constant roughness becomes the
  zero-width range `min = max = constant`. Native smoothness is
  `1 - (min + (max - min) * texture)`, so this is exact; the Factory remains
  validated it at runtime.
- Unsupported switches keep the template value and are written to the
  compatibility notes. Regression test:
  `restores_source_gloss_switches_from_shipped_permutations`. All 286 library
  tests pass.

The rebuilt skeleton and rag materials are byte-identical to the separately
validated remains repair (`bc6a9977…`, `5908c03a…`).

Results after the repair: gloss switch differences fell from 142 to 13, and
no non-gloss difference was introduced. Eight of the 13 are exact zero-width
emulations. Five remain unresolved, because no shipped permutation exists:

- `metal_b`, `metal_column_02`, `metal_column_08` and `metal_gray_vc` (0.11e)
  still lack their intensity mask.
- 0.20e `rabica` still lacks its Fresnel map.

`converter-switch-notes.json` lists every restored, emulated or unresolved
switch. Other residual differences are unrelated to gloss: 14 decals bind the
flat dummy normal map, and `wall_pol_potolok02` binds a template normal map.

### Packaging

The changed materials (128) plus the door texture repair form a guarded
material-only overlay
(`tools/playable-map-pack/prepare_material_switch_overlay.py`). Every
replaced resource must still hold the original bundle bytes in the target
database, and the manifest pins the database hash. `vostok-port apply` then
merges it. Its whole-database model check previously failed on an unrelated
retail omission. It now validates every model that binds a replaced material,
but skips unrelated retail models (`226ff17`). Applied to the current package
database (`bc411413…`, which already includes the gameplay worker's project
patch), the overlay produces `bf248730…`. That database lists the same paths
and verifies against the overlay. It sits in a private harness only. The
shared package is unchanged until the runtime batch confirms there are no
missing-permutation errors and the matched views improve.

## Radar 0.20e (196) arms already captured

Twelve frozen original views. Cameras match exactly, all receipts succeeded,
owned processes exited, and the private database and presets were restored.

| Arm | Mean full-frame RGB MAE | Views better than native |
| --- | ---: | ---: |
| native preset, merged hook | .0899 | – |
| source ambient + cloud lighting off (accepted) | .0510 | 9/12 |
| + source cloud coordinates/shader | .0488 | 9/12 |
| + coordinates/shader + Mie migration | .0511 | 12/12 |

The accepted ambient and cloud-off combination reproduces with the merged
hook, so it is promoted.

The cloud-coordinate swap is not promoted. The bytecode does show native
uploads `(sin,cos)` where source uploads `(cos,sin)`: the native helpers
`0x38ff0`/`0x39000` are `fsin`/`fcos`, and source calls `0x29d236` (sin) and
`0x672ed6` (cos). But in the matched sky view, the native layout already
resembles the original's cloud mass, and the swap worsened that view from
.0688 to .0781. The original reference session had no clock synchronisation,
so frames cannot validate cloud phase. Cloud shadows also move the
`hangar-stairs` wall brightness between arms, so that view's per-arm number
reflects cloud phase, not a fix.

The Mie migration has not yet been measured without the cloud swap. That arm
is part of the pending batch.

## Radar 0.20e stair sunlight

An offline analysis ruled out the sun direction, every axis or sign swap,
face culling, the low-detail mesh, cascade sizes and bias, portal selection
and the other caster filters. Every caster omitted from the original's sun
shadow uses a material with `g_stage.with_static_vertex_color_meshes=true`.
Dropping those materials as casters matches 227 of 288 wall pixels. Keeping
all casters matches 139 and lights none. The likely code site is each
material's per-layout shadow-effect pointer (`+0x94` at `0x105788` in 0.20e,
`+0x90` at `0x1751cf` in 0.26), but no write to it has been traced.

Nothing is promoted. A read-only hook in the original 0.20e session at
`0x105788` would confirm or refute this. Details and scripts:
`/tmp/radar-020e-stair-shadow/fork-analysis/FINDINGS.txt`; hashes in
`fork-analysis-sha256.txt`.

## Pending single runtime batch

`run_batch.sh` (private scratch) runs, in order:

1. The original 0.11e client, capturing seven new matched views: Chemical
   `boiler-hall`, `chem-bones`, `chem-wall` and `chem-cabinets` (the reported
   spots) and three Radar door-metal views.
2. The current database with the merged hook (baseline, including the door
   binding observer).
3. The material-fixed database, alone and with each candidate: post-tail,
   cone opacity, sun sentinel plus F0=1 local diffuse, donor 0.11e SSAO, and
   the 0.20e accepted and Mie arms.

All runs use one client at a time. They hold the shared candidate runtime
lock and refuse to start beside another client.

## Clean rebuild (2026-10-15, later)

Candidate `/tmp/three-day-map-candidate-next` (shared package left untouched):
`resources.db` SHA-256 `ace67766d05aa1403f35695567dc2de0ebd98c5f31545601d5fc7db311431fc9`.
It was rebuilt from retail 0.26g with `build-survarium021-maps` for 0.11e and
0.20e, then `compose-day-map-bundles` (13,812 resources verified), then
`vostok-port apply`, then `vostok-port verify`. No post-build patches. All 294
library tests pass, including the merged converter-review and gameplay-fixes
branches.

### File-level diff against the current package (`8e72816c…`)

13,592 bundle files are byte-identical. 208 differ and 12 are new:

| Files | Cause |
| --- | --- |
| 178 materials (119 v011e, 59 v020e) | Source switch/render-state/empty-slot restoration (`1607fa0`) |
| 24 flora/tree model settings | Foliage game-material classification (gameplay, `1063a00`) |
| v011e level_03 project | 16 `wire_set` objects restored (`3659fe0`); empty `antenna_tree` collision omitted (`2e1b6c0`); 5 ambient sounds now donor stems (`2722518`); environment gamma, fog start and renamed fields (`36b43cc`); respawn ally/enemy radii removed (gameplay) |
| v011e level_04 project | Environment gamma, fog start and renamed fields (`36b43cc`); respawn radii removed; 1 ambient sound stem |
| v020e level_03 project | 7 empty-folder collision instances omitted (`2e1b6c0`); cloud lighting off and Mie 2 (`9fb7b65`); respawn radii and inherited 6 s protection removed (gameplay) |
| remains skeleton/rag DDS | Converter BC2→BC3 (`36aaf76`); RGB blocks identical to the earlier overlay |
| `door_metal.dds` | DXT3→DXT5 by the converter; RGB blocks identical |
| 12 new `sounds/…/v011e/scapes/*` | Donor stems with native-format options (`2722518`) |

Battery positions on all three maps match the current package.

## Material switches: shader-variant safety

The first gloss-switch database crashed during a live Chemical load. The
missing key was `gbuffer_pass.ps/_3___1101_01101_…002…`, a translucent
wind-animated tree with a restored intensity mask on vertex input 1. The
earlier check matched only material digits. The catalog now indexes shipped
keys by projection, by the remaining material digits, and by geometry
variant. Geometry positions 17/22/31/33/51 come from the `vertex_base.vs`
mask. A switch is restored only if every variant the material can request
still ships. Plain static materials need only vertex inputs 0/1; all others
need every variant (regression test included).

On the rebuilt bundles, all 178 changed materials pass an independent Python
re-check. Against the donors, differing switches fall from 211 to 77 and
template textures in empty source slots from 156 to 21. The 67
`use_tiled_vertex_blended_mask` (vertex-blended materials) remain; turning it
off fails the safety check.

## User reports traced

- **Sheet-metal wall** (23-38-54, 00-59-46): `ported/v011e/tiled/metal/metal_gray_vc`
  on `level/04/loading_station_02` (ray cast from the reported camera). The
  source has Fresnel and intensity masks, but no shipped variant can carry
  them for this vertex-blended material, so they stay unresolved. Its sheen is
  the native ambient specular.
- **Walkway** (`central_shadow`): `metal_red`, `metal_column_03` and
  `floor_plate_tile` have no gloss-switch difference.
  - `floor_plate_tile` uses its diffuse texture as roughness. The native copy
    of that trailing-space texture is effectively identical.
  - Red versus green channel: 0.384 vs 0.363 mean, a negligible difference.
  - The remaining shared cause is ambient probe specular. The 0.11e probe
    shader (`9dd5d3bf`) suppresses it by `1 - gloss^3`, AO^5, a cap of 4 and
    screen-space occlusion. The retained native producer has none of these;
    the Factory buffer audit measured it about 400× above the source on
    shadowed dielectrics. `CONFIG_SOURCE_PROBE_SPECULAR` (opt-in) is
    the A/B candidate.
- **Dark shadowed and indoor areas in play**: besides the height-ambient gamma
  (`36b43cc`), the user ran at 2558×1440. The archived SSAO replay accepted
  only 1280×720 depth. At any other size it never became ready, and the
  paired ambient decode drew without its AO input. The hook status showed
  "original SSAO requires single-sample 1280x720 R16F depth". Fixed in
  `477a371`; all earlier captures were 1280×720 and could not show it.
- **Cyan glossy pool next to the bones**: `ported/v011e/anomaly/anomaly_liquid`
  on `level/04/moss`. It is an anomaly-liquid material, not water, and has no
  switch difference from the source. The new `chem-bones` original view will
  settle its colour.
- **Cabinets**: `electrical_cabinet_01/02` and `mech_cabinet_02`. Their
  roughness and specular switches are now restored where variants ship.

## Looks

`environment_look` (per map in the port manifest) and
`build_day_presets.py` keep two looks:

- **`source`** (staged default): 0.11e LUT, geometry-fog fade, clouds (quarter
  turn, Chemical ×3), 0.11e post tail, god rays on, Rayleigh haze from the
  camera.
- **`legacy-010b-atmosphere`**: the snapshot's LUT, fog, clouds, tone curve
  and Radar god rays off, plus the template fog start via
  `ports/survarium-011e-day-legacy-look-to-026g.json`.

All bug fixes apply to both looks. Preset variants are staged under
`looks/` in the staged candidate.

### Rebuild after the footstep/border merge

After merging `e024b42` (moss footsteps, north-border boxes; 295 tests pass),
the staged candidate was rebuilt the same way. `resources.db` is now
`cc1a78d058fcbf754ad3bc3ddfe8c58a524e19f6b31c4a04c289cf15a9226050`. Against
`ace67766`, 13,807 files are identical and 5 differ:
`level/04/level_border.model/walker_collision/exported_primitives` and
`moss{,2,3,4}.model/settings`. The path lists are equal and the 0.20e bundle
is identical. The gameplay diagnostic plans were regenerated with
`prepare_delivery_diagnostic.py` against this database under
`/tmp/day-map-gameplay-20261014/diag-next/`, including the new
`194-fence` spawn plan.

### Second load crash and the complete variant check

`cc1a78d0` crashed on the Chemical load: `gbuffer_pass.ps/_3___1101_01000_00____11_…`.
The material is `flora/grass_l04`, drawn by vegetation instances (vertex
input 11, a 68-character key). Its constant roughness had been restored by
flipping `use_troughness`, and the check had assumed that `.model` users need
only the static mesh inputs 0/1. `bg_trees`/`sosna_bark_lod` had also become
shadow casters, which means new shadow-pass requests.

The check (`material_conversion.rs::variants_ship`) no longer models how a
material is drawn. The unconverted material is known to load, so every key
it requests ships. Every shipped key, of any of the 286 shader families,
that agrees with the material on all decoded switch digits the family uses
could be such a request. The same key with the changed digits must also
ship. Geometry, LOD, instancing, vertex-input and undecoded material digits
carry over unchanged. Shadow casting is only ever removed, never enabled.
Regression test: `keeps_vegetation_vertex_input_variants_when_restoring_a_switch`.

`tools/render-compare/verify_shader_variants.py BASELINE CANDIDATE` applies
the same proof to every changed material of a built database against a
database known to load:

| Candidate (baseline `8e72816c`, played) | Changed materials | Materials with missing shaders | Missing names | New shadow casters |
| --- | ---: | ---: | ---: | ---: |
| `cc1a78d0` (crashed) | 178 | 95 (crash key listed for `grass_l04`) | 13,235 | 2 |
| `8e72816c` itself | 0 | 0 | 0 | 0 |
| `816cadcd` (rebuild) | 135 | 0 | 0 | 0 |

The proof is conservative: of the gloss switches only 18 are now restored.
Constant roughness is still emulated exactly on 54 materials; 41 lost Fresnel
maps and 29 lost intensity masks remain the template's choice. Template
textures in slots left empty by the source are cleared on 106 materials. The
rebuilt candidate `816cadcd…` differs from `cc1a78d0` only in 97 material
files.
