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
  displacement, fuzziness and detail maps are not reproduced.
  `verify_shader_variants.py --templates` proves each new material against
  the template it was built from.
* **Terrain** (0.69d layered terrain, five layers and a 2048² blend mask)
  becomes per-segment two-layer vertex blends: the segment's two dominant
  layers on a native vertex-blend template, the blend weight per vertex from
  the mask, stored in a new vertex colour (stride 28 -> 32).
* **Scene**: render and collision instances keep their transforms
  (`model_name` -> `lib_name`). The environment, sky and post-process are the
  native template's (level_03 day TDM), lit from the 0.69d sun direction.
  0.69d probes are encoded for its exposure pipeline and do not decode in
  0.26; one native global probe covers the map and native indoor cubemaps fill
  the 0.69d tunnel and building probe volumes. 0.69d ambient lights
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
* **0.69d**: lux and lumen as for Duga, but the lobby keeps its ambient
  lights: `type` 1 (multiplying, the painted contact shadows) is dropped,
  spheres take radius x scale, and intensities are scaled to the 0.26 lobby's
  median and capped at 3. Its probes keep their volumes on the 0.26 lobby's
  cubemap (`"probes": {"template": ...}`). `exclude_models` leaves out the New
  Year snow of volume 72's project; `native_materials` draws its window panes
  (`water/glass_water`) with 0.26's `tiled/window/glass`.
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
  * `probe_overrides` sets a donor probe by `full_name`: `volume` (`{"box": {centre,
    half_extents}}` or `{"sphere": {centre, radius}}`: the camp's box holds all the blocks;
    left out, every face outside the sun was black and the fog made them flat slabs),
    `diffuse_scale` and `saturation` in place of the lobby's baked ones (the outdoor sky
    irradiance is bluer than the room's).
  * A lobby's box probes get `radius` 0.75 x their largest half-extent. 0.26g draws the
    visible probes largest radius first, each over the ones before (stage_visibility's sort),
    and equal radii in its probe tree's order, which changes from run to run with the order
    the cubemaps load in. With every probe at the template's 15.3, the camp's outdoor box drew
    after the room's probes in about half the runs: a bluish haze over the room's shadows.
    0.10b's engine set this radius itself; 0.26g takes the project's, and a box's radius is
    only that sort key (its volume is its scale). So the outdoor box draws first and the
    room's probes, smaller, over it. A box whose radius ties the one before it (equal largest
    half-extents: two of the hangar's) goes just below it, so the donor's order decides.
  * The donor lights its far blocks with outdoor ambient light boxes
    (`environment_and_lights/outdoor/`); the photometric scale made for the room left them
    at 0.15, so `light_overrides` sets them (0.26 does not apply the outdoor probe that far
    out: the block at 88 m drew as a flat slab with it alone).
  * `environment_overrides` gives the camp level 02 day's outdoor fog (rayleigh 15..300 m,
    mie 40..1200 m): the lobby template's (rayleigh from 1 m to 185 m) is an interior's.
  * `native_scenery`: 0.26's own models drawn outside, `{lib, position, yaw, scale}`, as
    render visuals only (no collision, so the camera's ray never meets them): the camp's
    overgrown yard (level 02's poplars, birches, willows, fruit and dead trees, bushes, the
    yard trees scaled 1.1-2.1 so their crowns reach the room's windows), level 04's tree
    belts past the blocks, and level 02's own background terrain and tree rings (as the
    school map has them, one transform for the four) for the horizon.
