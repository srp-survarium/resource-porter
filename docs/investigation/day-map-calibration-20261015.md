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

## Runtime batch 2 on `816cadcd` (2026-10-15 03:01–03:09)

The batch captured the original 0.11e at seven new views: Chemical
`boiler-hall`, `chem-bones`, `chem-wall`, `chem-cabinets`, and Radar
`door-close-pz`/`px`, `door-far`. It then ran five harness arms: Chemical and
Radar 0.11e with the `source` look and with `source-probe-specular`, and
Radar 0.20e `source`. All frames are 1280×720 with matched cameras, and both
steps returned rc=0.

Scripts and outputs:
- Scoring: `measure_shadow_specular.py` (shadow and sunlit masks taken from
  the original frame) and `compare_release_arms.py`.
- Repro: `analyze_batch2.sh` in the lighting scratchpad.
- Outputs in the scratchpad: `analysis-batch2/{mae,sheen}-{chemical,radar,r20}`.

| Map (views) | `source` mean MAE | `source-probe-specular` mean MAE | Views where specular is better |
| --- | ---: | ---: | ---: |
| Chemical 0.11e (16) | .0802 | .0831 | 6 |
| Radar 0.11e (20) | .0761 | .0783 | 0 |

The source probe specular does not remove the reported sheen.

- **Shadowed pixels get darker.** Median luma ratio to the original falls
  from .735 to .719 on Chemical and from .736 to .721 on Radar.
- **Specular is not the main divergence.** On the reported surfaces it
  changes little. Region means are linear-light ratios to the original
  (R, G, B):

  | View, surface | `source` | `source-probe-specular` |
  | --- | --- | --- |
  | `chem-wall`, panels in open shade | .95 .98 1.08 | .71 .76 .89, and mottled (p95−median luma .184 vs .057 in the original) |
  | `central_shadow`, left wall | .24 .23 .28 | .13 .13 .20 |
  | `central_shadow`, red door | .22 .24 .36 | .21 .23 .35 |
  | `central_shadow`, shaded floor | .37 .35 .42 | .30 .31 .41 |
  | `chem-cabinets`, cabinets | .17 .18 .33 | .15 .16 .31 |
  | `chem-cabinets`, brick | .14 .16 .35 | .13 .14 .32 |
  | `central_shadow`, sunlit floor | .82 .66 .64 | unchanged |

- **What the "cool sheen" is.** Covered shade (under roofs and in passages)
  receives 3–7× less light than in the original, and the light is
  blue-shifted. Display B/R is 1.45 on the cabinets, door and brick, against
  0.96–1.15 in the original. The warm albedo (rust, red paint, brick)
  disappears and surfaces read as cold metal. Open-sky shade (`chem-wall`)
  already matches the original.
- **Next target.** The ambient diffuse term in occluded areas: the local or
  box probe diffuse, the skylight term, and the AO factor. The original
  Chemical `central_shadow` per-draw buffers (probe, skylight, SSAO) exist
  under `resource-porter/.work/chemical-plant-011a/comparison/011a/poses/`.
  A candidate buffer capture at the same view is needed to localise the
  factor.

`source` stays the default look. `source-probe-specular` stays selectable
under `looks/`.

Doors: `textures/ported/v011e/door/door_metal.dds` is 1024×1024 with 9 mips,
the same as the 0.11e source. Its DXT5 colour blocks are bit-identical to
the source DXT3; only the alpha encoding differs.

Radar 0.20e (12 frozen views): `816cadcd` scores mean MAE .0518 and median
.0365. The promoted arm scored .0510 and .0363; 6 of 12 views are better.
Shadow and sunlit ratios are within ±.05 per view. No regression.

### Covered-shade deficit: offline groundwork (no launch)

- **The blue shift is a consequence of the darkness.** At equal display luma,
  the original and the candidate have the same blue/red ratio on both maps
  (Chemical: 1.86/1.88 at luma .06–.08, 1.47/1.52 at .10–.14, 1.05/1.10 at
  .20–.28). This is the authored 0.11e dark tint, `lerp(luma×(.8,.8,1.4),
  colour, saturate(luma×3.3+.21))`, acting on values that are too dark. The
  candidate simply has 3× more pixels at luma .10–.14.
- **Display transfer.** Over all 36 matched 0.11e views, candidate display
  code c maps to original ≈ 255·(c/255)^0.73 (10→24, 30→55, 100→130,
  200→222). Under the 0.11e Chemical tone curve this means about 3× too
  little HDR in dark tones and about 1.6× in bright ones.
- **The 2026-10-13 candidate matched 0.11a HDR in shade.** Config
  `.work/chemical-plant-011a/comparison/026g/source-lighting`: at
  `central_shadow`, ambient after HBA, composition HDR, post input and AO all
  match the original 0.11a per-draw buffers within about 10%. Left wall:
  composition .0305 vs .0328, AO×2.5 1.21 vs 1.24. Since then, the
  probe-diffuse, SSAO (0.11e shader, noise and parameters 1.3/1.3),
  composition, LUT, fog and post-tail assets have all changed.
- **0.11e is brighter in shade than 0.11a.** An offline model of the 0.11e
  post (exposure key, toe/shoulder, rational curve, dark tint), applied to the
  0.11a post input at `central_shadow`, predicts display luma .234/.190/.229
  (left wall / red door / shaded floor). The 0.11e original shows
  .320/.202/.337. The same model reproduces 0.11a's own display
  (.218/.211 vs .210/.206). 816cadcd shows .159/.108/.213, below even 0.11a
  lighting viewed through the 0.11e post.
- **Two factors follow.**
  - F1: the candidate's shade HDR fell since the matched 10-13
    configuration.
  - F2: 0.11e shade HDR exceeds 0.11a by about 1.5× on the wall and floor.
    The door is unchanged.
- **Next.** A candidate buffer capture at `central_shadow` and `chem-cabinets`
  (`analyze_buffers.py` in the scratchpad is ready) compares stage by stage:
  AO, probe encoding and alpha, diffuse before and after HBA, composition
  HDR, post input, exposure average and display.

## Covered-shade root cause: height ambient bypassed the probe decode

The buffer capture at `central_shadow` and `chem-cabinets` on `816cadcd`
located the loss in the ambient (HBA) stage. The table compares the
2026-10-13 configuration, which matched the original 0.11a, with
`816cadcd`.

| Left wall | 10-13 | `816cadcd` | Original 0.11a |
| --- | ---: | ---: | ---: |
| Accumulated diffuse before the stage | .149 | .134 | – |
| Accumulated diffuse after the stage | .215 | .048 | .205 (sky + probes) |
| AO×2.5 | 1.21 | 1.36 | 1.24 |

The stage's shader differs between the two runs:

- **2026-10-13:** the hook's decode override (`b2750fd5`, 920 bytes) runs as
  an overwrite. Its output, 8·E²·AO·2.5 = 8·.149²·1.21 = .215, matches the
  table.
- **`816cadcd`:** the native height-ambient multiply shader (`fff29cc6`,
  2388 bytes, blend ZERO / SRC_COLOR) runs instead. It multiplies the
  still-encoded probe light E (≈ √(L/8)) by the height colour
  (.32, .393, .5)·AO.

The cause is the converter. It set `use_height_based_ambient=true` for every
early map (`transfer_environment`). The hook can replace only 0.26's
SSAO-only multiply shader (660 bytes, `e852c81d`), so the decode never ran.
SSAO, probes, composition and post (H1a–d) were not at fault. The blue cast
and the "cold sheen" in covered shade follow from this darkness (see the
groundwork section above).

**Fix:** `1e3864f` keeps height ambient off for early maps. The rebuilt
projects are byte-identical to the in-place patch that was runtime-tested.

## Remains repair: runtime-proven switches

`7cb2f14`'s strict rule refused the skeleton/rag
`use_tspecular_intensity=1` restore. 104 shipped `gbuffer_pass` keys with
the remains' switch digits lack it. They differ in vertex input (all types
up to 14) and in undecoded material digits (34–37, 52), so they belong to
vertex-blended and other materials. Restricting to static vertex inputs
still leaves 24 such keys.

The repair loaded in the 2b35a2ed user session. Port manifests now carry
`runtime_proven_switches`:

- Each entry names the donor material, the switches and the evidence.
- Each entry is pinned to the SHA-256 of the sorted shipped shader key paths
  (`f84f6a63…`). Evidence for another catalogue is a build error.
- A restore is accepted when its switches are a subset of the proven ones.
  Notes say `restored from source (runtime-proven load)`.
- Everything else keeps the strict proof, so the flora crash case stays
  refused (test `runtime_evidence_restores_a_switch_the_static_proof_refuses`;
  297 tests pass).
- `verify_shader_variants.py --evidence MANIFEST` lists such materials under
  `runtime_proven` instead of `missing`.

## Rebuild `eeaa13ed`

Rebuilt from retail 0.26g (`c74078be`), with 13,812 resources verified. It
differs from `816cadcd` in exactly four files: the two 0.11e projects
(height ambient off) and the v011e skeleton/rag materials. Verifier against
`8e72816c` with evidence: 135 materials checked, 0 missing, 0 new casters,
2 runtime-proven.

Static arms on the harness, scored against the originals:

| Map | `816cadcd` mean MAE | `eeaa13ed` mean MAE | Views better | Shadow luma ratio | Sunlit luma ratio |
| --- | ---: | ---: | ---: | --- | --- |
| Chemical 0.11e (16) | .0802 | .0497 | 16/16 | .735 → 1.025 | .621 → .693 |
| Radar 0.11e (20) | .0761 | .0368 | 20/20 | .736 → 1.063 | .695 → .868 |
| Radar 0.20e (12) | .0518 | .0515 | – | unchanged (project untouched) | |

Display luma per region, `816cadcd` → `eeaa13ed`, with the 0.11e original in
the last column:

| Region | `816cadcd` | `eeaa13ed` | 0.11e original |
| --- | ---: | ---: | ---: |
| `central_shadow` left wall | .159 | .256 | .320 |
| `central_shadow` red door | .108 | .190 | .202 |
| `central_shadow` shaded floor | .213 | .299 | .337 |
| `central_shadow` sunlit floor | .497 | .576 | .586 |
| `chem-cabinets` cabinets | .139 | .250 | .284 |
| `chem-cabinets` brick | .108 | .226 | .240 |

Remains, original → `816cadcd` → height-ambient fix alone → `eeaa13ed`:

- **`chem-bones` bone pixels:** p99 luma .937 → .910 → .925 → .809. The
  specular streaks on ribs and skull are gone and the bones read matte as in
  the original. Mean luma is .620 → .454 → .527 → .479, still darker than
  the original.
- **Radar skeleton views:** MAE .050 → .029 (`skeleton-lower-path`) and
  .134 → .067 (`skeleton-central-path`).

The first Radar run after copying the new database rendered only the sky:
203 presents over the whole run, against about 4,450 normally. This is
consistent with a cold cache after the 6 GB copy. A rerun was normal and is
the one scored.

## Floor lettering and stripes (user reports 04:29–04:39)

The "missing texture" lettering on the control-room moss is in the G-buffer
normal. A world position reconstructed from the captured depth lands on the
decal `editor_decals_moss` at (-83.0, 12.19, 35.6). In the 0.11e source that
decal has `use_nmap=false`, but the template kept it on. With the switch on,
the decal samples the source's own `textures/editor/editor_dummy_nmap`, which
is itself a normal map lettered "missing texture". The original never drew
it. Only g-stage switches were being restored.

Fix (`885cf02`):
- Switches on other stages are restored from the source when a native
  material uses the same effect with exactly the resulting switch set
  (2 v020e moss decals, `use_tsmoothness`).
- Otherwise they are restored when runtime evidence lists them as
  `stage.use_...` (14 v011e decals, `decals.use_nmap=0`). The evidence is a
  static Chemical and Radar load of the patched database plus the A/B at the
  moss.
- An audit of every ported material against its source found no other
  non-g-stage switch divergence.

Separately (`adbb9d7`), references to textures that exist in neither archive
are bound to an all-zero texture. This is what the original's failed load
leaves its shaders reading (0.10b `resource_manager::on_texture_loaded`). It
covers 101 sampled and 118 inert dangling references, the same set in
`8e72816c` and `eeaa13ed`; the retail and source `asphalt` normal is one of
them. `verify_material_textures.py` reports 0.

Floor stripes at the user's spot near the east sitting skeleton:
- They are thin horizontal lines in the G-buffer albedo inside the
  `puddle_rust_01` decal at (-69.7, 55.8).
- Hiding that decal removes them. Hiding the overlapping `puddle_rust`,
  raising draw priority, or multiplying `decal_far_distance` by 4 does not.
- The decal's texture and parameters match the source byte for byte.
- The stripes appear identically with no shader overrides at all (pure
  native renderer), and the SSAO replay writes only its own targets.
- So this is the 0.26 decal pass on this source decal. It is not fixed. The
  fidelity run compares the original at this pose.

Rebuild `25ab9683` differs from `bf7d304b` in 16 decal materials.
- Verifiers: 0 missing textures; 0 missing shaders (2 g-stage runtime-proven,
  14 stage-switch evidence entries).
- Static arms, mean MAE: Chemical .0497, Radar 0.11e .0367, Radar 0.20e
  .0511. These equal `eeaa13ed` within noise.
- The moss normal buffer is clean.
- Harness fix `4f286c8`: readiness ignored stale status and logs, which had
  produced two sky-only captures.

## Fidelity run 2026-10-15 (`25ab9683`)

Gallery: `/mnt/hdd/Projects/survarium-fidelity-20261015/index.html`. Each view
page has an in-place toggle (click or hold Space) between original, port and
port with the probe-specular look; side-by-side views; a FLIP error map;
highest-error tiles; and a checklist with notes (browser storage, exported as
JSON). The run is pinned by `manifest.json`, scored in `report.json`, and the
tools are in `tools/render-compare/fidelity/`.

Method:
- Fresh original references for every pose: 0.11e for Chemical and Radar,
  0.20e for Radar.
- Settings aligned to the port: texture 2 on both sides. On 0.11e: post 3→2,
  decorations 2→1, motion blur 3→2, preset 4→5 (custom).
- Five frames per pose on each side, compared as the per-pixel median.
- Every frame's camera is checked against the request; 0 views rejected.
- NVIDIA FLIP (LDR) per pixel. Sky is flagged and excluded from the non-sky
  score.
- 1280×720, which is also the user's -next resolution.
- The 0.11e capture hook stops acknowledging after about 155 frames per
  launch. Its remaining views were captured in a second launch.

| Map (views) | FLIP full | FLIP non-sky | FLIP shadow | FLIP sunlit | Shadow luma ratio | Sunlit luma ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Chemical 0.11e (37) | .222 | .221 | .141 | .452 | 1.00 | .68 |
| Radar 0.11e (36) | .192 | .191 | .128 | .396 | 1.02 | .83 |
| Radar 0.20e (28) | .191 | .190 | .153 | .298 | .87 | .91 |

The probe-specular look changes FLIP by less than .002.

Findings:
- Covered shade now matches (luma ratio about 1.0).
- The largest remaining gap is brightly lit surfaces on Chemical (ratio .68).
  The worst view is the control-room floor `upper-sit1-down60` (FLIP .605):
  the original floor is brighter and warmer.
- The 0.20e `hangar-stairs` sunlight is the known missing casters/shadow item.
- At `sit-east-000` the original's `puddle_rust_01` shows a bright orange rust
  area and no stripes. The port's is dull and striped, so the 0.26 decal
  blend differs from the source for this decal.
