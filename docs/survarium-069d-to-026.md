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
* **Project schema**: the project carries every container group of the native
  template (`victory_item_spawn_points`, `temporary_render_visuals`,
  `temporary_collision_objects`), empty. Builds without them crashed the
  client 20 ms after match setup (ACCESS_VIOLATION at "thir"), before any
  resource loaded; every working port has them.
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
* **Performance**: imported models with LODs switch by distance 12/30 m;
  authored far LODs are simplified (meshoptimizer, error-bounded; LOD2 may
  collapse across UV seams and drop small parts) and heavy LOD-less models
  get generated LODs; terrain render meshes are simplified with locked
  borders (tiles stay sealed; error <= 0.4% of a tile, about 14 cm);
  imported textures above 1024 (smoothness maps above 512) lose their top
  mip. **Material clusters** merge render instances into render-only models
  with one surface per material and LOD: first models alike (same LOD
  switching, same material set: copies of a model, or the native elka
  trees sharing `flora/elka`) per 32 m cell, then the remaining small
  imported instances (radius <= 10 m) across models per 12 m cell; glass,
  water, terrain and projected decals stay alone, foliage and native
  models merge only with models alike. 0.26 picks a model's LOD by the
  distance to its bounding-box centre (`select_lod`), so each cluster
  switches later by its centre-to-member reach: no member is drawn coarser
  than it would be alone. Collision instances are unchanged.

Static budget at the 42 spawns (`examples/view_budget`, omnidirectional):

| | mean tris | worst tris | mean draws | worst draws |
|---|---|---|---|---|
| level_03 day_4 | 634 026 | 680 149 | 1 081 | 1 105 |
| level_07 day_4 | 790 954 | 905 423 | 1 457 | 1 606 |
| Duga, first build (b9) | 988 080 | 1 034 572 | 1 936 | 1 945 |
| Duga | 814 014 | 965 991 | 1 717 | 1 724 |

Texture payload 651 MiB (level_03 638, level_07 720); particles 0. Draws
stay ~7% above level_07's worst spawn: larger alike cells (40 m) bring them
to 1 611 but push the worst spawn to 1.15 M triangles.

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
