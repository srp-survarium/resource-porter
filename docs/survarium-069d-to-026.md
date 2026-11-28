# Survarium 0.69d maps in 0.26g (Duga)

The first port *down* from a build newer than 0.26: 0.69d0 (2022) level_11,
"Duga radar station", becomes 0.26g map **205**, mode 4 (team deathmatch),
on the 0.69d TDM spawns.

```sh
vostok-port build-survarium-069d-map \
  --manifest ports/survarium-069d-duga-to-026g.json \
  --donor-db donor-069d.split --target-db "$TARGET_026_DB" --output "$BUNDLE"
vostok-port apply --base-db "$TARGET_026_DB" --bundle "$BUNDLE" \
  --work-dir "$WORK" --output-db "$DB0"
# Loading screen and minimap (Flash): needs ImageMagick.
tools/playable-map-pack/build_069d_ui.py --porter vostok-port --magick magick \
  --target-db "$TARGET_026_DB" --donor-db donor-069d.split \
  --donor-mode gameplay/game_modes/level_11_day_tdm.options --output "$UI"
vostok-port apply --base-db "$DB0" --bundle "$UI" --work-dir "$WORK" --output-db "$DB"
tools/playable-map-pack/build_duga_package.py "$PACKAGE" "$DB" "$BUNDLE" \
  "$SOC_DM_PACKAGE" "$VALIDATION"
```

The donor volumes needed in full are listed by `examples/split_closure.rs`;
32 of the 76 are extracted (the UI images and localization live in 32, 49,
54, 60 and 70).

## What 0.69d changed underneath

**76 patch volumes.** `resources.00.db` .. `resources.75.db`, mounted in
numeric order: a later volume overrides a path, and an *erased* node in a
later volume removes it (patch volumes are mostly erased nodes; `53` is 293
erasures and one file). `ResourceDb::open` accepts a `.split` text file
listing the volumes in mount order. A volume may be only its FAT head (header
and node buffer, a few hundred KiB, read from the archive without inflating
the payloads): it still indexes, and reading a payload from it fails with a
request to extract that volume. `split-fingerprint` hashes the heads, which
pin every payload's offset, size and stored hash wherever the volumes live.
An erased node may name a folder: 749 of 9044 erasures remove a whole
`.model` folder before a later volume re-exports it, so an erasure removes
everything below the path too (otherwise stale surfaces of old exports
leak into the newer model).
Duga needs 27 of the 76 volumes in full.

**16-byte config records.** Every binary config (projects, settings,
materials, texture options) packs a value into 16 bytes instead of 24:

```
u64 bits  0..44  data: inline scalar (integers sign-extended), or byte offset
          44..48 kind: the same 0..8 codes (bool, int, float, named, indexed,
                 string, float2..4)
          48..64 count: children of a table, or byte size of the payload
u64 bits  0..48  byte offset of the key (named children)
          48..64 16-bit key hash (not needed to read)
```

The root table's children start at byte 16, which no 24-byte config can
have, so `BinaryConfig::parse` tells them apart; writing always produces the
24-byte layout. (`dump-db-config` failed with "type 12288": that is kind 3 in
the high nibble of the u16 a 24-byte reader expects.)

**Tangent basis.** Static vertices keep the 0.26 layout (float3 position,
12-byte basis, half2 UV, optional colour) except the basis: 0.26 packs each
vector as bytes `z, y, x, alpha`; 0.69d stores each as one float equal to
`(x << 16 | y << 8 | z) / 2^24`, the same three bytes carried exactly by the
mantissa. Verified on `tiled/metal/metal_floor_03` shipped by both builds
(positions and UVs byte-identical). 0.69d stores no alpha; the canonical 0.26
static values (normal 127, tangent/binormal 255) are written. Chunks 110 and
114 (mesh statistics) are dropped. Collision payloads are unchanged.

## Conversion choices

* **Materials** are template-first: each 0.69d material is categorized
  (opaque, two-sided, vertex colour, vertex blend, alpha test, foliage,
  grass, glass, mesh decal, projected decal, water, terrain) and drawn with a
  native 0.26 template of that category; only texture names and constants of
  the same meaning change, so every material requests a shipped shader
  permutation. 0.69d `t_albedo`/`t_normal` map directly; smoothness feeds
  0.26's `t_roughness` channel (which the gbuffer pass reads as gloss) at
  `roughness_scale` 0.2, matching native value ranges. Metalness, parallax,
  displacement, fuzziness and detail maps are not reproduced. A vertex-blend
  material is told before an alpha-tested one (0.69d's
  `alpha_test_consider_vertex_blend` lets the blended-in opaque layer close the
  base's cut-outs: house_6's plaster over laths), and vertex-colour categories
  keep `two_sided` (the templates `vertex_colour_two_sided`,
  `vertex_blend_two_sided`, `vertex_blend_alpha_test[_two_sided]`; a manifest
  without them falls back with a note).
  `verify_shader_variants.py --templates` proves each new material against
  the template it was built from.
* **Terrain** (0.69d layered terrain, five layers and a 2048² blend mask)
  becomes per-segment two-layer vertex blends: the segment's two dominant
  layers on a native vertex-blend template, the blend weight per vertex from
  the mask, stored in a new vertex colour (stride 28 -> 32).
* **Scene**: render and collision instances keep their transforms
  (`model_name` -> `lib_name`). The environment, sky and post-process are the
  native template's (level_03 day TDM), lit from the 0.69d sun direction.
  Duga keeps native probes (`"probes": {"native": ...}`): one native global
  probe covers the map and native indoor cubemaps fill the 0.69d tunnel and
  building probe volumes. Rudnya converts the donor's own (see Rudnya). 0.69d ambient lights
  (photometric, shapes 0.26 lacks), sound environments, anomalies, artefacts,
  BR/CTP objects and winner points are dropped. Neutral resupply zones become
  one zone per team. Native ambient sound emitters come from level_07.
* **Models** identical in 0.26 (same collision, render bounds within 5 cm)
  stay native; the rest are imported under `ported/v069d`.
* **Project schema** (three client crashes on match setup, found by fixed-
  camera loads and their minidumps labelled with `tools/re-kit/v026g`):
  - The project carries every container group of the native template
    (`victory_item_spawn_points`, `temporary_render_visuals`,
    `temporary_collision_objects`), empty, as every working port does.
    Without them the client crashed 20 ms after match setup
    (ACCESS_VIOLATION, EIP "thir"); with them it got further.
  - Vegetation layers keep `wind_factor` (0.26 reads it too) and take any
    other native layer key the donor lacks from the template: without it
    `grass_cook::translate_query` crashed 140 ms after match setup.
  - Merged surfaces stay native-sized (at most 32 000 vertices, 900 kB of
    vertex data, 96 000 indices; native surfaces stop at 35 001 / 980 084 /
    103 935, `examples/surface_sizes`): `static_render_surface::load` takes
    a surface's buffers on the main thread's stack and overflowed it with a
    1.2 MB merged surface.
* **Name, loading screen, minimap**: `mode.ui.map_name` is the donor's
  (`st_level_11_name`); its text from each 0.69d localization (english,
  german, original, polish, russian, ukrainian) is added to 0.26's. 0.26 has
  no data-driven loading screens or minimaps, both are Flash:
  `LobbyMenu._load_images` lists 26 bitmap classes indexed by
  `ui.loading_screen_id`, and `Minimap` maps HUD keys (`project_name`) to
  bitmap classes and props, sending unknown keys to `level_09`.
  `build_069d_ui.py` puts the donor's loading image
  (`flash_movies/textures/ui_loading/level_11_day.png`, centre-cropped to
  1920x798) into slot 24 (Level8_night; only level_08 night modes name it,
  and they have no project), and the donor minimap (`ui_hud_maps/
  map_11_minimap.png`) into Map_09 (no 0.26 map is level_09); the project
  keeps the donor key `level_11`. 0.69d places its minimap by
  `ui.minimap_props` (centre x/z `position` -4/-2, half extents x/z `scale`
  120/148, -z downwards, checked against the scene); 0.26 uses one half
  extent `size` and centre posX/posY, so the image is resampled into a
  296 m square and the level_09 props in Minimap's constructor become
  size 148, posX -4, posY -2, rewritten in place (same code length, a
  double constant used only there).
* **Performance**. Static geometry is the hard budget: 0.26 maps load
  25-54 MiB of render geometry (`examples/geometry_bytes`), and the client's
  static vertex pool holds 88 MB (95 MiB of vertices logged "No enough memory
  in vertices pool" 1 669 times and left holes); the raw port had 310 MiB.
  In order:
  - LOD0 and LOD-less imported surfaces are simplified (meshoptimizer,
    error <= 0.4% of the mesh extent, target 30%): 3.86 M -> ~1.8 M
    triangles. Authored LOD1/LOD2 are simplified further (error 4%/6%; both
    may collapse across UV seams and drop small parts), heavy LOD-less
    models get generated LODs, imported textures above 1024 (smoothness maps
    above 512) lose their top mip.
  - Terrain render meshes are simplified with locked borders (tiles stay
    sealed; error <= 0.4% of a tile, about 14 cm; 976 k -> 50 k triangles).
  - Imported models that are translated copies of another (the Duga antenna
    sections, exported as separate models at their own offsets; re-exported
    wires may differ by a few vertices, so surfaces match exactly or by
    vertex count within 0.2% and bounds within 1 cm) draw the first copy,
    moved by the offset.
  - **Material clusters** merge render instances into render-only models
    with one surface per material and LOD: first models alike (same LOD
    switching, same material set: copies of a model, or the native elka
    trees sharing `flora/elka`) per 32 m cell, then the remaining small
    imported instances (radius <= 10 m) across models per 12 m cell; glass,
    water, terrain and projected decals stay alone, foliage and native
    models merge only with models alike, and models above 64 KiB of
    geometry stay instanced (baking copies multiplies memory: a 128 KiB cap
    saves 320 more draws for 35 MiB more geometry). 0.26 picks a model's LOD
    by the distance to its bounding-box centre (`select_lod`), so each
    cluster switches later by its centre-to-member reach: no member is drawn
    coarser than it would be alone. Collision instances are unchanged.
  - Imported models draw LOD0 where they drew LOD1, and LOD1 geometry is
    not imported (`lod1_shares_lod0`; clusters share it too): it was a third
    of the geometry. They switch to the far LOD at 30 m.
  - Every surface's `export_properties` follows its mesh as emitted: `type`
    2 for 32-byte (vertex colour) vertices, `source` counts. Blended terrain
    gains a colour and kept type 0: 0.26 read it with the 28-byte layout and
    drew exploded terrain over the map (`examples/vertex_sanity` checks
    both).

Static budget at the 42 spawns (`examples/view_budget`, omnidirectional) and
render geometry (`examples/geometry_bytes`):

| | mean tris | worst tris | mean draws | worst draws | geometry |
|---|---|---|---|---|---|
| level_03 day_4 | 634 026 | 680 149 | 1 081 | 1 105 | 46.6 MiB |
| level_07 day_4 | 790 954 | 905 423 | 1 457 | 1 606 | 48.1 MiB |
| Duga, first build (b9) | 988 080 | 1 034 572 | 1 936 | 1 945 | |
| Duga, clusters without memory cap | 814 014 | 965 991 | 1 717 | 1 724 | 310.5 MiB |
| Duga | 663 831 | 741 901 | 2 330 | 2 357 | 81.2 MiB (69.4 vertex) |

Texture payload 651 MiB (level_03 638, level_07 720); particles 0. Draws are
~45% above level_07's worst spawn and geometry 1.7 times a native map's:
Duga's buildings use many more materials and polygons than 0.26's. A
fixed-camera load renders the map without pool errors and starts the match.

## After the first playtest

The owner played the package: distant trees, the antenna lattice and rubble
showed rainbow pixel noise, and dark cave mouths showed bright flat slivers.

* **Texture headers.** Retail 0.26 textures have flags 0xA100F and
  `dwPitchOrLinearSize` = every byte after the header (mips plus the
  trailing colour-space byte); 0.26's streamer locates the mips from that
  field (found and proven on branch `soc-mp-pool-026`). 0.69d's own files
  follow it, but halving a texture above 1024 wrote the top mip's size
  there: 349 of 792 textures, the anomalous-tree branches among them, read
  their mip tail one byte early. `drop_top_mip` writes the native pitch,
  every emitted DDS goes through `canonical_dds` (checks header + mip chain
  + 1, writes the native flags and pitch), and `examples/dds_headers` checks
  a database (792 of 792 now).
* **Cave mouths.** The slivers were sky seen through cracks: LOD0
  simplification moved open edges where cave meshes meet the terrain and
  other models. LOD0 and LOD-less surfaces now keep their open edges
  (`lock_borders`), and far LODs no longer collapse across UV seams
  (`lod2_permissive` off; it smeared distant walls into streaks). The cave
  water (0.69d `water_l11`, smoothness 0) was drawn with 0.26's water, which
  reflects a sky probe where screen-space reflection finds nothing: matte
  donor water now reflects the native indoor probe (`matte_water_probe`)
  with local reflections off, and every water takes the 0.69d scattering
  colour as its fog colour.
* Duga has no light-shaft, darkening or portal materials: of its 325
  materials 287 are g-stage, 18 decals, four add a forward glow
  (stalactites, bonfire, lamp emitters; converted to g-stage templates) and
  two are water. No template adds a render stage its source lacks.

Budget after the fixes: 869 574 / 944 246 triangles, 2 332 / 2 359 draws,
92.3 MiB geometry (79.2 MiB vertices of the 88 MB pool). Before/after
fixed-camera shots: `validation/gallery/index.html` in the package.

## Ladders

The 0.69d "ladders" (`generic/silo/*ladder*`, `props/ladder/*`) are stairs:
their walker collision rises at 10-36 degrees (with instance scale), well
inside the 0.26 controller's 60 degree limit and 0.6 m step. They walk the
same in 0.26 and need no ramp. 0.26 has no climbing state (0.69d has none
either: no ladder object type, every "ladder" model carries its own walker
ramp). The watchtowers' walker collision is a stair to their platforms
(faces up to 58 degrees, under the controller's 60). `examples/
steep_walker.rs` lists every narrow walker face steeper than 55 degrees that
rises over 2 m; in the TDM scene these are tree trunks, fence posts,
pylon and antenna legs (3 420 faces, none on a ladder model), not climb
routes, so no ramp or plank was added.

## Lobby scenes (0.46c2 hangar, 0.69d tent camp)

The same builder makes a lobby project when the manifest has `lobby` in
place of `map` (`build-survarium-later-scene` is an alias of the command):

```sh
vostok-port build-survarium-later-scene \
  --manifest ports/survarium-046c2-lobby-hangar-to-026g.json \
  --donor-db v046c2.split --target-db "$TARGET_026_DB" --output "$BUNDLE"
vostok-port build-survarium-later-scene \
  --manifest ports/survarium-069d-lobby-camp-to-026g.json \
  --donor-db donor-069d.split --target-db "$TARGET_026_DB" --output "$BUNDLE"
vostok-port pack from-bundle --base-db "$TARGET_026_DB" --base-patch ... \
  --bundle "$BUNDLE" --output <id>-<version> --id <id> --version <version>
```

* A lobby is a client project only (`projects/lobby_scene/<name>/client_project`):
  no server objects, mode, dictionary row or localization. Its templates are
  the 0.26 lobby's (`projects/lobby_scene/day`). Both donors put the lobby's
  character where 0.26 does (`gameplay/lobby_menu.options`: `local_player` at
  the origin, rotation 180), so the scene keeps the donor's coordinates.
* Vegetation instance files go to `projects/lobby_scene/vegetation/<vegetation_folder>/`:
  every lobby shares the level directory and the donors name theirs alike.
* `namespace` (`ported/v046c2_lobby`, `ported/v069d_lobby`) keeps a lobby's
  imported files apart from a map port of the same build.
* **0.46c2** has 0.69d's 16-byte configs but 0.26's vertex basis (the same
  mesh is byte-identical) and material vocabulary (`texture_diffuse`,
  `use_nmap`, `texture_roughness` as gloss): `geometry` keeps the basis and
  drops chunks 110/113, `vocabulary_046` adds the 0.69d names so materials
  convert through the same templates (`roughness_scale` 1), a LOD's surface
  table becomes a list (`lod_lists`), and an empty game material with id 0
  becomes 0.26's 65535. Its lights are 0.26 intensities; its probes are
  0.26's format (a cubemap and its `_diffuse`) and are imported
  (`"probes": "donor"`). Wire sets are the same object and are kept.
* **0.69d** (`"probes": "donor_light"`): the camp is lit as a donor-lit map is (Rudnya
  below, `Lighting::Donor`): every probe with its own cubemap re-encoded and its SH
  irradiance, the largest also as the map-wide sphere (a lobby's reaches 1 km, its
  horizon), every ambient light as a 0.26 ambient light, the sun, 0.69d's exposure and
  curve (the lighting pack's exposure profile, marker 33100) and materials by the two
  builds' shading, all in one scale. Its environment's exposure is fixed (minimum and
  maximum EV 9.5), so its profile's floor is its ceiling; it has no colour grading, no
  `sun_tint` and `sky_intensity` 1. Two rules only a lobby meets so far:
  * point and spot lights (`lighting::PointLight`, 0.69d's `deferred_lighting.ps` and the
    constants of 0x4427a0): `c * luminous_power * 1000 / D * (1 - (d/r)^16)^2 / d^2`,
    `D` 4 pi^2 for a point and pi^2 for a spot (not spread over its cone), times
    `diffuse_influence_factor`; `intensity` and `attenuation_power` are not read. 0.26 has
    no 1/d^2: the light keeps its range and takes the intensity and attenuation power
    closest to that falloff in ratio, by volume over 0.1 to 0.9 of the range;
  * emission (`donor_emission`; 0.69d's is the forward effect's, constants at 0x53d758):
    `t_emission * colour^2.2 * 2^(EV_m - 3)`, `EV_m` the material's `exposure_value` moved
    towards the scene's by `autoexposure_factor`, an additive one times its alpha; 0.26
    adds `constant_emissive * multiplier * texture` unlit, so the colour goes linear and
    the multiplier into the scene's scale (the notebook's screen: 0.295).

  `exclude_models` leaves out the New Year snow of volume 72's project;
  `native_materials` draws its window panes (`water/glass_water`) with 0.26's
  `tiled/window/glass`.
* `reuse_native_textures` (both manifests, as Rudnya's): a donor texture whose 0.26g
  namesake is the same image (`native_same_image`) is drawn from 0.26g's file, not
  imported. The hangar: 72 of 131 (13 byte for byte, 59 decode alike), 123.8 MiB; the
  camp: 54 of 220 (50 of them re-encodes about 1 level apart), 59.3 MiB.
* What skhid-srp's lobby choice (client patch 81, `srp-026g/docs/notes/lobby-scenes.md` there)
  takes from a lobby project: the 0.26 lobby's camera `track` (copied; a project without one
  is refused), the root `camera`, and the root table `srp_lobby` with `weapon` (where the
  weapon floats in its upgrade window) and `weapon_view` (the camera's yaw there, 0.26
  convention: forward = (-sin yaw, 0, cos yaw)), both from the manifest. The lobby camera's
  distance is cut by a ray against the project's static collision; `camera_walls` adds
  invisible boxes (0.26's unit primitive `editor/walker_collision`, scaled) where the walls'
  own collision is open (the camp's windows and doorway), over the floor and under the
  ceiling (the eye stops 0.15 m short of them, never on them) and around the props the eye
  would get into (the camp's counters, pillows, cylinder, chandelier...): the boxes the
  lobby audit of 2026-10-30 found (skhid-srp's lobby-scenes.md).
* `native_props`: 0.26's own models placed in the lobby frame, `{lib, position, yaw}` (a
  render visual and its collision, cgroup 8 / cmask 16, which the camera's ray meets): the
  hangar has no table, so the stock `props/furniture/workbench_02` stands against its
  sandbags under the weapon's spot.
* The view out of the windows (the 0.26 lobby camera orbits all the way round; 0.69d's was
  fixed). The camp's room is a floor 22.7 m over the donor's own background (level 02's
  apartment blocks `bg_house_1/2`, `bg_shop`, its billboard trees, a terrain patch):
  * A lobby's box probes (`"probes": "donor"`, the hangar) get `radius` 0.75 x their largest half-extent. 0.26g draws the
    visible probes largest radius first, each over the ones before (stage_visibility's sort),
    and equal radii in its probe tree's order, which changes from run to run with the order
    the cubemaps load in. With every probe at the template's 15.3, the camp's outdoor box drew
    after the room's probes in about half the runs: a bluish haze over the room's shadows.
    0.10b's engine set this radius itself; 0.26g takes the project's, and a box's radius is
    only that sort key (its volume is its scale). So the outdoor box draws first and the
    room's probes, smaller, over it. A box whose radius ties the one before it (equal largest
    half-extents: two of the hangar's) goes just below it, so the donor's order decides.
  * `environment_overrides` gives the camp level 02 day's outdoor fog (rayleigh 15..300 m,
    mie 40..1200 m): the lobby template's (rayleigh from 1 m to 185 m) is an interior's.
  * `native_scenery`: 0.26's own models drawn outside, `{lib, position, yaw, scale}`, as
    render visuals only (no collision, so the camera's ray never meets them): the camp's
    overgrown yard (level 02's poplars, birches, willows, fruit and dead trees, bushes, the
    yard trees scaled 1.1-2.1 so their crowns reach the room's windows), level 04's tree
    belts past the blocks, and level 02's own background terrain and tree rings (as the
    school map has them, one transform for the four) for the horizon.

## Rudnya (0.69d level_01)

0.69d's Rudnya becomes 0.26g map **204** (mode 4, day), level number 14, project
`v069d_level_01/day_4` (`ports/survarium-069d-rudnya-to-026g.json`). 0.26g already has a
Rudnya (`level_01`) in the same coordinates (the team bases match within a few metres), so
the port takes that map as its template: native Rudnya's sky and fog (see below what is
0.69d's own). The HUD key stays `level_01`, so native Rudnya's minimap draws it
(same layout).

**Lit as 0.69d lights it** (`"probes": {"donor": ...}`, `survarium_026/lighting.rs`;
the formulas come from both builds' shader disassembly):

* One scale: 0.26 linear values are 0.69d's exposed values at its outdoor exposure, the auto
  exposure's ceiling (`max_auto_exposure_value` 12.5, x 1/(1.2 * 2^12.5)), where a sunlit view
  sits. At that scale the sky 0.26's own atmosphere draws and 0.69d's probes agree within a
  few percent, and 0.26's eye adaptation and curve show sunlit and shaded grey as 0.69d's
  curve does.
* Sun: 0.69d's `sun_intensity_r2 / pi * solar irradiance * T`, T its atmosphere's
  transmittance (Rayleigh x `air_density`, Mie x `air_turbidity`, ozone) at the sun's
  elevation: for Rudnya colour (1, 0.758, 0.535), intensity 4.88 (native 2). 0.26 lights a
  white surface with `sun_color^2.2 * sun_intensity * N.L` (no 1/pi, as 0.69d): its environment
  loader raises `sun_color` to 2.2 (0x34e447 of the 0.26g exe, through 0x1bfcb0), so the scene
  stores the colour to 1/2.2 (Rudnya's (1, 0.882, 0.753)). Written linear, the sun drew at
  (1, 0.544, 0.253) of its intensity in the game: 0.78 of its light and red.
* Probes: all 69 donor volumes with their own cubemaps, each volume with its full
  rotation (the church's floor lights lie on their side, the bus stop's probe turns
  about every axis) and its diffuse light times its `multiplier` (0.69d's
  `environment_probe.ps` scales the SH, not the reflections, by it: the church x2.5 to
  x4, the tractor station's atp(1) x10). 0.69d stores `rgb = s/m`,
  `a = m/(1+m)` of `s = sqrt(L)` in capture units (scene = capture x 1.2 * 2^capture EV);
  0.26 reads `L = 0.25 * (rgb/a)^2` (RGBD, written `e = 4L`, `m = max(1, e)`,
  `rgb = sqrt(e/m)`, `a = sqrt(1/m)`). Every mip is re-encoded (both builds prefilter with
  GGX). 0.26's `_diffuse` cube (16 texels, 3 mips) is evaluated from the probe's SH as
  0.69d's `environment_probe.ps` does (E/pi with `directional_factor`). `specular_multiplier`
  -1 (off) becomes 0. 0.26 has no sky light: the donor's largest probe's cube is also the
  map-wide sphere, on the template's own (native Rudnya's).
* Probe tiers: 0.69d composes its probes per tier (`layer`: 0 global, 1 local, 2 detail),
  `sum(w L) / max(W, 1)` in a tier and the global tier only where `1 - W` of the local one
  remains; 0.26 lerps each probe over the ones before it, so between two abutting local probes
  the outdoor light of the global ones showed through (the church nave's pale line: 0.69d
  83 -> 87-94 -> 100, the port 83 -> 160-293 -> 100). A local probe (layer 1) is placed for skhid's
  exe patch 203, which draws a 0.69d scene's local probes as 0.69d's tier with three-maps-021-lighting's
  `environment_probe_tier_*` programs (@16): a box (a cylinder in its circumscribed box), no smart
  attenuation, its radius `-(shape + 0.25 + 0.5 / (1 + R))` (`local_probe_radius`: below 0 the
  mark, shape 1 a box and 2 a cylinder for the programs, the fraction keeping 0.26's order by the
  sort radius R among them). The build stops on a local sphere, a local probe whose
  `attenuation_power` is not 1 (the programs draw 0.69d's linear falloff) and a detail-tier
  probe. Global probes stay 0.26's own draw. Rudnya: 62 local (2 cylinders, the church's
  belfry and tower), 7 global and the map-wide sphere. A lobby's probes are not marked (its
  builder places them as 0.26 draws them). The map's pack ships with the exe patch and the
  lighting pack: an exe without patch 203 would draw the local probes by 0.26's lerp after every
  global one, with the box programs' smoothstep.
* Ambient lights are 0.26's own ambient lights, all 288 (`lighting::NativeLight`). 0.26 draws
  one with an intensity above 1 as adding `colour^2.2 * (intensity - 1)` times its falloff to
  the diffuse light, and one at or below 1 as multiplying the light under it
  (skhid-srp `renderer-026g.md` 5.4); an earlier converter took them for darkening only and
  baked 0.69d's adding lights into probe volumes of one value each, whose borders showed on
  walls and floors. Now: 0.69d's 244 adding lights (lux; `illuminance * 1000 / pi *
  colour^2.2` off a white surface) add that light in the scale; its 10 darkening ones
  multiply by their factor; its 34 brightening ones (a multiplier above 1, which a UNORM
  target cannot hold) add the share they brighten by of the probe light under their centre
  (0.69d draws its multiplying lights before its adding ones, so they multiply the probes
  alone). Shapes: a sphere's radius is its largest scale (the exe, 0x20029b9b0; `radius` is
  not read); 0.26 fades a sphere by `(1 - (d / 0.98 R)^2)^p` where 0.69d fades from
  `outer - side_width` to `outer`, so the radius ends where 0.69d's light ends and the power
  is the closest by volume (`sphere_fit`; 0.26 has no flat middle, a light with one is
  followed within 0.33: exact needs 0.69d's own falloff in a guarded `ambient_light.ps`). A
  box keeps 0.69d's own falloff, which the lighting pack's `ambient_light.ps` draws (0.26's
  box program fades by a smoothstep and has no power): per axis `(1 - t)^p` over the donor's
  `side_width` (its inner box at least 0.01 m from the centre), and with
  `gradient_attenuation` times `sat(0.5 - 0.5 y)^p` along its local y, full at one face and
  nothing at the other (a mean of `1 / (p + 1)`): the light from a window or door falling off
  into the room. Rudnya's church floor, barn, apiary and two sheds have 9 such boxes, which
  the port had drawn uniform at full strength, 2 to 5 times 0.69d's light and hard-edged. The
  power and the gradient ride in `attenuation_power` with the facing term (`ambient_facing`;
  a box without `dot_normal` carries the term 1). 0.26 has no cylinder: one is placed as the
  box around it (half extents radius, half height, radius) and the lighting pack draws 0.69d's
  cylinder in it (the distance from the axis); drawn as that box, the church towers' three
  near-black darkening cylinders also darkened the box's four corners outside their disc. The
  facing term (`dot_normal`) is 0.69d's own, `sat(sat(x/2 + 1/2)(1 + cutoff) - cutoff)^power`
  to the light's centre, not 0.26's `max(N.L, 0)`: the light's `attenuation_power` carries
  0.26's distance power with the cutoff and power (`ambient_facing`; exact for every value
  0.69d uses, a value it cannot carry stops the build), and the lighting pack's
  `ambient_light.ps` draws the term in a 0.69d scene (without the pack a sphere light so
  encoded draws nothing). The colour is packed with red
  in the low byte (packed the other way a warm fill draws blue). 0.69d fades a light out
  beyond its `fade_out_distance` from the camera; a light farther than that from where
  players can be, the ground box around every respawn point of every mode, never shows in
  play and is left out (`ambient_light_chaes`, 430 m, over the power plant 444 m out of
  Rudnya's box: kept, it lit the backdrop white into the haze). Measured from the spawns
  themselves, as until vostok-069d@7, the rule dropped room fills a player walks into 30 to
  60 m from a base: Vostok's 12 hospital and window fills (its stairwell), School's gym fill. Height-based ambient (a multiply 0.69d has not) is off.
* Materials, from the two builds' shading (`surface.rs`; `g_stage_default.ps`,
  `sun_lighting.ps` against `gbuffer_pass.ps`, `composition.ps` of shading quality 1 to 3):
  - Gloss. 0.69d's GGX alpha is `max((1 - s)^2, 0.001)` of its smoothness; 0.26's is
    `0.5 r^6 + 0.1 r` of `r = 1 - gloss`, at most 0.6. The gloss of the same alpha is not a
    line in `s`, and a smoothness map is drawn as it is (238 textures are 0.26's own files),
    so its range takes the least-squares line through that curve from the roughest 0.26 can
    draw (s 0.2254) to 0.6: `-0.17 + 0.69 s`, within 10 % of 0.69d's alpha there (96.8 % of
    Rudnya's smoothness texels are at or below 0.6; above, the line is rougher than 0.69d,
    2.2 times at 0.8); a constant smoothness takes the curve itself. The factor 0.2 used
    until then was right near s 0.3 to 0.4 only. A manifest may keep its own factor
    (`roughness_scale`: Duga and the camp lobby, fitted with 0.2; 0.46c2 has 0.26's gloss
    channel, 1).
  - Fresnel. 0.69d: `F0 = 0.04 + m (A - 0.04)`, diffuse `A (1 - m)`. Without a metalness
    map `m` is a constant of at most 0.1 on every material (the old Fresnel numbers): 0.04.
    With a map (43 of Rudnya's materials) the surface's two 0.26 maps are made texel by
    texel from the albedo and the metalness map, every level of the chain
    (`surface::metal_maps`, under `<namespace>/metal/`): the Fresnel `f` whose specular
    luminance at normal incidence is `F0`'s under 0.26's tint (`lerp(1.3 hue, 1,
    sat(1.04 - 4 f^2))`), and the diffuse `A (1 - m)(1 - F0) / (1 - f)`, which 0.26 draws
    times `1 - f`. A template without a Fresnel map (the alpha-tested, two-sided and
    vertex-colour categories) takes the map's mean as its constant. The metalness map bound
    raw as 0.26's Fresnel map (f 0.04 to 1) reflected a dark pipe twenty times too strongly.
  - The specular scale is 1 (0.69d has none). Not equal whatever the values: 0.26's sun
    highlight carries 1/pi of 0.69d's energy at the same alpha (its D has the 1/pi, neither
    diffuse has), and it cannot be rougher than alpha 0.6 (80 % of Rudnya's smoothness
    texels are).
  - A smoothness map needs a template that draws one. With a map 0.69d never reads its
    constant `smoothness` (`min + (max - min) * map.g`); a template without
    `use_troughness` drew that unused constant instead: 19 of Rudnya's materials, the
    alpha-tested two-sided ones on `flora/roots_tile`, among them the sheds' rusty sheet
    metal (`tiled/metal/metal_rust`: constant 0.8, gloss 0.62 everywhere, where its map, a
    mean smoothness of 0.07, is matte), `metal_gray_rust` (0.9) and the glass tiles (0.98):
    a wet, greasy sun highlight. The build stops on such a template;
    `alpha_test_two_sided` and `glass_cutout` take `tiled/roof/roof_broken`, the same
    switches as `roots_tile` with the gloss map on. All of 0.69d has 83 alpha-tested
    two-sided materials with a smoothness map.
  - Vertex blending keeps the donor's own numbers. Both builds weigh the base layer by
    `sat(pow(w, (1 - mask)^2 falloff) strength)`, the tiled mask turned by its rotation and
    scaled by its UV scale, and sample the second layer at the UV times its tile (0.69d
    `g_stage_default.ps`; 0.26g's `gbuffer_pass.ps`, skhid
    `scripts/v021d-weather/vertex-blend.md`). The converted materials kept the template's
    strength, falloff, tile, mask scale and rotation: 55 of Rudnya's 68 blends drew their
    moss, trash and brick patches other than 0.69d's (the lath under the wallpaper at a
    third of its tiling). The formula depends on the blend's technique and tiled-mask
    switch (linear; by the albedo's alpha; by a tiled mask), so a manifest names a template
    per switch the category's own does not draw (`<category>_technique<n>[_tiled_mask]`:
    Rudnya's technique 0, technique 1 without a mask, technique 2 with one, and the two-sided
    technique 0, each a native 0.26 blend of that switch with the category template's other
    switches, which stock 0.26 has for every combination). With one template a category,
    10 of Rudnya's 68 blends kept the template's numbers: the sheds' rusty roof sheets
    (metal over rust, technique 0) drew rust all over where 0.69d shows its patches (with
    the rule the patches follow 0.69d's; the rust layer's colour stays more saturated than
    0.69d's: the blended layer is not made a metal surface). The alpha-tested ones (4 on Rudnya,
    technique 1 without a mask) stay on their category's template: their cut is the blend
    pack's selector for it, and stock 0.26 has no one-sided alpha-tested template of that
    switch; noted.
  - Technique 2 (always with a tiled mask) draws no second layer in either build: both
    programs draw the base layer and discard where the masked weight is under 0.4 (0.69d
    `g_stage_default.ps`, 0.26g `gbuffer_pass.ps`; alpha-tested, where the weight times the
    base alpha is under `alpha_ref`). So a technique-2 blend binds no second texture and is
    a blend all the same: the terrain overlays (`terrain_tiled/{grass,dirt,sand_grass,earth}_vc`,
    `tiled/trash/trash_vc`, `trash_dirt_moss_alpha_vc`: 6 of Rudnya's materials, on its 12
    `terrain_game` tiles, the bus stop, the culture centre and house_3) were taken for
    one-texture vertex-colour meshes and drawn whole to their mesh borders: straight seams
    where two ground types meet. The alpha-tested one (`earth_vc`) takes
    `vertex_blend_alpha_test_technique2_tiled_mask`, 0.26g's `terrain/ground_moss_s_alpha_vc`.
  - Not converted: 0.69d can cut an alpha-tested blend by its second layer's alpha
    (`consider_blended_albedo_opaque` false: `alpha = lerp(blended.a, 1, w)` with the base
    opaque); 0.26g's programs read only the base's (`lerp(1, base.a, w)`). The lath
    `stucco_wood_pbr` is 38.5 % transparent with black under it, so those faces draw black
    (house_7's stairs: `tiled/wall/paper_01_palki_vc`; 3 Rudnya materials, 17 in all of 0.69d).
* Sky: the donor's clouds (`sky/sky_test`, imported; lit through the template's cloud normal
  map, since the donor's have none and 0.26 draws unlit clouds dark) over 0.26's own
  atmosphere.
* Exposure, grading and curve are 0.69d's own (`tone.rs`), not fitted to frames. 0.69d
  exposes each view by `1 / (1.2 * 2^EV)`, `EV = clamp(log2(L) + 3 + bias, min, max)` of
  the view's metered luminance (the environment's 6 to 12.5), grades in AP1 (shadows,
  midtones, highlights) and draws the curve `x^a / (x^d b + c)`. 0.26's curve follows the
  adapted mean between 0.05 and 2 only. So the scene carries an exposure marker
  (`mie_fog_base_height` 33000 + the level's number, an integer above every camera: every
  native shader reads the field as 1 there) and resource-porter's lighting pack draws the
  native tone block behind it with the mean clamped to 0.69d's range and its own key
  (`ExposureProfile`: floor 0.001151, ceiling 0.104167, key 0.3367 for Rudnya, printed in
  the build's notes; the curve is `1 - 2^(-X)`, sm_4_0's `exp` being `2^x`: read as
  `1 - e^(-X)` until rudnya-069d@8, with key 0.2334, it put 0.69d's look on 0.74 of the
  light, every 0.69d scene shown as if lit at three quarters); and the scene's colour grading texture
  (`textures/<namespace>/color_grading/day`, 16^3) holds, for each colour 0.26's curve can
  show, what 0.69d's grading and curve show for the light that gives it. 0.26's blue shift
  of dark colours is off. Gone with it: `exposure_bias`, `sun_saturation`, the raising of
  dark interior probes (`adaptation_gain`), the per-area gains (`area_gains`) and the floor
  under darkening lights. The pond's mist is the donor's `volume_fog`, the same program and
  constants in both builds (its colour is a constant, `color^2.2`, lit by nothing): copied
  field by field, it is as luminous against the scene as in 0.69d at the outdoor exposure.
* Scene objects both builds have come from the donor's own project: its 22 sound emitters
  where it has them (11 sounds; `scapes/level_01/water_under_car_01`, which 0.26 does not
  ship, is imported: an Ogg stream, its loudness track and options with the same fields in
  both builds), and a particle 0.26 ships under the same name (`fx/environment/fly`). A
  manifest with `ambient_sounds` takes a native map's emitters instead (Duga).
* Not reproduced yet: 0.69d's height fog over the whole map (it lives in `sun_lighting.ps`:
  extinction `fog_density * 0.001 * exp((fog_base_height - y) / fog_falloff_height)` per
  metre, 0.0031 at Rudnya's ground, lit by the horizon sky times `fog_tint`; 0.26's Mie fog
  can hold its extinction for a level ray, not its thinning with the target's height nor
  its light), its sky (a precomputed atmosphere; 0.26's Rayleigh sky follows
  `sun_intensity^1.4` and agrees at the zenith in this scale, with a brighter horizon), its
  clouds' own lighting, light shafts, sound environments, probe parallax correction and
  specular fade, the exposure profile under the two depth-of-field tone programs.

**Sharp, as 0.69d draws it.** No `max_texture_size` or `texture_limits`: every imported
texture keeps its full size and every mip (605 MiB, Duga's 1024-capped set was 651);
none is larger than 0.26g's stock maps use for its kind (2048; terrain normal and gloss
maps 1024). `reuse_native_textures`: 238 of the 463 are the same image as 0.26g's file at
the same path (0.69d re-encoded what it kept: about 1 level apart, its retouched ones 3 to
60) and are drawn from it, 345 MiB not imported; the other 225 (297 MiB) are imported.
LOD0 and LOD-less surfaces keep their authored geometry (no `lod0_*`); only authored
far LODs are simplified (110 k -> 52 k triangles). The terrain keeps every vertex:
simplified tiles under full-density blend overlays (`*_vc`) left the overlays up to 2 m
under or over the ground (1 075 overlay vertices more than 25 cm off, 94 without), the
dark seams and holes along trenches and mounds; 35.6 MiB of geometry for TDM, 40.9 MiB
for the charging-station mode (native maps 43-48). Imported models are 47 MiB of files in all. What 0.26
cannot draw stays as before: metalness, parallax, detail maps, fuzziness (the notes of
the bundle list each material).

**What the builder learnt for it.**

* `level_id` and `map_name` (map spec): a port of a map 0.26 has takes its own level
  number (`match_making_params.map_id`; the lobby keys names, room tiles and play counts
  by it) and a name key the target's dictionaries carry (skhid-srp's localization adds
  `st_level_14_name`), instead of the donor's `st_level_01_name`, which is native
  Rudnya's.
* The record names the map (`maps`), so `pack from-bundle` declares it to the server
  (`server.maps`); the asset id follows the output project.
* The template's light is asked for only when the donor has lights (native Rudnya has
  none).
* `wood_tin` (a misspelt game material on `props/tool/ax`, in 0.69d's table under no
  name) and the snow, slime, smoke and marker materials have 0.26 stand-ins;
  `generic/underground_black_mat` (a flat black colour without a texture) is drawn with
  native `tiled/concrete/concrete_black`.

Spawns: the 405 respawn points of `logic/modes/team_death_match/` (team 0: 313 in four
groups around (49..77, 32..54); team 1: 92 around (-34..-51, -83..-94)).
