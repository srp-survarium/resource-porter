# Retail 0.21d maps in 0.26g

For the build and two-client launcher scripts, start with
[Playable maps for 0.26](playable-maps-026.md). This document records the
conversion contracts and playtest evidence.

The importer adds six old maps, each with day, evening and rain variants, to
**0.26g0 build 2777**. It uses the old static render geometry, collision, textures,
placements, probes, spawn points and battery collectors, with 0.26 material
instances and shaders. Native maps remain available. Only the map dictionary
is extended in the native namespace.

## Inputs and build

`ports/survarium-021d-to-026g.json` pins both databases by SHA-256:

| Input | SHA-256 |
| --- | --- |
| Retail 0.21d build 2010 | `e28cc2878e0e2f04ab9ab0a1861034622af487a491bf230ea24f44e9af9715a4` |
| Retail 0.26g0 build 2777 | `c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c` |

```sh
vostok-port build-survarium021-maps \
  --manifest ports/survarium-021d-to-026g.json \
  --donor-db "$DONOR_021_DB" --target-db "$TARGET_026_DB" \
  --output .work/021-to-026/bundle
vostok-port apply --base-db "$TARGET_026_DB" \
  --bundle .work/021-to-026/bundle \
  --output-db .work/021-to-026/resources.db
vostok-port verify --db .work/021-to-026/resources.db \
  --bundle .work/021-to-026/bundle
```

Use an empty bundle output directory. Apply checks the target fingerprint too.
Use this bundle directly: the generic compose command does not propagate its
build fingerprint or map metadata.

## Registration and runtime

| Source level | Day ID | Evening ID | Rain ID |
| --- | ---: | ---: | ---: |
| level_01 | 160 | 161 | 162 |
| level_02 | 163 | 164 | 165 |
| level_03 | 166 | 167 | 168 |
| level_04 | 169 | 170 | 171 |
| level_05 | 172 | 173 | 174 |
| level_06 | 175 | 176 | 177 |

Projects are `projects/v021d_level_XX/<weather>_2/client_project`. Mode options
clone the corresponding native battery mode, changing the project path and
mode name. Existing loading-screen labels are retained.

A manifest map may also set `map_id`, the mode's
`match_making_params.map_id` (the server's map number: unset, the native
level's, so a server counts the import as a weather of the native map; the
0.26g exe never reads this key), and `name`, its own `ui.map_name` key with a
text in each of the six client languages (written into every
`localization/<language>/localization`). The build refuses a number a native
mode uses, one number for two source levels, and a key a localization has.

The loading screens of 0.21d's maps are the same pictures 0.26g shows for the
same level and weather (`LobbyMenu._load_images`, byte-identical bitmaps in both
`lobby_menu.swf`), so the cloned `ui.loading_screen_id` is already the original.
0.21d's loading screen showed only the picture and the map's name; it had no
description, and the clone keeps 0.26's mode description.

### Skhid revision 17: RLS Vostok and Mamayev Kurgan

`ports/survarium-021d-two-maps-to-026g.json`: 166 (level_03) and 175
(level_06), day only, native 0.26 lighting, map numbers 10 and 11, named
`st_level_10_name` / `st_level_11_name` ("RLS Vostok (v0.21)", "Mamayev
Kurgan (v0.21)", each language's own name with "(v0.21)"). Built as a bundle,
converted to a content pack against the stock archive with the released
patches 12 and 14 (`pack from-bundle --base-patch`), and composed with the
equipment packs into `resources.db.patch_17`:

```sh
vostok-port build-survarium021-maps --manifest ports/survarium-021d-two-maps-to-026g.json \
  --donor-db "$DONOR_021_DB" --target-db "$TARGET_026_DB" --output maps-bundle
vostok-port pack from-bundle --base-db "$TARGET_026_DB" --base-patch resources.db.patch_12 \
  --base-patch resources.db.patch_14 --build-base-db "$TARGET_026_DB" \
  --bundle maps-bundle --output packs/maps-021d-1 --id maps-021d --version 1
```

Both projects keep 0.21d's respawn points (22 and 20, half each team's) and
collectors (container 0 is team 1's).

Use the development `match-server-026` and its matching patched 0.26 client;
these are described in `../srp/servers/match-server-026/README.md`. Pass, for
example, `--map 166`. Use the imported database in an isolated client directory. The reviewed older
SRP executable generates a random session ID: omit `--roster` when using it.
The retail archive signature also rejects modified databases. Create the
separate map-testing executable from the reviewed SRP client:

```sh
python3 tools/patch-026-map-resources.py /path/to/survarium-srp.exe \
  /path/to/survarium-ported.exe
```

This hash-pinned, five-byte patch skips the resource hash/RSA verifier call at
VA `0x135d3`. It leaves certificate cleanup, TLS checks and multiplayer state
hashes alone. Its input SHA-256 is
`45baf0a591cf6535a9f7afa073ae59551979d92904670a4e1adf70336c1004e9`;
the output is
`6a35b09ed3f9e48b18e19cdd4bf8ba8a5518eceeb75bba9049ad80150658e4cf`.
Keep the retail `resources/` directory beside `resources.db`, including its
SSL certificate and splash screen. Launch `survarium-ported.exe` with `-localization=english` for English UI.

New IDs are not in SRP's built-in time-limit/results fixtures, so do not enable
`--finish-timed`, `--finish-objective` or `--results` for these IDs until their
fixtures have been extended. This importer does not modify SRP.

## Compatibility decisions

* Static vertex layouts already match: 28-byte static and 32-byte vertex-colour
  meshes, with half-float UVs. Audits decoded 3,717 level surfaces in 0.21 and
  5,484 in 0.26 without invalid positions, UVs or indices. Model payloads are
  retained; no geometry decimation is performed.
* All 56 old physical-material names retain the same IDs in 0.26 (which adds
  six). Collision material IDs therefore need no remapping.
* Materials start from 0.26 templates. Texture bindings and numeric appearance
  parameters are transferred; shader feature switches remain from the template.
  Two reviewed alternatives retain vertex-colour support that the same-named
  0.26 materials lost. A visual match to 0.21 is not guaranteed.
* Scene objects receive missing fields from the matching 0.26 map/weather.
  `render_visuals[].visual_name` is the model resource path, copied from
  `lib_name` before relocation. Newly introduced object groups start empty. Template placements and objective
  objects are not copied into those groups.
* All 33 imported particle effects receive six new properties where required:
  emitter transform/velocity coupling, animated random-velocity multiplier,
  turbulence noise type and cloud/shadow controls for particle lights. Existing
  curves stay intact; newly introduced behaviour switches default off. A native
  crash dump identified the missing `AnimatedVelocityMultiplier` lookup.
* Environment probes receive `<weather>_2` entries pointing to the old probe
  textures, including the implicit `_diffuse` DDS/options companions the engine
  requests. Source assets are namespaced under `ported/v021d`; vegetation keeps
  its project-relative directory hierarchy.
* Short particle, texture, material and sound names are rewritten in their own
  resource family. `fx/lights/oil_lamp` names both an imported material and a
  shared native particle: relocating the latter as though it were the material
  crashes the particle loader. Shared native effects keep their original names.
* Native sound resources use `.raw_options` and `.high.rms` alongside
  `.high.ogg`. Scene `full_name` values are labels, not sound-resource paths;
  model instance `lib_name` values *are* resource references. Composite sounds
  are dependencies too: the old electric-anomaly hit composite is absent from
  0.26, and omitting it crashes its sound loader.
* Stale collision instances use the same donor model's surviving collision or
  walker-collision payload when its requested counterpart is absent. A
  counterpart that is an empty folder node is an authored "no shape" and its
  instance is omitted instead (as for the 0.20e/0.11e day maps). Before the
  archive code moved to skhid-srp's `resources-db` crate, the reader missed
  collision files that share their bytes with another node (0.21d stores
  `level/03/anomal_tree_0X_`'s walker collision as its bullet collision's
  bytes, `barrel_stove_01_fire`'s collision as its `hq_collision`'s): 20
  instances on level_03 (9 anomaly-tree walker instances made bullet ones,
  `soviet_comp_01`'s the other way) and 10 on level_06 (3 trees, 2 barrel
  stoves) had the other kind. Every collision instance of the two maps now
  matches the donor's (1,447 and 864, none changed). Two old
  poplar models (`poplar_04_sux` and `poplar_06_sux`) have no collision payload
  at all: their explicitly reviewed dangling collision instances are omitted.
  Their rendered trees remain. These trees therefore have no added collision.
* Stale material and LOD references to absent donor geometry are pruned and
  recorded. The medical UAZ only ships its low-detail body mesh; it is
  explicitly promoted to a single main LOD. Missing geometry is not invented.
  The level_04 border's single surface is explicitly renamed from
  `anomaly_moss_base_sg` to `donotwalkhere_sg`, matching its actual geometry and
  the repaired 0.26 settings. Unknown material associations remain errors.

## Minimap identity

The project root's `project_name` is the HUD lookup key, not the directory from
which geometry or vegetation loads. Preserve the donor's `level_01` … `level_06`
there; the mode's project path remains `v021d_level_XX/<weather>_2`. Renaming the
HUD key made every imported map fall back to **level_09** in 0.26's
`survarium.controls.Minimap.setProps`.

The minimap images are embedded JPEG/alpha resources in `flash_movies/hud.swf`.
Comparing both HUDs confirms byte-identical images and identical scale/offset
settings for Rudnya (`level_01`), School (`level_02`) and the chemical plant
(`level_04`). School uses size 110, posX 21.079, posY 105.098 in both builds.

The other three embedded minimap images changed between builds. In addition,
the fort's posX changed from -10 to -1.096. They now select their corresponding
0.26 minimap, but faithful 0.21 minimaps for levels 03, 05 and 06 still require
separate HUD image/settings support; the importer does not replace native HUD
assets. This limitation is distinct from the incorrect level_09 fallback.

For level_03 and level_06 the 0.26 HUD keeps 0.21's settings exactly (size 100,
posX 14 / posY -42.196 and posX 30 / posY 109) and redraws the same plan: level_03
has the same outline and buildings with lighter fills; level_06 the same plan,
with a little more area drawn at its right edge and a slightly different
bottom-left border. Their 0.26 minimaps fit the 0.21 geometry.

## Old battery collectors

Both builds use `victory_items_container`, with team, ID, `visual_geometry`
and `collision_geometries` fields. The model changed from
`item/quest/container` to `item/storage_device/box_atlant12`. The importer keeps
the old model, its placement and its authored interaction-zone references,
and verifies that the two collectors belong to opposing teams and their zones
exist. The old collector is a static model, with no skeleton/animation to port.

A collector's own `position` is not where it is. 0.21d level_06 has both at
(30.04, 15.433, 82.267) and (32.398, 12.144, 85.445), in the fire pit 45-56 m
from their models and zones, and native 0.26 level_06 the same (raised 7.28 m);
native level_02, 04 and 05 have theirs 13-152 m off too. In 0.26g nothing of
the container reads it: its cook takes `visual_geometry`'s position, rotation
and model (and the model's battery locators, 0x232749..0x232791), its load
`collision_geometries`, team and id (0x2F81F0, 0x31CAD0), its transform is its
zone's (`usable_object::get_transform` 0x2F8440, vtable +44h of both container
vtables), the minimap's container marks come from that transform
(`gather_victory_items_rule::fill_match_hud_state` 0x3140C0), and a deposit is
a 2 m ray onto the zone. The importer keeps the donor's values.

0.26 spawners add a nested `spawned_item` transform. The importer derives this
from each old spawner's own transform. Copying a template point would put
batteries somewhere else in the map.

Schema compatibility is evidence for feasibility, not proof of interaction.
A physical battery pickup and deposit in the old collector is the acceptance
check; synthetic deposit events only test the game rule and cannot establish
that the collector's zone/model works.

## How much changed

The databases contain 12,557 / 52,660 / 78,903 resource files in 0.100b / 0.21d /
0.26g. For 0.21d to 0.26g, the main map-related families compare as follows:

| Family | Identical bytes at same path | Changed at same path | Old-only paths | New-only paths |
| --- | ---: | ---: | ---: | ---: |
| models | 10,722 | 4,218 | 2,009 | 11,563 |
| textures | 2,334 | 976 | 1,236 | 15,566 |
| material_instances | 390 | 1,506 | 29 | 445 |
| particles | 0 | 54 | 40 | 34 |
| projects | 0 | 63 | 36 | 92 |
| sounds | 4,205 | 401 | 3,167 | 9,832 |
| gameplay | 1 | 109 | 14 | 398 |

These count resource files, not individual objects or engine changes. A model
is a directory of resources, and a renamed file appears in both the old-only
and new-only columns. Shader paths encode permutation keys, so their more
useful comparison is by shader name and bytecode below.

## Shader and cloud investigation

The retail shader inventories contain 261 / 288 / 286 shader names and
3,402 / 19,236 / 14,745 compiled permutations in 0.100b / 0.21d / 0.26g.
Between 0.21 and 0.26, 134 shared shader names have identical bytecode payload
sets, another 44 have the same instruction sets, and 90 have changed instruction
sets; 18 names were added and 20 removed. These are comparisons of compiled
variant sets, not counts of source-code edits. Shader key masks also differ.
Replacing the whole shader database is therefore not a compatibility solution.
Selective reuse of old compiled shaders is plausible where input signatures,
constant buffers, texture/sampler slots and render-stage contracts agree. The
engine would also need the right permutation-key mapping. No such shader-loader
adapter is implemented here; using 0.26 materials and shaders already lets the
old map geometry render.

The 0.26 cloud shaders sample cloud textures and atmospheric scattering data.
The environment adds cloud distance, cloud normal texture, sunlight/skylight
controls and an optional cloud-lighting switch. The importer preserves old
shared environment values and supplies new fields from the corresponding
0.26 environment. This uses the new cloud renderer; it does not reconstruct
the old appearance exactly.

0.100b is a separate conversion task: its static geometry and project schema
predate the matching 0.21/0.26 layouts. This importer deliberately accepts only
the pinned 0.21d and 0.26g databases.

## Validation

`cargo test`: 270 tests passed. The bundle contains 18 variants and 20,629
resources, including 813 adapted materials and seven repaired model settings.
An independent donor comparison verified byte identity for all 5,270 render
payloads, 824 collision vertex/index pairs, 661 face-material payloads and 60
vegetation files (763,190,873 bytes in total). A final-project audit checked
all 36 collectors and their zones, 684 battery spawn transforms and 180 vegetation layers
across the 18 variants; every placed particle and collision resource resolves. Most sound assets and the animated anomaly model remain
shared with 0.26; missing donor sounds are imported.

All six day maps (IDs 160, 163, 166, 169, 172, 175) and level_03/rain (168)
loaded in the native 0.26 client, reached gameplay and produced seven verified simulation checkpoints
each in 40-second sessions on the pre-minimap-fix database (bundle-v10). No access violations occurred.
School/rain (165) was subsequently played with two clients after the minimap
fix. The other ten weather variants are built and audited, but not client-tested.
An earlier 100-second level_03/day session also checked movement; its screenshots
are preserved in `.work/021-to-026/check-166-previous/`. Smoke-test screenshots and
logs are under `.work/021-to-026/check-<id>/`, with machine-readable results in
`runtime-audit.json`. These are short smoke tests, not full-map playthroughs.
Physical battery pickup/deposit in the old collectors remains untested.

Known remaining visual warnings: missing `wood_planks_04_spec`,
`concrete_greenwall_spec`, `metal_column_nmap` and `debrisstone_bump` textures;
`nomaterial` on several border/collision/light surfaces; and an arsenal LOD
material on level_03. These did not prevent gameplay. All four missing textures are absent from both retail inputs; no substitute
appearance is invented. Lighting/material parity with 0.21 has not been validated.

The initial release was bundle-v10, SHA-256
`96b4626e271be54cd99691a18d3ffb76066368eccbd11530b0e206a066e5c46e`.
The minimap fix rebuilds all maps as bundle-v11. An independent comparison
confirmed that only the 18 root `project_name` values changed: every other
project field and all other emitted resource hashes are identical. School/rain
also has user confirmation of two-client gameplay before this HUD fix.

Current packed database after the HUD-key fix: `.work/021-to-026/resources.db`,
8,979,925,927 bytes, SHA-256
`68586425ae4e18d6659d127a5e091c256836f7fe92f4510f2d2544a7dff52b68`.
Built from bundle-v11; apply reopened it and verified all 20,629 resource hashes.
The previous database is retained as `resources-before-minimap.db`.

After the fix, two native clients loaded School/rain (165), started the match
in English and produced verified checkpoints. The displayed minimap was
visually checked against the original School bitmap; it now shows School's
courtyard and grounds. Screenshot:
`.work/021-to-026/minimap/after-english.png`.

## Native 0.26g School with dense volume fog

This separate experiment uses retail School/day (map ID 67), with its geometry,
gameplay objects and minimap preserved. The builder replaces the environment's
properties with School/rain's overcast settings, disables rain and the sun disc,
reduces atmospheric visibility, and adds Rudnya's native `volume_fog` object.
The fog box spans x=-138.921..181.079, y=-60..120, z=-54.902..265.098,
covering the School arena. Its density is 0.06, with a 0.01 height falloff and
0.85 transparency multiplier. Animated noise is disabled (`noise_scale = 0`):
the user confirmed this fixes extra whitening during iron-sight aiming while
retaining dense volume fog. This uses 0.26g's existing volume-fog shader.

```sh
vostok-port build-survarium026-fog --target-db "$RETAIL_026_DB" \
  --output .work/026-dense-fog/volume-bundle-v3
vostok-port apply --base-db "$RETAIL_026_DB" \
  --bundle .work/026-dense-fog/volume-bundle-v3 \
  --output-db .work/026-dense-fog/resources-fog-final.db
```

The builder checks the pinned retail hash, compares the project after undoing
the two intentional scene edits, and roundtrips the serialized config. `apply`
regenerates archive checksums and verifies the packed project. Directly changing
bytes in retail left a stale per-resource CRC and produced the client's
“resource archive is corrupted” error; always use the packer.

The initial `resources-packed.db` experiment changed only distance haze and
retained the sunny sky. It was rejected visually and is superseded by the volume
variant. The imported 0.21 maps database is separate from both experiments.

The earlier noisy volume variant, `.work/026-dense-fog/resources-volume.db`,
has SHA-256:
`2defc500ca9628fea678d5f2b5a2d7ae4c815a11cfb7477aa52d36636802d8d1`.
The confirmed fixed database is `.work/026-dense-fog/resources-no-noise.db`,
SHA-256 `b5c6636ceb5f2c5556a95a8c61a1b344125b260979f907084888c8ef6537f563`.
It was produced by applying `no-noise-bundle` to the noisy volume variant;
the builder now emits the same final project directly from retail.
Rebuilt `volume-bundle-v3` and compared its project byte-for-byte with the
playtested `no-noise-bundle` project: identical.
The test client's `resources.db` symlink selects this fixed database. Its
launch profile remains native School/day (67). Both test clients use English.

Runtime checked: both clients loaded the volume variant, started the match and
produced verified checkpoints. Screenshots `volume-final.png` and
`volume-peer.png` under `.work/026-dense-fog/` show dense fog across the two
spawn interiors. Full outdoor traversal and fog motion have not been audited.

Investigation: the user reported extra whitening when aiming with iron sights.
Magnified scopes have not been tested. Disabling bloom and depth of field did
not resolve it. Neither did replacing the eye-adaptation shader with a fixed
average luminance, nor fixing all three luminance channels. Those shader edits
are diagnostic artifacts only and are not part of the builder or baseline.
Removing distance haze while retaining the volume also left a visible change
in an automated aiming comparison; its cause has not been established.

The isolation variant `volume-off-bundle` changes only the added volume
object's `transparency_multiplier` from 0.85 to zero. It applies to the baseline
database, restoring all native shaders and retaining atmospheric haze, the
overcast sky and native post-processing. This is a diagnostic, not a fix or a
replacement for the requested dense fog. Pack it through `apply`, as above,
into `resources-volume-off.db` before selecting it in the launcher.
The user confirmed that iron-sight aiming has no problems with this variant.
Its database SHA-256 is
`b76e0956ad5b3f20fb5624b575e090529e9365d0a58cf5ef17313a26c2fc9907`.
This implicates the added volume's rendering; it does not yet identify the
faulty calculation.

The successful variant, `no-noise-bundle`, restores the noisy volume and changes
only its `noise_scale` from 0.8 to zero. Density, height falloff and transparency
remain unchanged. The native pixel shader uses a reconstructed world position
for its noise lookup, separately from its fog-path integration. Disabling noise
isolates those paths while retaining volume fog. The user confirmed the fix.
No shader replacement is used. The exact engine/shader calculation responsible
has not been established; this is a verified configuration workaround, with
animated fog noise omitted.
