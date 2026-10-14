# Converter correctness review, 2026-10-14

Scope: the early/0.20e → 0.26g path (`src/survarium_026.rs`,
`src/survarium_026/early.rs`, `src/material_conversion.rs`, `src/dependency.rs`)
and the patch scripts around it. The question is where the converter writes
something the source engine would not render or simulate, especially values
inherited from a native 0.26 template instead of derived from the source.
Line numbers refer to branch `converter-review` unless stated otherwise.

Data: the three-map candidate `/tmp/three-day-map-candidate/resources.db`
(194 Chemical 0.11e, 195 Radar 0.11e, 196 Radar 0.20e), donors 0.11e
(`694e1772…`), 0.11a (`f59650ee…`, Radar server fallback) and 0.20e
(`5f871046…`), retail 0.26g (`c74078be…`). Ground truth: the 0.10b source in
`vostok/sources`, the 0.11e/0.20e PDB builds, and the 0.26g labels in
`skhid-srp/tools/re-kit/v026g` plus disassembly.

## How the candidate was produced

I rebuilt both bundles from the converter at `61d358c`. Then I compared every
bundle file with the candidate database (`examples/diff_bundle_db.rs`):

| Bundle | Same | Different |
| --- | ---: | --- |
| 0.11e (194, 195) | 8,579 | boiler `walker_collision/exported_primitives`, both projects (`invulnerability_time` 2000→2 on 44+43 points), `remains/{skeleton,rag}.dds` (BC3 overlay), dictionary |
| 0.20e (196) | 5,211 | project (`use_sky_clouds_lighting` true, `mie_fog_sun_color_falloff` 1), dictionary |

The candidate is therefore the converter output plus three post-build
patches. The 196 project is **older** than the converter's own 0.20e cloud
and Mie fixes: the converter now writes false and 2. Every material in the
candidate is byte-identical to the converter output. The material findings
below therefore describe the converter's behaviour, not a stale patch.

## Ranked findings

Owner: **L** = lighting worker (material switches, glossy metal, lighting);
**G** = gameplay worker (cylinders, respawns, batteries, collectors);
**R** = this review (fixed on branch `converter-review`, or still open).

### 1. Material shader switches come from the 0.26 successor, not the source (L, open)

*Impact:* visible on every converted map; this is the glossy skeleton bug.
`MaterialCatalog::convert` clones a native template.
`merge_safe_visual_values` (`material_conversion.rs:516-604`) then copies
only donor floats, non-feature integers and existing textures. Every
`use_*`/`is_*`/`with_*` switch keeps the template value, and the test
`keeps_template_shader_switches_but_moves_visual_values` enforces it. 485 of
the 620 templates are the same-named 0.26 material, so the template is the
0.26 re-authored successor of the source material.

I diffed every parameter of all 558 converted materials on the three maps
against its donor material. The full table is in
`converter-review-20261014/material-source-diffs.tsv`:

| Class | v011e | v020e |
| --- | ---: | ---: |
| switch value differs from source | 145 | 80 |
| texture binding differs from source | 145 | 39 |
| switch absent in source, enabled by template | 47 | 31 |
| texture absent in source, bound by template | 36 | 31 |
| scalar differs from source | 0 | 0 |

179 of the 558 materials have at least one switch that differs from the
source:
- **Gloss** (130 materials): `use_troughness` false→true ×54,
  `use_tfresnel` true→false ×41, `use_tspecular_intensity` true→false ×29.
  - The shipped 194/195 `remains/{skeleton,rag}` still have
    `use_troughness`=true, `use_tspecular_intensity`=false and a 0.1–0.2
    roughness range. The source has false, true and constant 0.5. The Factory
    database patch never reached these maps.
- **Not lighting, but the same mechanism:**
  - `use_alpha_test` false→true on `item/container_for_qi`,
    `props/bag/sandbags` and `props/tool/gymnasium_tools`: holes cut by the
    diffuse alpha.
  - `use_alpha_test` true→false on v020e `trash_brick_vc`: cutout rendered
    opaque.
  - `two_sided` flipped on 6 v020e materials. The template also adds
    `two_sided`=true on 11 v011e decal, cone and water materials; 0.10b
    back-face culled forward water.
  - `wind_motion` 2→0 on v020e `vehicle/tent_det`.
  - `use_parallax` lost on `bricks_brick`.
  - `with_static_vertex_color_meshes` flipped on `pol` and `concreat_decals1`.
  - `use_masked_diffuse_color` lost on both chemical barrels.
  - `use_nmap` false→true on 14 decals.
  - Kamaz/UAZ wheels get a `metal` base material with
    `is_anisotropic_material`, `use_emissive_map` and
    `fresnel_material_type` 0. The source was `complex`, non-emissive and
    non-anisotropic.
  - v020e `metal_green` loses `use_emissive_map` and anisotropy.
  - `is_cast_shadow` true→false on `flora/bg_trees` and
    `flora/sosna_bark_lod`, outside the effect block. This is the
    `_not_move` template choice that `prepare_static_tree_wind.py` patched
    for 0.10b.
  - 37 decals switch base material from `decals_default` to `decal_test_2`.
- **New 0.26 features switched on by the template:**
  `use_tiled_vertex_blended_mask` with a native
  `tiled/vertex_blend_masks/tiled_mask_01|03` on 36 v011e and 31 v020e
  vertex-blended materials. The source blended by vertex colour alone, so
  the blend pattern changes. This is the hangar "green edges" issue left
  open in `day-evening-render-parity.md`.
- **Textures:** 145 of the bindings are template textures where the source
  had `""`. Examples are `texture_roughness` ×34 (e.g. `grass_l04` →
  `flora/grass_l04alt_smap`), `normal_waves_texture` ×34, `texture_fresnel`
  ×13 and the detail/alphablended normals. `merge_safe_visual_values` keeps
  a non-empty template texture whenever the donor's is empty (`:592-601`).
  Combined with a template-enabled switch, the source surface samples the
  successor's roughness or Fresnel map.

*Fix direction (L):* derive every switch from the source and pick a shipped
permutation that matches. Where none exists, emulate it with scalars, as the
remains plan did with min=max=0.5. Clear template textures whose source
slot is empty. `is_cast_shadow`, `base_material` and the stage selectors need
the same treatment. The uncommitted `restore_source_gloss_switches` in
`/tmp/resource-porter-render-compare` covers only the three gloss switches.

### 2. Collision cylinders (G, open)

*Impact:* movement and bullets on 194/195. Every exported cylinder stores
z=0. 0.11e `create_bt_primitive` (0x724930, z read at 0x7249e2) builds
`btCylinderShape` from [x,y,z], so the shape is flat with zero margin and is
mostly skipped by broadphase and ray tests. 0.20e and 0.26 (0x7a2b9) build
from [x,y,x], a solid cylinder. 2,009 primitives on 194 and 1,044 on 195
(692 and 269 in movement groups) are therefore solid in 0.26 but were not in
0.11e. 196 is unaffected. Box, sphere, capsule, rotation and scale are
identical across builds.

The boiler `collision_successors` entry (`5d240c0`) fixes one symptom of
this, for one model on one map. The next 0.11e map will block players at
every walker cylinder.

### 3. Empty collision payloads were replaced by the other collision kind (R, **fixed** `2e1b6c0`)

*Impact:* movement and bullets on 195/196. Ten `collision_objects`
references point at an empty folder node in the donor. Retail 0.26 ships the
same empty `walker_collision` nodes for these models and references them in
`level_03/day_2`. The source cook probes them and creates no shape.
`adapt_collisions` substituted the other kind:
- the bullet meshes of `anomal_tree_01..04_` (5 instances, 285–460 triangles
  up to 12 m) entered the movement group on 196;
- `antenna_tree` (1,777 triangles) did the same on 195;
- the walker shape of `soviet_comp_01` entered the bullet group on 196.

Fix: empty folder nodes are omitted, with the note "source creates no
shape". Absent references keep the reviewed allowlist behaviour. Test:
`empty_collision_folder_is_omitted_not_replaced_by_the_other_kind`. The
rebuilt bundles show exactly these repairs and no other change.

### 4. Most early lighting fidelity lives in runtime hook presets, built for other donors (L, open)

*Impact:* everything lit on 194/195; 196 has no hooks at all. The candidate
presets load the `SRP_LEGACY_*` shader replacements through `camera_hook.dll`:
- atmosphere LUT, geometry fog, sky and clouds;
- probe diffuse and box probes;
- tonemap, composition and ambient decode;
- SSAO, present gamma, grain and aberration.

`tools/playable-map-pack/prepare_day_candidate.py` copies the
**chemical-011 (0.11a)** preset to Chemical 0.11e and the **factory-010
(0.10b)** preset to Radar 0.11e. The hook data does not match 0.11e:
- **LUT:** 0.10b extinction constants (−0.176/−0.298/−0.617, mie 0.0126).
  0.11e `make_mie_rayleigh_texture` uses −0.859/−1.488/−3.086 and 0.1257.
- **Geometry fog:** lacks the 0.11e `saturate(depth/(0.25·D))` fade.
- **Tonemap curves:** 0.10b/0.11a values. The 0.11e tail
  (`legacy_011e_post_tail.hlsl`) exists but is not wired in.
- **SSAO:** hard-coded 2/1.5, while 0.11e authors 1.3/1.3 (194) and 1.5/1.5
  (195).
- **Radar probe gather:** 0.10b mip 1 ÷9, where 0.11e uses the last mip ÷8.
- **Inconsistent compensations:** the probe sort fix is active on Chemical
  only, and god rays are disabled on Radar only.
- **Leftover diagnostic:** `SRP_CAPTURE_MATERIALS=1` remains in the Chemical
  preset.

None of this is reproducible by the converter. A new map gets none of it, or
the wrong donor's version.

### 5. Height-based ambient colour has the wrong gamma (L, open)

*Impact:* all indirect light on 194/195 is about 2.3× too dark.
- **Converter:** `early.rs:500-524` writes `skylight_color ×
  skylight_multiplier` into `height_based_ambient_high_color`.
- **0.10b** (`stage_ambient_lighting.cpp:156`): uses `pow(skylight_color,
  2.2) × skylight_multiplier`.
- **0.26:** raises environment colours to the power 2.2 at load (helper
  0x1bfcb0) before upload.
- **Result:** the multiplier ends up applied before the power, so the
  multiplier 0.5 becomes 0.5^2.2. The value to write is
  `color × multiplier^(1/2.2)`.
- **Chemical clouds:** they lose their `sky_clouds_color_multiplier` of 3 for
  the same reason.

### 6. Shipped candidate is not the converter's current output (process, open)

- **196:** still has `use_sky_clouds_lighting` true and
  `mie_fog_sun_color_falloff` 1 from the native template. The converter
  already preserves false and 2 (`survarium_026.rs:385-432`).
- **194/195:** depend on three post-build patches. Two of them are now in
  the converter: respawn seconds (`1e43d53`) and the boiler (`5d240c0`).
- **Not applied anywhere:**
  - the remains material plan (`prepare_factory_011e_remains_overlay.py`);
  - the door BC3 (`prepare_radar_door_bc3.py`);
  - the 196 battery body offsets (`release-fix-plan/plan.json`).

Rebuild the candidate from the converter, not by composing patches.

### 7. Early wire sets were dropped (R, **fixed** `3659fe0`)

*Impact:* all 16 power wires missing on 195. `early::project` skipped
`wire_set` with a note promising a "static reconstruction" that never
existed. The 0.11e wire schema equals 0.26's. 11 of the 16 wires are
byte-identical to the 0.20e wires that 196 already ships, and 0.26
`object_wire::load` (0x293e90) reads the same fields. The wires are now kept,
and `adapt_objects` adds `layer`/`is_temporary` from the Rudnya schema. The
rebuilt 195 project has 16 `wire_set` objects. Test:
`early_wire_sets_are_kept_with_their_authored_points`.

### 8. BC2 textures stay at 64×64 in 0.26 (R, **fixed** `36aaf76`)

*Impact:* visible blur on 195/194. 0.26's pool holds only BC1/BC3 entries
(0x11b090), and streaming registration (0x1550b3) skips other formats.
Retail 0.26 contains zero DXT3 textures, while 0.10b, 0.11e and 0.21d ship
10, 10 and 5. The remains were fixed by a one-off overlay, and `door_metal`
(195) was prepared but never applied.

The new `src/dds_bc3.rs` re-encodes every imported BC2 texture to BC3:
- colour blocks, mips, cube faces and the sRGB byte are kept;
- alpha is fitted without crossing the 0.25/0.5 alpha-test references.

Rebuilt result: skeleton, rag and door have 0 threshold crossings and a
maximum alpha error of 10.2, 17.0 and 10.2 /255, matching the reviewed
overlay. Tests are in `dds_bc3::tests`.

### 9. Respawn fields inherited from the native template (G, open)

`adapt_objects` → `fill_missing` (`survarium_026.rs:363`) adds the template's
`ally_check_radius` 0.8 and `enemy_check_radius` 1.2 to every respawn point:
44, 43 and 22 points on the three maps. Neither source build had this check.
The 0.26 loader (0x2f4580) defaults both radii to 0 when absent, which
disables the check. 196 also inherits `invulnerability_time` 6 s; in 0.20e
protection came only from the base `effect_zone` (2000/2000 ms).

### 10. Post-process and fog fields that are dropped or inherited (L, open)

`transfer_environment` (`early.rs:417-560`) starts from the native
environment and writes only template keys or aliases.

Per map, 194/195 and 196 respectively:
- **Copied:** 39 and 74 fields.
- **Inherited from the template:** 25 and 13 fields.
- **Left at the 0.26 default:** 32 and 35 fields.

Concrete differences:
- **`rayleigh_fog_near_distance`:** inherited 35/40, while the source haze
  starts at the camera. It should be 0.
- **`bloom_halo_color` → `bloom_color`:** unmapped, so the source tints
  (0.7,0.5,1) and (0.8,0.7,1) are lost.
- **Bokeh depth of field:** `use_bokeh_dof` true and
  `dof_near_blurriness_far` 0.8 (0.26 `dof_far_blurness_amount`) are lost.
- **God-ray colours:** need `^(1/2.2)`, because 0.11e uploads them raw.
- **Inherited values with no 0.11e meaning:** `cloud_distance`, `sun_size`,
  `blueshift`, `bloom_intensity`, `sharpen_amount` and
  `sky_clouds_skylight/sunlight`.

Sun direction, intensity and colour are verified correct.

### 11. Probe semantics (L, open)

- **Sort order:** 0.11e sorts box probes by 0.75 × max(axis) (VA 0x703b7a);
  0.26 sorts by the authored radius. This gives 41 order inversions on 194
  and 94 on 195. Only the Chemical hook compensates. Writing the 0.11e key
  as each box probe's radius would make it converter data.
- **`cubemap_resolution_index`:** inherited as 2. The Radar global probe
  should be 3.
- **`diffuse_probe()`:** a mip tail, not irradiance.

### 12. Ambient sounds used native 0.26 options (R, **fixed** `2722518`)

*Impact:* audible on 194/195. 0.11e ships `.raw_options`, `.high.ogg` and
`.high.rms` under the same names as 0.26; the ogg payloads are
byte-identical. The dependency scanner preferred the shared native options,
which 0.26 re-mixed:

| Sound | spl_value | volume_k |
| --- | --- | --- |
| `industrial_space` | 50→60 | 1.8 |
| `creatures_line` | 35→60 | 1.2 |
| `metal_construction` | 35→40 | 0.6 |
| `wind_in_trees_wood_squeak` | 40→35 | 0.8 |

Donor stems that have `.raw_options` are now forced from the donor for early
builds, and a donor RMS cache is reused instead of generating a duplicate.
The rebuilt 194/195 projects reference `ported/v011e/…`. Test:
`early_sound_stems_with_raw_options_come_from_the_donor`.

Still open (R):
- **SPL presets are shared by name with native.** The 0.11e `default.spl`
  is a 10-key dB curve (60 dB at 1 m, 20.8 at 91 m). The 0.26 `default.spl`
  has a different volume/SPL structure. The 0.20e `anomaly.spl` also
  differs from 0.26. Relocating the presets needs 0.26 loader semantics for
  the old curve format first.
- **Legacy options for 0.10b/0.11a.** `build()` (`survarium_026.rs:1641`)
  replaces every legacy `.single_sound_options` with `wind_day.raw_options`
  (spl 35). It does not fire on these maps. For 0.10b, the dominant
  1/d curve is equivalent to spl 60, so those sounds would be about 25 dB
  too quiet.

### 13. Half-float UV precision (R, open)

0.26 static surfaces accept only half UVs (0x16ea30); 0.11e stores float2.
`early::geometry` packs them correctly, but island rebasing runs only for the
six surfaces hand-listed in `wrapped_uv_surfaces`. Measured with each
material's diffuse size over 2,070 0.11e surfaces:
- **> 0.5 texel error:** 992 surfaces.
- **> 1 texel:** 520 surfaces.
- **> 4 texels:** 38 surfaces. Examples: `anomaly_rust_tree` 16 texels,
  `central_factory` `concrete_pliti_s` 15.7, `hospital` 8.
- **Fixable to ≤ 0.5 texel by the existing rebase:** 502 surfaces.

Detail maps tile 20×, so their error is 20× larger. The decision should be
automatic per surface when all of these hold: wrap addressing, integer tile
factors, no UV animation, and an error that improves.

### 14. Physical materials follow 0.26's table (R, open)

IDs and names map correctly, and `remap_game_material_settings` changes no
ID across 710 settings. Remaining differences:
- **0.11e `resistance`:** never read by 0.26, which uses `armor`.
- **196 armour retuned:** concrete 0.8→0.23, metall_thick 0.8→0.2,
  stone 2→0.7, brick 0.5→0.2. Walls are about 3.5× easier to shoot through.
- **`foliage` → `foliage_coniferous`:** applied to deciduous flora (36
  settings, e.g. `anomal_tree_0X_`, poplars). 0.20e and native 0.26 author
  `foliage_deciduous` for the same models.

### 15. Lower-impact items

- **Game mode (G):** `.options` is a verbatim clone of the native mode
  (match time 900, respawn 8, 10 batteries, rules, `map_id`). The SRP
  `match-server-026` timer/result fixtures have no rows for 194–196.
- **Particle defaults (R):** `particle_defaults` takes
  `AnimatedVelocityMultiplier`, `Noise` and `MinScreenFactorForShadowCasting`
  from the first native file that has them. The values today are 1.0,
  Perlin and 1. Make them explicit constants.
- **Water (L):**
  - 0.11e `water`/`water_green` reference a reflection cube that exists in
    no database.
  - `soft_intersection_factor` is inherited.
  - Foam neutralisation is correct.
- **Light `min_screen_factor_for_shadow_casting` = 1 (L):** inherited. The
  feature did not exist in the source; 0 would be faithful. The effect is
  small.

## Past one-off fixes not in the converter

| Fix | Mechanism | Status |
| --- | --- | --- |
| Remains gloss switches (Factory 0.10b, Radar 0.11e) | DB patch / unapplied overlay | Not in converter; 194/195 still glossy (finding 1) |
| `metal_radio` `use_tfresnel` | DB patch | Not in converter; correct on 0.11e by template luck |
| Static tree `is_cast_shadow`/wind (`prepare_static_tree_wind.py`) | 1-byte patch, v010b only | Not in converter; 194/195 trees lose shadows |
| Roughness channel R→G (`adapt_roughness_channels.py`) | overlay experiment | Not in converter |
| Empty source roughness binding (`floor_plate_tile`) | open | Template texture still kept (finding 1) |
| Hangar tiled-mask edges | open, unmerged `codex/compare-day-evening` | Template mask still enabled on 67 materials |
| Bulb emission (`restore_factory_lamp_emission`) | in converter but keyed to one path | Hard-coded single material |
| Chemical collector zone (`repair_chemical_collector_zone`) | in converter | Hard-coded to 0.11a level_04 and one position |
| 0.20e cloud lighting / Mie falloff | in converter | Gated on 0.20e only; candidate not rebuilt |
| BC2→BC3 remains / door | overlay / unapplied | **Now in converter** (`36aaf76`) |
| Wires | never done | **Now in converter** (`3659fe0`) |
| Respawn ms→s | patch, then converter `1e43d53` | In converter |
| Boiler walker collision | patch, then per-map manifest `5d240c0` | Per-map; generic cylinder issue open (finding 2) |
| 196 battery body offsets | `release-fix-plan` DB patch | Not in converter: `battery_body_transform` runs only when `spawned_item` is absent (G) |
| Chemical battery 00 relocation | unmerged `codex/day-map-objective-spawns` | Not in converter (G) |
| HBA off with ambient decode (Chemical 0.11a) | manual project patch | Conflicts with `early.rs:500` forcing HBA on |
| God rays off, probe sort, probe diffuse gather, tonemap, SSAO, sky/fog/LUT, gamma, grain, cone volumes, F0=1 diffuse, 0.20e clouds/ambient | runtime hook presets | Not in converter (finding 4); partly expressible as data (god rays 0, box-probe radius, cloud orientation) |
| 0.10b→026g manifest | untracked file on `v0.10b` checkout with +279 uncommitted `src` lines | Radar 0.10b cannot be rebuilt from committed code |

## Fixes on this branch

| Commit | Change |
| --- | --- |
| `72fa0cb` | Review tooling: `examples/dump_config_leaves.rs`, `diff_bundle_db.rs`, `dds_formats.rs` |
| `2e1b6c0` | Omit empty-folder collision instead of swapping kinds |
| `3659fe0` | Keep early `wire_set` objects |
| `2722518` | Import early donor sound stems with native-format options |
| `36aaf76` | Re-encode BC2 textures as BC3 |

Rebuilt with the branch, the 0.11e bundle differs from the candidate only by:
- the four expected fixes;
- the respawn and boiler changes already on the base branch;
- 12 added donor sound files.

The 0.20e bundle differs only in the 196 project: the collision omissions,
plus the base branch's cloud and Mie preservation.
