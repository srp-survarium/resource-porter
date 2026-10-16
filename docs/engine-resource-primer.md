# From a Map File to a Bullet Hole

When a game displays a scene, it is not simply opening a model and putting a
picture on it. Several independent systems cooperate:

1. geometry says where triangles are;
2. a render material says how those triangles should look;
3. textures provide images and other per-pixel data to that material;
4. collision geometry says where the physical world is;
5. a physical material says what that world is made from;
6. effects use the physical material to choose bullet marks, sounds, sparks,
   dust, footsteps, and similar reactions;
7. a level or project places all these resources into a playable world.

Most of our porting bugs happened when one layer was valid but disagreed with
another. A wall can look like brick but behave like metal. A texture can exist
while its shader cannot load. A model can render correctly while its collision
material is an invalid sentinel. Understanding those boundaries makes the
failures much less mysterious.

## Resources and the resource database

A **resource** is an object the engine can request by a logical name. Examples
include:

- `models/props/crate.model/settings`;
- `textures/props/crate.dds`;
- `material_instances/props/crate.material`;
- a level project;
- a collision vertex buffer;
- a sound or particle definition.

Survarium stores these named resources in `resources.db`. The database is more
like a virtual filesystem than one monolithic game structure. A model itself is
a directory-shaped family of resources: settings, render surfaces, collision
vertices, collision indices, and collision face data.

Paths are part of the data contract. A valid payload stored under the wrong
logical path is still missing from the engine's point of view.

The engine loads many resources through **cookers**. A cooker takes serialized
source data and creates a runtime object such as a GPU texture, render model,
shader, or Bullet collision shape. The assertion
`data->new_resource->shader_source == 0` came from such a boundary: the loader
was trying to finish a material, but one of the shader resources required by
that material had not been produced.

In general engine terminology this stage is often called loading, cooking,
deserialization, or runtime resource creation.

## Geometry, meshes, surfaces, and models

A **mesh** is primarily:

- vertices: points with attributes such as position, normal, tangent, UV, and
  sometimes bone weights;
- indices: integers grouping vertices into triangles.

A **surface** is a subset of a model rendered with one material. A building may
therefore be one model with separate brick, glass, metal, and decal surfaces.
The model settings must list exactly the surfaces for which render data exists.

A **static model** has vertices fixed relative to the model. A **skinned model**
adds bone indices and weights. At runtime its vertices move with a skeleton and
animations. Incorrect bone indices or coordinate conversion produce the
“exploded mesh” effect: vertices remain valid numbers, but are transformed by
the wrong bone or from the wrong reference space and fly far away from the
body.

A model's **bounding box** and **bounding sphere** are cheap approximations of
its occupied space. The renderer uses them for visibility tests. Bad bounds can
make a model disappear too early, remain visible when it should not, or distort
camera and streaming behavior, even if its triangles are correct.

## UVs and textures

A **texture** is an image sampled by a shader. A vertex normally carries a
two-dimensional **UV coordinate** telling the shader which location in the
image corresponds to that point on the triangle. Bad UVs cause the right image
to be wrapped around geometry incorrectly. A missing texture usually gives a
fallback color or error image instead.

Common texture roles are:

### Diffuse or albedo

This is the base visible color of a surface: brick color, painted metal, cloth,
and so on. In older rendering pipelines “diffuse” may already contain some
painted-in lighting. In more physically based pipelines “albedo” means the
material color without lighting.

For the SKS, changing the material's diffuse binding to `sks` told the shader
to sample the SKS base-color texture. It did not change geometry or collision.

### Normal map

A normal map changes the direction from which a pixel appears to face without
adding triangles. It creates the lighting impression of small grooves, dents,
cloth weave, and bevels.

Normal maps are vectors encoded as color channels, not ordinary photographs.
Engines differ in channel order, sign convention, compression, and whether one
component is reconstructed. Copying a newer normal-map DDS byte-for-byte into
an older engine can therefore produce inverted or nonsensical lighting.

The v0.100b texture contract also includes DDS format metadata and an `A2D5`
marker for its DXT5 normal-map representation. We convert the STK2 source
channels into that target representation instead of treating the image as a
regular diffuse texture.

### Specular map

A specular map controls how strongly light reflects. Dark areas reflect less;
bright areas reflect more. Depending on the shader, a separate map or channel
may control specular power, gloss, or roughness—the size and sharpness of the
highlight.

This is why two metal surfaces can share a similar gray diffuse texture but
still look very different. Their highlight behavior carries much of the visual
identity of metal.

### Feature, mask, and packed maps

These names describe textures whose channels are masks rather than colors.
One channel might select painted versus bare metal, another dirt, another
gloss, and another an effect strength. Their meaning is shader-specific.

The earlier SKS fix bound:

- diffuse: `sks`;
- feature map: `sks_fmap`;
- normal map: `sks_nmap`;
- specular map: `sks_smap`.

Those names were inserted by parsing and reserializing the length-based binary
config. They were not unsafe same-length byte substitutions. The serializer
updated the encoded string lengths and surrounding structure.

### Alpha

Alpha is commonly used for opacity. **Alpha testing** discards pixels on one
side of a threshold and keeps the rest. It is useful for leaves, wire fences,
and other cut-out shapes. **Alpha blending** supports partial transparency but
requires more expensive ordering and has different visual behavior.

If foliage is forced through an opaque material, every leaf card appears as a
solid rectangle. If it uses the wrong transparency family, leaves may vanish,
sort incorrectly, or create bright/dark borders.

### Mipmaps and color space

**Mipmaps** are successively smaller copies of a texture. The GPU selects an
appropriate size when the object is distant, reducing shimmer and bandwidth.

**sRGB** identifies color data that needs gamma-aware sampling. Diffuse color
is generally sRGB; vector-like normal and mask data generally is not. Sampling
either in the wrong color space changes the numeric values seen by the shader.

In v0.100b a DDS, its `.options` resource, its FourCC compression format, its
mipmap count, and its trailing sRGB byte must agree. A visually valid DDS can
still violate the engine's serialized texture contract.

## Shaders, material templates, and material instances

A **shader** is a GPU program. It transforms vertices and calculates pixel
colors using geometry, lights, textures, and parameters.

A **material** selects a shader family and supplies its inputs. It may say:

- which diffuse, normal, and specular textures to use;
- whether alpha testing is enabled;
- whether both sides of a triangle render;
- whether the surface is foliage, metal, terrain, or a decal;
- numerical values such as gloss or effect strength.

A **material template** or base material defines the general shader behavior.
A **material instance** specializes it with particular textures and settings.

This separation is why replacing only texture strings is sometimes safe and
sometimes insufficient. If a new material uses the same target shader features
as an old material, we can retain the proven v0.100b template and replace its
bindings. If it requires a feature the selected shader variant does not expose,
the engine may request a shader permutation that v0.100b does not possess.

For the current STK2 conversion we preserve the target material's shader
feature booleans and translate supported texture bindings. We do not copy newer
shader flags blindly. Opaque, alpha-tested, two-sided, decal, and metal families
use separately reviewed v0.100b templates.

## Render materials and physical materials are different

A **render material** answers “how does this surface look?”

A **physical material** or **game material** answers “what is this surface made
of when gameplay interacts with it?”

Examples of physical materials are concrete, brick, thin metal, thick metal,
wood, earth, and glass. Their definitions can affect:

- bullet penetration and ricochet;
- impact sound;
- bullet-hole decal;
- sparks, dust, chips, or blood particles;
- footsteps;
- friction and other physics properties.

The two systems may be correlated, but neither replaces the other. A brick
diffuse texture does not automatically make collision behave as brick.

This distinction explains our red `Oops` marks. The bullet-hole textures are
present and valid. However, our first STK2 collision adapter generated a
single collision face name, `collisionSG`, and inherited this mapping from the
lobby template:

```text
collisionSG -> game material ID 65535
             "not assigned or deleted"
```

ID 65535 is an invalid/unassigned sentinel. A ray from the gun can still hit
the collision triangle, but the impact system cannot select a valid material
pair and displays its error decal. The correct fix is to map collision faces
to real v0.100b game-material IDs and reject ordinary collision that retains
the sentinel.

## Collision geometry and face data

Render triangles are optimized for appearance. **Collision geometry** is a
usually simpler representation used by physics and gameplay queries. It can
contain triangle meshes or primitives such as boxes, spheres, cylinders, and
capsules.

The v0.100b triangle-collision family consists of:

- `collision/vertices`: packed `float3` positions;
- `collision/indices`: three `u32` indices per triangle;
- `collision/face_data`: a dictionary of material names and one `u16`
  dictionary index per triangle;
- parent model settings mapping those names to numeric game-material IDs.

This is a two-step lookup:

```text
triangle
  -> u16 face index
  -> name in collision dictionary, for example "brick_collision"
  -> game_material_settings.brick_collision.game_material_id
  -> v0.100b physical material, for example brick
```

Keeping indices and vertices while discarding or inventing face data preserves
the wall's shape but loses what each triangle means. A robust porter must
convert the entire collision family as one atomic contract.

Collision also explains why an invisible wall can exist: its collision loaded,
but its render geometry or material did not. The opposite produces visible
ground through which the character falls.

## Decals

A **decal** is a small projected or overlaid visual applied to an existing
surface. Bullet holes, blood splashes, scorch marks, dirt, posters, and painted
road markings are typical decals.

There are two relevant kinds in our work:

1. authored map decals are placed by the level project and rendered as part of
   the environment;
2. gameplay impact decals are spawned dynamically after a ray or projectile
   hits collision.

A decal still needs a render material, texture, transparency behavior, and
often a surface normal or projection orientation. A bullet impact additionally
needs a valid physical-material lookup to decide *which* decal to spawn.

Thus a red `Oops` on impact does not necessarily mean the bullet-hole DDS is
missing. It can mean the earlier physical-material decision failed.

## Terrain

**Terrain** is a large ground surface usually stored differently from ordinary
props. A terrain cell can contain:

- a height field or grid of vertex heights;
- layers selecting grass, dirt, rock, asphalt, and other textures;
- blend weights determining where each layer appears;
- UV scale and projection parameters;
- collision derived from the same ground shape;
- vegetation/grass placement data;
- additional detail, normal, and macro textures.

Generating valid terrain triangles is only the first step. If blend weights,
layer indices, UV scale, or material semantics are translated incorrectly, the
ground is physically present but colored blue, transparent, monochrome, or
covered by the wrong texture.

**Grass** is often a separate system rather than triangles baked into terrain.
It may depend on density masks, prototype models, wind shaders, and cell
streaming. Therefore “terrain loaded but there is no grass” is a coherent
partial result, not proof that the ground texture failed.

## Fog, environment, and post-processing

Fog is not necessarily a texture painted onto terrain. It is usually an
environment or volume calculation applied after opaque geometry is rendered.
It can depend on camera position, depth, height, distance, sun color, and an
environment lookup. Crossing a fog boundary can therefore darken the terrain,
trees, character hands, and weapon at once. That shared spatial change is
strong evidence against a missing diffuse texture.

An environment object commonly controls fog, sky, exposure, ambient color and
post-processing. Copying a newer environment object byte-for-byte is unsafe:
the old renderer may expect different fields or material resources. The porter
must clone the v0.100b object schema and translate only understood semantics.
A map that is stable under a stock environment can consequently be too bright
or lack the source fog while still being a valid conversion checkpoint.

Survarium v0.100b has two relevant fog mechanisms:

- global distance fog in the post-process combine pass;
- spatial volume-fog scene objects with density, color, transform, noise and
  height-falloff parameters.

Its global shader starts blending after `environment_near_fog_distance` and
uses `environment_far_fog_distance` as an exponential falloff divisor. The
latter is not the distance at which fog suddenly reaches 100%. Recovered
renderer code does not read `environment_fog_alpha`, even though old material
configs serialize that field. Treating it as an enable switch would therefore
be an invented and ineffective conversion.

Color grading is separate from fog. A grading lookup table remaps rendered
colors after lighting and can make an image look darker, cooler or hazier
without changing fog density. STK2 stores the A01 LUT as sixteen 16x16 slices
packed into one 256x16 TGA strip. v0.100b expects those samples as a
16x16x16 volume DDS with its own mip layout. The converter must rearrange and
cook that payload; merely changing the material's texture string would point
the old renderer at a resource with the wrong dimensional contract.

## Sounds and ambient sound objects

An encoded sound payload (such as Ogg Vorbis) is only one part of a sound
resource. Sidecar options describe the target engine's playback contract, and
a project `object_sound` places or activates the logical sound. Ambient loops
may be non-positional even though their scene object retains a position.

Survarium 0.20 stores source-side `.raw_options`, while v0.100b resolves a
single sound through:

```text
object_sound sound_name
  -> sounds/single/<name>.high.ogg
  -> sounds/single/<name>.single_sound_options
```

The STK2 A01 ambient adapter therefore copies only the donor OGG. It clones the
old `.single_sound_options` and `object_sound` structures from declared stock
templates, changes their logical name and placement, and validates the Ogg
signature. This is the same target-schema/source-payload rule used elsewhere
in the converter.

## Models, maps, levels, and projects

These names vary by engine, but a useful distinction is:

- a **model** is a reusable object such as a wall, tree, weapon, or building;
- a **map** or **level** is the world assembled from model instances, terrain,
  lights, decals, particles, sounds, and gameplay markers;
- a **project** is the serialized scene/container the engine loads, including
  object placement and references to external resources;
- server-side level data defines gameplay facts such as spawn points, teams,
  objectives, and match rules.

The STK2 project files are editor/source scenes. They are not ready-made
Survarium runtime maps. We must:

1. interpret the source project and its referenced models;
2. convert each supported payload to the v0.100b runtime ABI;
3. build target-native material and texture resources;
4. convert collision and physical-material mappings;
5. generate a v0.100b client project;
6. generate matching server level data;
7. intentionally drop or replace unsupported systems such as some anomalies.

A client map can render while being unplayable if its server project, spawn
coordinates, or collision groups disagree. Conversely the server may accept a
match while the client stalls loading a malformed resource.

## Coordinate systems, transforms, and instances

An object's **transform** contains translation, rotation, and scale. Different
engines or exporters may use different handedness, axis directions, matrix
layout, angle units, or multiplication order.

Symptoms are recognizable:

- front and back reversed: handedness or forward-axis mismatch;
- helmet rotated 90 degrees: up/forward axis mismatch;
- entire model misplaced: world versus local transform confusion;
- only some limbs explode: bind pose, bone index, or skin-space conversion;
- collision offset from visuals: render and physics transforms converted
  differently.

A map usually stores one model once and creates many **instances**, each with a
transform. An instance-qualified resource name can also include scale or other
suffix data. The porter must normalize that suffix for dependency lookup while
preserving it for the runtime instance.

## Dependencies and why “the file exists” is not enough

Resource loading forms a graph:

```text
project
  -> model
     -> settings
     -> surfaces
        -> material instances
           -> shaders
           -> diffuse/normal/specular textures
              -> texture options
     -> collision face dictionary
        -> physical-material IDs
           -> impact effects and decals
```

A porter needs to validate both graph closure and cross-resource contracts.
Useful questions are:

- Does every referenced path exist?
- Is it encoded in the target version's format?
- Does every render surface have exactly one settings entry?
- Does every collision dictionary name resolve to a valid target game material?
- Do DDS metadata and texture options agree?
- Does the chosen target shader support all emitted bindings?
- Are donor numeric IDs translated by name rather than copied?

Earlier dependency scanning preferred donor files whenever donor and stock had
the same path. That silently mixed partial newer model directories with older
settings. The resources existed individually, but the resulting model had
three render surfaces and settings for only two. The safe policy is to keep
stock dependencies by default, import missing donor resources deliberately,
and validate the completed target view.

## A practical way to diagnose visual failures

Start by deciding which layer is wrong:

| Symptom | First systems to inspect |
| --- | --- |
| Object has the wrong picture wrapped around it | material binding, diffuse texture, UVs |
| Lighting dents appear inverted | normal-map channel/sign conversion |
| Leaves are rectangles or disappear | alpha test, two-sided shader, alpha channel |
| Object is invisible but blocks movement | render model/material; collision probably loaded |
| Object is visible but can be walked through | collision resource, collision transform |
| Bullet impact shows an error mark | collision face material, game-material pair, impact decal |
| Mesh stretches far from a character | bone indices, weights, bind pose, vertex layout |
| Map is monochrome or has a screen-space circle | post-processing/profile/camera resources |
| Ground exists but grass does not | separate grass placement/prototype/shader resources |
| Loader asserts on a null shader resource | material template, shader variant, texture contract |

Logs reveal failed resource requests, but silence does not prove correctness.
The engine may accept a validly encoded yet semantically wrong value. That is
why the porter also needs offline structural validation and controlled
comparisons with real v0.100b resources.

## The central porting rule

Do not ask only whether newer bytes can be inserted into the old database.
Ask what runtime object v0.100b expects at each resource path, then construct
that object from the newer or STK2 source data.

That changes the work from hopeful asset copying into explicit conversion:

```text
source meaning -> intermediate understanding -> target-native contract
```

Unsupported meaning can then be rejected or deliberately omitted. Supported
meaning becomes reproducible, testable, and reusable across every map instead
of accumulating one-off binary patches.

# Reading the Vostok resource porter

The rest of this document is a companion to the converter's Rust source. It
explains the vocabulary used by the code and how data moves through the
program. The source is much easier to follow if one keeps this pipeline in
mind:

```text
manifest
  + target v0.100b database
  + donor database or STK2 source tree
        |
        v
source parsing and dependency discovery
        |
        v
explicit version/format adapters
        |
        v
additive overlay directory
  + port-manifest.json
        |
        v
compose zero or more bundles
        |
        v
0.26g: a patch, the overlay files that differ from the
stock database (and earlier patches), packed as
resources.db.patch_<n>; v0.100b: the stock database
written again with the overlay over it
        |
        v
reopen, hash-verify, and structurally validate
```

The important architectural decision is that conversion and archive packing are
separate. Conversion produces ordinary files under an `overlay/` directory.
Packing later merges that directory with stock resources. This makes generated
content inspectable before a multi-gigabyte database is created.

## Source tree tour

| Module | Responsibility |
| --- | --- |
| `main.rs` | CLI commands and argument routing |
| `archive.rs` | safe read access to the `resources.db` virtual filesystem |
| `binary_config.rs` | parse, edit, and serialize Vostok's self-describing configs |
| `manifest.rs` | declarative input schemas and generated build-record schemas |
| `dependency.rs` | resource-reference graph discovery and donor/target selection |
| `weapon.rs` | weapon/ammunition generation plus shared bundle/apply/compose machinery |
| `armor.rs` | skinned-model, skeleton, weight, and armor texture conversion |
| `material_conversion.rs` | cross-version material inspection and target-template selection |
| `map_compare.rs` | independent structural and dependency-graph comparison of map projects |
| `map_port.rs` | older dependency-closure maps, playable lobby, and STK2 project assembly |
| `target_native_map.rs` | explicit newer-Survarium-to-v0.100b map ABI adapters |
| `stk2_project.rs` | parser for STK2 editor projects and composite library objects |
| `stk2_content.rs` | STK2 weapons, static models, terrain, collision, materials, and textures |
| `maya_binary.rs` | extraction of skin influences and weights from Maya Binary scenes |
| `model_validation.rs` | effective-view model/settings/surface validation |
| `lib.rs` | exposes the modules and central types to the CLI |

The archive code is intentionally separate: skhid-srp's `resources-db` crate
(`srp-026g/crates/resources-db`, a git dependency) owns the physical archive
container format: the reader, the overlay of an archive and its patches, and
the writer. The main crate owns the *meaning* of the files stored inside it.

## `ResourceDb`: the virtual filesystem view

`archive.rs` wraps the crate's reader in `ResourceDb`. Its important
operations are:

- `open`: index all virtual paths; `open_layers`: the stock archive with its
  `resources.db.patch_<n>` files over it, as the 0.26g game mounts them (for
  each path the last archive's file);
- `contains` and `is_file`: distinguish an exact resource from a folder prefix;
- `paths_with_prefix`: walk one virtual subtree;
- `read`: read one file;
- `extract_file`, `extract_prefix`, and `extract_all`: materialize resources on
  the host filesystem.

The wrapper intentionally stores entries in a `BTreeMap`. This gives stable
ordering, which in turn makes reports and generated manifests reproducible.

An archive that names a path twice, or holds node kinds 0.26g's archive
has none of (compressed, inlined, soft-linked, erased or external nodes), is
refused by the reader rather than guessed at; every Survarium archive from
0.1.1a to 0.26g reads.

`write_over` writes a whole archive again: every base file, with the given
files added or put in their place, for a build that mounts no patches
(v0.100b's `apply`). Files that share their bytes in the base (hard links)
share one copy in the output, as separate file nodes.

`safe_join` prevents a virtual resource path from escaping an output
directory. Absolute paths, `..`, `.`, and other non-normal components are
rejected. This is both a security boundary and a reproducibility rule: a
manifest must name resources, not arbitrary host files.

## Binary configs: the converter's central intermediate tree

Much of Vostok's data is a self-describing binary configuration. It is not Lua
text, JSON, or a C struct dump. `binary_config.rs` converts it into:

```rust
enum ConfigValue {
    Boolean(bool),
    Integer(i32),
    Float(f32),
    String(String),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    Named(Vec<NamedValue>),
    Indexed(Vec<ConfigValue>),
}
```

A `Named` value is like a JSON object. An `Indexed` value is like an array.
Unlike JSON, key names and strings are Windows-1251, scalar type identity is
preserved, and every serialized value has a fixed 24-byte descriptor pointing
to other data.

Each named key stores a CRC32. Parsing recalculates and checks it. Strings must
be NUL-terminated and their encoded length includes that terminator.

This is why a longer texture name cannot safely be pasted over bytes. The
serializer must rebuild:

- string storage;
- offsets into that storage;
- string lengths;
- table offsets;
- child counts;
- key CRCs.

`ConfigValue::set_path("a.b.0.c", value)` performs a typed tree edit.
`BinaryConfig::to_bytes()` then reconstructs a valid binary layout. The weapon
and armor manifests expose such changes as `patch` operations.

`replace_strings` is a recursive semantic replacement. It still operates on
parsed `String` values and then reserializes the entire tree; it is not an
in-place byte replacement.

The parser deliberately fails on:

- out-of-bounds offsets;
- unknown type tags;
- malformed Windows-1251;
- unterminated strings;
- key CRC disagreement;
- unreasonable nesting.

Those checks stop a damaged donor config from becoming plausible-looking
target output.

## Manifests: intent rather than procedure

The JSON files under `ports/` describe *what* to build. Rust implements *how*
each declared operation is converted.

The manifest structs use `#[serde(deny_unknown_fields)]`. A typo therefore
fails instead of becoming an ignored instruction. Each manifest also has a
`validate()` method for semantic checks such as:

- supported format version;
- non-empty asset and resource names;
- unoccupied item dictionary IDs;
- safe relative resource paths;
- complete material-template families;
- explicit reasons for deliberate omissions;
- non-empty fallback allowlists;
- valid source/target build relationships.

Important manifest families are:

### `PortManifest`

Declares a weapon and ammunition port: target IDs, source and template configs,
patches, copied resources, model surfaces, localization, and server metadata.

### `ArmorPortManifest`

Declares an armor item and a skinned-model conversion. It names surface
selection, target skeleton/bind-pose templates, material output, texture
compression, and optional weight-transfer inputs.

### `Stk2ContentManifest`

Builds the initial STK2 content slice: rigid weapons and optionally a replaced
lobby character/scene.

### `MapPortManifest`

Describes the older dependency-closure map path. It declares donor map roots,
forced donor resources, explicit overrides, and categories that can be dropped
or preserved.

### `TargetNativeMapManifest`

Describes a newer Survarium map reconstructed using explicit v0.100b ABI
adapters. It contains reviewed material-template overrides, texture aliases,
game-material translations, collision-family decisions and unsupported-object
allowlists.

### `Stk2MapSliceManifest`

Describes conversion from an STK2 editor/source tree: source project, model
archive exceptions, material templates, unresolved texture allowlists, terrain
input, spawn positions, and server project generation.

An allowlist is an auditable exception, not “ignore all errors.” The build also
checks that declared exceptions were actually used. A stale override or stale
omission fails, because it may hide a change in the source corpus.

## Build records and provenance

Every successful builder emits `port-manifest.json`. `BuiltFile` records:

- logical resource path;
- the operation/source that produced it;
- byte size;
- SHA-256.

The `source` description distinguishes operations such as:

- exact donor copy;
- exact stock reuse;
- generated config;
- static vertex-layout conversion;
- texture conversion;
- composed earlier bundle.

This provides reproducibility and lets `verify` check the final packed database
without trusting the intermediate work directory.

The term **identity-proof** in a source label means that we have established
that the donor bytes already satisfy the target consumer's exact contract for
that resource family. It does not mean “the two builds happen to parse it.”

The term **adapter** means bytes were decoded or structurally understood and
then emitted in a target-native layout.

## Overlay, bundle, composition, and application

These four terms have specific meanings in the converter:

- an **overlay** is a directory tree whose relative paths are virtual resource
  paths;
- a **bundle** is an overlay plus its build record and optional server metadata;
- **compose** merges bundles in the specified order and records provenance;
- **apply** writes the base database again with the overlay over it,
  reopens it, and verifies hashes (v0.100b); for 0.26g, content packs make a
  patch instead (`pack patch`, `docs/content-packs.md`).

Composition is ordered: later bundles supersede earlier files at the same
path. Duplicate paths within one bundle are rejected. Superseding across
bundles is allowed because it is explicit in the composition order.

The default design is additive. Existing stock resources remain unless an
output deliberately targets the same path. This is why AKMN and SKS receive
new dictionary IDs and new paths instead of replacing AK-74u.

The manifest and overlay are the authoritative source. A database is written
to `<output>.partial` and renamed when complete; a failed one can be deleted
and rebuilt.

## Resource dependency graphs

`dependency.rs` performs a breadth-first traversal starting from declared root
resources. For each successfully parsed binary config it recursively collects
strings and probes the archive for resource paths represented by those strings.

The graph has three source policies:

- `DonorRequired`: declared donor roots and explicit donor subtrees must come
  from the donor;
- `TargetPreferred`: a dependency referenced by donor content uses stock when
  the same target path exists, otherwise donor;
- `TargetOnly`: dependencies reached from stock remain stock.

That policy fixed the “mixed model directory” failure. Earlier behavior
preferred donor for every shared path. A later map bundle could replace part of
a common stock model while leaving another stock part behind. The resulting
surface/settings mismatch crashed the renderer.

**Forced donor paths** make exceptions explicit. A root can force its complete
subtree, while shared global dependencies still prefer the target.

References can be:

- exact files;
- directory-like roots;
- extensionless texture names requiring `.dds` and `.options` probing;
- project-relative paths resolved by walking parent project directories;
- instance-qualified model names with a `#[scale]` suffix;
- `resources/...` runtime references needing canonical path normalization.

`unresolved_references` is part of the report. A path-like reference must not
silently disappear just because no candidate matched.

Dependency scanning is deliberately described as best effort. Arbitrary binary
payloads can contain resource relationships that are not serialized strings.
Version-specific builders therefore add explicit family knowledge and
validation on top of the generic scanner.

## Target view, donor view, and effective view

These words occur frequently:

- **target** means the v0.100b/build 802 database whose runtime ABI must be
  obeyed;
- **donor** means a newer Survarium database or the STK2 source corpus;
- **overlay view** means only generated files;
- **effective view** means `stock target + overlay`, using the overlay at path
  conflicts.

Many validations require the effective view. A generated model may intentionally
reuse a stock material. Looking only inside the overlay would incorrectly call
that dependency missing.

Conversely, validation must not treat a stock file as proof that a deliberately
converted donor root succeeded. Declared converted roots must exist in the
overlay with recorded provenance.

## Schema, format, ABI, and contract

The code uses several related words:

- **schema**: named fields and their types in a self-describing config;
- **format**: byte-level representation such as a DDS or vertex chunk;
- **ABI**: the exact representation a compiled runtime consumer expects;
- **contract**: all cross-file and semantic requirements for one runtime
  resource family.

Self-description helps with configs but does not make versions automatically
compatible. A newer config can contain a field the old binary never knew, or
omit a field the old binary assumes exists. Non-config payloads such as vertex
buffers are not self-describing enough to infer every semantic.

An adapter is accepted only when the target contract is established through
one or more of:

- a matching stock v0.100b corpus example;
- recovered source/PDB symbols;
- verified binary consumer behavior;
- exact byte-identity across known compatible builds;
- a runtime experiment backed by offline structural checks.

“It did not crash once” is evidence, but not an ABI proof.

## Chunks and payload families

Many model payloads begin with:

```text
u32 chunk_id
u32 payload_size
payload bytes
```

The chunk ID identifies a broad data role. The payload still needs a known
stride and semantic layout. Examples used by the STK2 adapter include:

- collision vertices: chunk `0x19`, packed 12-byte `float3`;
- collision indices: chunk `0x1a`, packed `u32`;
- collision face material dictionary: chunk `0x1b`;
- collision face indices: chunk `0x1c`.

Render vertices use a declaration describing attributes and byte offsets.
Indices alone do not reveal whether the associated vertex stream is static,
colored static, or skinned.

Every reader first checks chunk ID, declared byte count, actual byte count,
stride divisibility, and index ranges. Arithmetic uses checked operations so
malformed counts cannot wrap.

## Static vertex conversion

`map_port.rs` and `stk2_content.rs` contain the proven static layouts. A vertex
declaration names attributes such as:

- `POSITION`;
- packed normal;
- packed tangent;
- packed binormal;
- UV;
- optional vertex color.

The converter does not merely copy a stride-36 stream. It checks the source
declaration and moves/widens attributes into the v0.100b layout. A declaration-
less stream with a familiar byte count is rejected because multiple meanings
can share one stride.

The ordinary static target layout and colored-static target layout differ.
Vertex color must land at the exact target offset and the declaration must
advertise it. Otherwise the GPU interprets some other bytes as color.

After conversion, geometry bounds are recomputed from positions and written
into per-surface export properties.

## Skinned vertex conversion and retargeting

`armor.rs` handles a more difficult pipeline:

```text
source skinned/static vertices
  -> source influences and weights
  -> source-to-target bone correspondence
  -> target bind-space positions/normals
  -> v0.100b hardware-skinned vertex layout
```

Relevant concepts are:

- **skeleton hierarchy**: named bones and parent relationships;
- **bind pose**: the reference transform of every bone when the mesh was bound;
- **inverse bind matrix**: converts a vertex from model space into bone space;
- **bone palette/index**: compact index stored in each vertex;
- **weight**: contribution of one bone to that vertex;
- **retargeting**: translating source bone influence into the target skeleton;
- **weight transfer**: deriving skin weights from a related reference mesh when
  the compiled donor representation lacks them.

The runtime target supports a fixed maximum influence count and a specific
48-byte hardware layout. The converter normalizes weights, validates indices,
and rejects non-finite transforms.

Maya Binary scenes can retain the original skin cluster even when a cooked
model does not. `maya_binary.rs` walks Maya's chunked IFF-like structure,
extracts influence names and weight arrays, and can compare static-model
vertices to scene vertices.

Geometry similarity is useful evidence, but nearest-point transfer is not
automatically correct. Symmetric limbs, overlapping clothing layers, and
different topology can select the wrong side or body part. That is why the
armor code tracks reference correspondence and retargeting explicitly.

## Coordinate and tangent-space conversion

Position conversion is only one part of a vertex adapter. Normals, tangents,
binormals, triangle winding, bind matrices, instance transforms, and bounds
must use the same coordinate convention.

**Triangle winding** is the clockwise/counter-clockwise vertex order used to
identify a front face. Reversing axes without correcting winding can turn a
model inside out.

**Tangent space** is the local basis in which a normal map is interpreted:
normal, tangent, and binormal. A mesh can have correct positions but incorrect
lighting if that basis was packed or transformed differently.

The converter validates finite values and recomputes bounds, but visual
comparison remains important for semantics such as handedness.

## Material conversion

`material_conversion.rs` parses donor and target material instances into a
catalog. It compares structural features rather than matching filenames alone.
Features include:

- base material/shader family;
- named field set and types;
- shader feature toggles;
- texture binding roles;
- alpha behavior;
- two-sided rendering;
- lighting model and decal behavior.

`rank-material-templates` scores target materials for a donor material. The
ranking is investigative evidence, not automatic permission to use the first
candidate.

For newer Survarium maps, `target_native_map.rs` selects a reviewed target
template and rewrites supported texture bindings. Manifest overrides cover
cases where a donor feature has no unambiguous target analogue.

For STK2, `Stk2SourceMaterialCatalog` indexes source material instances by
their exact diffuse texture. It records the base family and auxiliary roles
such as normal, specular intensity, specular power, emissive, translucency,
detail normal, and bump.

If one diffuse is used by source material instances with incompatible classes
or different auxiliary bindings, diffuse-only resolution is ambiguous. The
converter must not guess silently. `surface_material_overrides` binds a fully
qualified `library#render/surface` to an exact source material instance.
Generated materials are keyed by that source instance while the common diffuse
DDS is emitted once. Thus two surfaces can share one image while retaining
different wood, metal, alpha, or auxiliary-map contracts.

## Texture conversion

Texture conversion has three layers:

1. decode the source image;
2. transform semantic channels and generate mipmaps;
3. encode the exact v0.100b DDS plus matching `.options`.

`armor.rs` and `stk2_content.rs` share target-side validation ideas but handle
different source contexts.

Supported output families currently include DXT1 for opaque/no-alpha color,
DXT5 for alpha-bearing color, and the target DXT5 normal-map convention.
Source `DXT1a` is deliberately transcoded to target DXT5: ImageMagick's DXT1
writer discarded its one-bit alpha and turned foliage cards into opaque
rectangles. The converter now rejects a DXT1a transcode whose DXT5 blocks
contain no non-opaque alpha samples.

Validation reads the DDS header and cross-checks:

- magic and header size;
- width and height;
- FourCC;
- mip count;
- compressed payload length;
- target trailing sRGB byte;
- `.options.format`;
- `.options.has_mip_levels`;
- `.options.srgb`.

The old engine's BC mip-chain convention is narrower than a generic DDS
writer's convention. v0.100b stops when either dimension reaches one complete
4x4 compression block: 1024x1024 therefore has 9 levels and 2048x512 has 8.
ImageMagick normally continues with encoded 2x2 and 1x1 blocks. Leaving those
levels in place, keeping ImageMagick's producer tag in the reserved header, or
writing only the top-level byte count into `dwPitchOrLinearSize` can make the
old texture cooker substitute its visible `missing texture` fallback even
though the DDS and `.options` paths exist. The converter trims the chain,
clears reserved fields, sets `DDSD_LINEARSIZE`, records the complete retained
payload plus Vostok's sRGB byte, and validates the resulting native contract.

An auxiliary texture plan carries both its source role and target material
property. The plan ensures, for example, that a source normal texture is
converted as a normal map and bound to `texture_normal`, not accidentally
compressed and sampled as diffuse color.

## Model settings and the one-to-one surface rule

A rendered `.model` consists of several sibling resources. Its settings contain
`material_settings` and an LOD hierarchy listing surface names. Each physical
render surface directory contains converted geometry and export properties.

The fundamental invariant is:

```text
surface names in settings
    == surface directories containing render geometry
```

Each settings entry must also bind a valid material. Additional LOD surfaces
must remain consistent with the LOD hierarchy.

`model_validation.rs` builds the effective target-plus-overlay view and checks
this invariant for every rendered model. It also has a narrowly scoped
normalizer for additive overlay settings, but normalization is not a substitute
for identifying an incompatible partial donor replacement.

The water-tank crash was a direct violation: geometry exposed three surfaces
while replaced settings declared two. The renderer selected `nomaterial`,
entered its fallback path, and eventually dereferenced a missing shader
resource.

## Collision conversion and game-material remapping

Collision is an atomic family. Vertices, indices, face dictionary, face indices,
and parent settings must be translated together.

Numeric physical-material IDs are build-local. The converter therefore:

1. reads a donor material name;
2. applies an explicitly reviewed name override if needed;
3. resolves that name in v0.100b `game_materials/game.materials`;
4. writes the target numeric ID;
5. rejects an unknown or invalid ordinary-collision binding.

`target_native_map.rs` contains a collision-binding preflight. It gathers
evidence before writing output and records whether a binding came from:

- exact stock correspondence;
- donor name translated into target;
- a manifest override;
- a deliberately accepted fallback.

Walker collision is treated differently because recovered v0.100b behavior
uses a literal zero fallback for many stock walker materials. That exception
must not be generalized to ordinary collision.

STK2 exported low-collision models retain vertices and indices but not the
original per-face material dictionary. The STK2 manifest must therefore name a
real model-level target physical material. Terrain uses a separately declared
physical material. This is a documented fidelity loss, but unlike ID `65535`
it produces a valid target contract.

## Terrain conversion

`Stk2TerrainCell` is an intermediate representation containing:

- cell transform;
- row size;
- source vertices/heights;
- layer texture names;
- per-triangle/layer information needed to group render output.

The converter:

1. parses terrain cells from the STK2 source directory;
2. resolves declared texture aliases;
3. builds one texture/material plan per layer;
4. generates target static render surfaces per used layer;
5. generates a triangle collision mesh;
6. generates target model settings and bounds;
7. places the terrain model as a client-project visual.

Terrain aliases retain the original target layer path but use a reviewed
alternative source payload. This keeps project bindings stable while making
the substitution visible in the manifest.

Current terrain conversion is intentionally incomplete. It does not imply full
support for every STK2 blend, vegetation, anomaly, lighting, or editor-only
terrain feature.

## STK2 project parsing and composites

`stk2_project.rs` reads the Lua-style editor project and emits `ProjectObject`
values. A project object records:

- object kind;
- library/model name when present;
- position, rotation, and scale;
- whether the instance came directly from the project or through a composite.

A **composite** is a reusable group of child objects. Parsing only direct
objects would omit much of a map, so the parser also reads
`composite.library`, expands children, and combines parent and child transforms.

The parser produces `ProjectParseReport` counters and an unresolved-composite
list. Unresolved composite children are a build failure because silently
dropping them can remove whole building sections.

Not every project object kind has a target adapter. Direct point, spot, and
sun lights are now a useful example of a real adapter: the converter selects
a v0.100b template with the same `light_type`, preserves the target field
types and target-only defaults, then projects compatible STK2 values and the
map-relative transform onto it. It does not copy the newer object bytes.
`z_bias` and `shadow_transparency` demonstrate why: STK2 stores fractional
floats while v0.100b stores integer policy values, so the old template values
must remain until those semantics are recovered.

The environment object follows the same principle. Its object layout remains
target-native, while the manifest chooses an existing v0.100b post-process
material. Spatial lights, probes, and decals inherited from a convenient
client-project template must be removed because their coordinates belong to
the template map, not the converted map.

The build report still counts omitted particles, source decals, wires,
SpeedTree instances, dynamic visuals, skeleton objects, and other categories.
“Parsed” and “ported” are therefore distinct states.

## Three map-building paths

The converter currently contains three map strategies. They should not be
confused.

### Dependency-closure map (`build-map`)

This path starts with a donor runtime project, selects declared donor roots,
and copies a dependency closure while preferring stock shared resources. It is
useful for close versions and investigations, but byte compatibility still
needs proof for every imported family.

### Target-native newer map (`build-target-native-map`)

This path decodes a newer Survarium project into understood categories and
re-emits v0.100b structures. It handles:

- project object tables;
- instance path normalization;
- static and colored-static render payloads;
- material translation;
- DDS/options verification;
- collision material remapping;
- server project generation;
- explicit category drops.

Its code is deliberately strict and evidence-heavy. It is the model for
bringing v0.20/v0.23 maps back without donating opaque global resource trees.

### STK2 source map (`build-stk2-map-slice`)

This path does not begin with a newer runtime database. It interprets editor
projects, exported model archives, source TGA/options, source material
instances, and terrain files. It then constructs a new target project.

The STK2 path has more source meaning available in some places, but less cooked
runtime information in others. Low collision lacking face materials is one
example.

## Map structure comparison

`map_compare.rs` is independent of the builder. It decodes two project trees
and builds separate resource graphs, then reports:

- section presence and value kinds;
- array/table shapes;
- numeric ranges;
- resource nodes and family counts;
- target-only and donor-only paths;
- shape/type divergences;
- unresolved dependencies.

The distinction between **structure comparison** and **conversion** matters.
The comparator tells us where versions differ; it does not automatically
decide how a difference should be adapted.

Likewise, identical high-level trees do not prove that referenced binary model
payloads share an ABI. The project can be structurally equal while a vertex
stream or shader contract differs.

## Client project and server project

The client project contains presentation and physical world references:

- visual model instances;
- collision objects;
- terrain;
- camera/post-processing-related data;
- client-side effects and scene categories.

The server project contains gameplay-side objects:

- respawn points;
- team bases;
- victory-item spawners and containers;
- server collision/gameplay categories;
- match-specific project identity.

Porting only the client project can show a map but leave the match unusable.
Porting only the server project can let matchmaking start while the client
cannot load the world.

The map builders use a known v0.100b server template, replace positions and
required categories, and give the generated client/server pair stable project
paths.

The presentation `lobby_scene` is not itself a normal playable multiplayer
level. `build-playable-lobby` clones its useful presentation scene under a new
project path and creates matching server data instead of mutating the menu
lobby.

## Items, dictionary IDs, localization, and SRP

An item is more than a model:

- gameplay config;
- visual/model resource;
- dictionary entry with a numeric ID;
- localization keys;
- inventory icon/atlas reference;
- server catalog declaration;
- compatibility relationships such as weapon-to-ammunition;
- an owned inventory instance at runtime.

Dictionary IDs are global within a build. The converter checks that a new ID is
free before insertion. A new weapon receives a new ID rather than impersonating
an old weapon.

Localization tables use target encoding and keyed entries. Russian strings are
added through parsed configs; locale selection at runtime is a separate client
setting.

Generated `server/srp-content.json` records additive server content. The
v0.100b SRP branch may still require compiled Rust changes because that old
server predates generic runtime ingestion of the manifest.

Equipping is server state. Rendering the equipped weapon is client resource
state. A weapon can exist in inventory but have a missing icon, equip correctly
but show a missing material, or render correctly but use incompatible reload
animations.

## Validation layers

The porter uses several layers because no single test is sufficient:

### Parse validation

Can the serialized bytes be decoded without invalid offsets, types, encodings,
or chunk lengths?

### Local format validation

Does one resource satisfy its own rules: valid DDS header, finite vertices,
indices in range, normalized weights, known vertex declaration?

### Cross-resource validation

Do surfaces match settings, collision names match game-material entries, and
texture options match DDS metadata?

### Graph validation

Does every required reference resolve in the effective database? Are
unresolved exceptions explicitly declared?

### Provenance validation

Does every generated path have one recorded producer, size, and hash?

### Packed-output validation

After packing, can the database be reopened and does every emitted resource
hash to the manifest value?

### Runtime validation

Can the retail binary load the lobby and match, render the content, collide
with it, animate it, and react to gameplay interactions?

Runtime testing is the final consumer check, but it is expensive and can enter
undefined behavior. Offline checks are designed to fail before the retail
binary encounters a malformed combination.

## Common Rust patterns in the code

The source uses a few recurring patterns:

- `anyhow::Context` adds the resource path and operation to errors;
- `bail!` rejects an invalid contract immediately;
- `BTreeMap`/`BTreeSet` make output deterministic;
- `checked_add`, `checked_mul`, and `try_from` prevent corrupt sizes from
  overflowing;
- `Option` represents genuinely absent optional evidence;
- `with_context` turns a low-level parse failure into a useful asset-level
  diagnosis;
- `serde(default)` makes an optional manifest collection empty;
- `deny_unknown_fields` makes configuration typos fatal;
- `pub(crate)` shares conversion helpers inside the crate without making them
  part of a public library API.

The builders generally follow this order:

1. validate the manifest;
2. prepare a new empty output;
3. open target and donor data;
4. perform preflight/discovery;
5. generate or copy each resource;
6. record it immediately;
7. sort and reject duplicate outputs;
8. validate the effective overlay;
9. serialize the build record.

## Suggested source-reading order

For a first pass:

1. read `archive.rs` to understand resource paths;
2. read the `ConfigValue` enum and `set_path` in `binary_config.rs`;
3. read the relevant manifest struct in `manifest.rs`;
4. read `weapon::build_bundle`, which is the smallest complete builder;
5. read `record_file`, `compose_bundles`, `apply_bundle`, and `verify_bundle`;
6. read `model_validation.rs`;
7. read `dependency.rs`;
8. read `stk2_content.rs` for our current STK2 work;
9. read `target_native_map.rs` after the smaller builders—the strict map
   adapter is intentionally the densest module;
10. use `main.rs` as an index of available investigative commands.

When following one generated resource, search for its `source` label from
`port-manifest.json`. Labels such as `generated:STK2-terrain-settings` or
`adapter:collision-face-data` lead directly to the function that wrote it.

When following a crash, start with the failing runtime object and walk its
contract backwards:

```text
shader failure
  -> material instance
  -> material settings binding
  -> model surface
  -> project instance

impact-decal failure
  -> game-material pair
  -> numeric target game-material ID
  -> model settings material name
  -> collision face dictionary/index
  -> collision triangle hit by the ray
```

That approach is much faster than searching every texture or copying a whole
donor folder.
