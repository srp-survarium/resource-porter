# Survarium 0.69d maps in 0.26g (Duga)

The first port *down* from a build newer than 0.26: 0.69d0 (2022) level_11,
"Duga radar station", becomes 0.26g map **205**, mode 4 (team deathmatch),
on the 0.69d TDM spawns.

```sh
vostok-port build-survarium-069d-map \
  --manifest ports/survarium-069d-duga-to-026g.json \
  --donor-db donor-069d.split --target-db "$TARGET_026_DB" --output "$BUNDLE"
vostok-port apply --base-db "$TARGET_026_DB" --bundle "$BUNDLE" \
  --work-dir "$WORK" --output-db "$DB"
```

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
* **Performance**: imported models with LODs switch by distance; authored far
  LODs are simplified (meshoptimizer, error-bounded) and heavy LOD-less
  models get generated LODs; terrain render meshes keep every second grid
  line; repeated imported models are statically batched in 64 m cells;
  imported textures above 1024 (smoothness maps above 512) lose their top mip.

## Ladders

The 0.69d "ladders" (`generic/silo/*ladder*`, `props/ladder/*`) are stairs:
their walker collision rises at 10-36 degrees (with instance scale), well
inside the 0.26 controller's 60 degree limit and 0.6 m step. They walk the
same in 0.26 and need no ramp. 0.26 has no climbing state. `examples/
steep_walker.rs` lists every narrow walker face steeper than 55 degrees that
rises over 2 m; in the TDM scene these are structural faces (fence posts,
transformer ribs, pylon frames), not climb routes, so no ramp or plank was
added.
