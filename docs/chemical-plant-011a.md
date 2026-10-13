# Chemical Plant from v0.11a

The local v0.11a build 816 archive already contains Chemical Plant (`level_04`),
including its client/server projects, industrial buildings and original vegetation.
This is the May 14, 2013 binary version `0.1.1a`. Historical update notes announce
the map in v0.11b; that does not change the verified contents of build 816.
Only its authored day variant is imported, as match ID **193**.

Inputs are pinned by the manifest:

| Archive | SHA-256 |
| --- | --- |
| 0.11a donor | `f59650ee69e834f7757461c54affd93b6fbb767ab841d70468e85019b059c4f6` |
| 0.26g target | `c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c` |

The donor is available locally at
`/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011a/app/resources.db`.
Its hash was checked directly; no download was needed.

## Reproduce

Build `vostok-port` from this branch with Cargo. Put FFmpeg on PATH, then run:

```sh
python3 tools/chemical-plant/rebuild.py \
  --porter target/release/vostok-port \
  --donor-db /path/to/011a/resources.db \
  --target-db /path/to/026g/resources.db \
  --output /new/chemical-plant-output
```

The command builds an additive bundle, packs it over the untouched target, and
reopens the result to check every emitted resource against its recorded hash.
The existing early-import implementation was copied from the main checkout into
this isolated branch; the source snapshot and exact commands are preserved with
the evidence. Existing running archives and the saved lighting preset were not
modified.

## Verified result

The October 13 build emits **3,983 files and 173 materials**. Its archive hash is
`66147095350f245afceb315827d9716ac5df79494f21f38e400dc59185cd1179`.
The source scene has 442 placed render models, 600 collision objects, 44 respawn
points, 12 local lamps plus the sun, and 13 environment probes. Its 12 vegetation
layers retain all **11,346 authored positions**. One Soviet computer collision
payload is absent from the original archive and explicitly omitted by the port.

The isolated 0.26 client loaded map 193, verified its server checkpoint, and
rendered the factory interior. A 1280×720 screenshot was saved with `ui off`.
This proves scene loading and rendering, not a playthrough or lighting parity.
The shared diagnostic runtime stall then stopped camera/present counters at
41/300; none of the eight widely separated requested camera poses completed.
The owned Wine prefix and server were stopped afterward.

The library suite passes 279 tests. This task added no test functions: it extends
an existing material test, which fails with the old selection order and passes
with the fix. The nine additional tests relative to the earlier 270-test worktree
already belonged to the imported early-format adapter.

## Materials and remaining limits

Material conversion previously prioritized similar filenames over matching shader
features, despite its diagnostic ranking doing the opposite. Both now prefer
features after enforcing the render-stage and semantic-domain contracts. The
`lamp_greenhouse_clone` material consequently selects the native emissive
`lamp_lights_light` template, retaining its emissive shader flags.
Explicit manifest choices remain authoritative.

Early imports no longer automatically force every same-named modern material.
Twelve previously used forward-rendered templates remain explicit exceptions:
the old schema has no `two_sided` selector and the native schema adds it. Their
effect IDs match, and their added selectors are recorded in the material audit.
This preserves the previous compatibility behavior without claiming identical
rasterization. The fresh Chemical Plant build changes 54 template choices; their
mapping is saved for visual review.

Runtime warnings remain for original missing model-surface materials, battery
container locators, and two active texture references. `vehicle/common/vehicles_spec`
is missing in both source and target. `floor_plate_tile` has an empty original
roughness binding but the converter inherits a missing texture from its template.
The latter needs an explicit empty-texture/fallback policy; no replacement texture
was invented. Most other missing texture strings belong to disabled channels.
Native 0.26 rendering still differs from the old engine.

## Shader evidence

`vostok-versions/versions/0.11a/builds/816/shaders.jsonl.gz` indexes 261 shader
names and 3,408 permutations. Every recorded blob hash was checked against the
actual donor archive. Full DXBC is available there; five selected assembly
listings are stored in that directory's `shader-assembly` subdirectory.

The local 0.10b→0.11a comparison records instruction changes in five shader names:
`environment_probe_lighting.ps`, `gbuffer_pass.ps`, `god_rays.ps`,
`terrain_gbuffer_pass.ps`, and `vertex_base.vs`. The translucency shader is
unchanged. These are version-specific facts, not a claim that native 0.26 shaders
are interchangeable with them.

Evidence is retained under `resource-porter/.work/chemical-plant-011a`: archive,
bundle manifest, source snapshot, build and test logs, shader verification,
material audits, launch recipe, screenshot, and failed controlled-camera request.
