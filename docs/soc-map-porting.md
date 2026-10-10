# Converting Shadow of Chernobyl maps

Target: Survarium v0.100b / build 802. Source: a retail Shadow of Chernobyl
installation or an unpacked `gamedata` tree. Thirteen reviewed manifests are
included under `ports/soc-*.json`. The same source reader also supports the
Clear Sky Training Camp manifest.

## Inputs and build

Supply the source installation and an untouched target `resources.db`. The
reviewed build 802 database has SHA-256
`160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409`.

```sh
vostok-port build-soc-map \
  --manifest ports/soc-cordon-to-0100b.json \
  --source-root /path/to/STALKER \
  --target-db /path/to/v0100b/resources.db \
  --output .work/cordon-bundle
```

The reader opens the retail archives directly. An unpacked source tree is
detected automatically. Use the [README](../README.md#pack-and-verify) to
compose, apply and verify bundles.

## Converted and inherited resources

The porter converts static geometry, tree placement transforms, source UVs,
collision and physical material assignments. Visuals are merged by shader
and split to fit the target's 16-bit index space. Spatially split collision
pieces live inside model archives alongside render geometry.

Diffuse textures and available `_bump` normal maps are converted. X-Ray stores
normal X in alpha and Y in blue; the target reads X from alpha and Y from
green. The converter swaps green/blue and recompresses to DXT5nm. DDS mip
chains stop at the last complete 4x4 block, and the container gains the
target's colour-space byte. Normal texture options use `NORMAL_MAP`.

Terrain colour maps, tiled detail layers and source masks where available
are assembled using the target's terrain shader. Other materials also use
reviewed Survarium templates: diffuse and normal bindings are replaced where
supported, while other texture slots and shader switches inherit the template
unless explicitly overridden. X-Ray's `_bump#` height/gloss files are not
converted.

The default map build drops lightmaps and baked vertex lighting, using the
target's sun, sky, environment, fog and post-processing. Environment and fog
anchors and the skylight height band are adjusted to the map. Water uses
Survarium's water material over the source geometry.

Default static map builds do not recreate source particles, sounds, gameplay
entities or LOD impostors. Ladder animation conversion is a separate
[experimental path](soc-ladder-animation.md). Instant-kill collision materials
retain footing without their damage behaviour.

## Resource contracts

- Render and collision arrays use different model-reference conventions;
  generated references preserve the target's prefixes and suffixes.
- Geometry is in world coordinates, so cloned instance placement is cleared.
- Model settings name exactly the emitted surfaces. Each surface appears
  once in the LOD hierarchy; unused target levels remain empty.
- Each model archive's bounds are computed from its own geometry.
- The retained directional sun has `lib_name == "sun"`; positional template
  lamps and local probes are dropped.

Unsupported shaders and physical materials fail with the source name. Inspect
the source before adding an override or changing a classification.

## Inspect and diagnose

```sh
vostok-port soc-probe-level --source-root /path/to/STALKER --level all
vostok-port soc-audit-shaders --source-root /path/to/STALKER --level all
vostok-port soc-spawn-candidates \
  --source-root /path/to/STALKER --level testers_mp_bath
vostok-port soc-probe-ground \
  --source-root /path/to/STALKER --level testers_mp_bath --x 19.43 --z 16.27
```

Authored `zone_team_base` points are preferred spawn candidates. Maps without
them use a collision heuristic; confirm the candidate's floor height with the
ground probe. An SRP map profile must use the same spawn as the manifest.

The thirteen-map SoC release has been loaded and walked in the build 802
client. This validates basic loading, collision and spawning, but does not
establish that lighting or material appearance matches the source game.
