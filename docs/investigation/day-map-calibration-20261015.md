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
