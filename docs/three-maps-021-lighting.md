# Shared day-map lighting compatibility

`examples/lighting-021-pack.rs` makes the small shader pack described by
`ports/three-maps-021-lighting.json`. It reuses the fingerprinted original
shader adapters; it does not rebuild the map packs or change individual
objects. Version 3 has 88 guarded shader replacements:

The acceptance target is a close match using stock 0.21 values and as few
renderer changes as practical, not pixel identity. The owner approved the
@3 Rudnya interior comparisons as the desired standard on 2026-11-17.
Further changes require a noticeable compatibility defect, not merely a
small numerical image difference. School, Fort and native-map isolation
still require their final review.

| Pass | Resources / scope |
| --- | --- |
| Geometry fog | 1, original arithmetic and authored Rayleigh colour |
| Tone curve | 4 supported native variants, original dark-scene behavior |
| Sky and atmospheric lookup | 2, original sky and scattering equations |
| Clouds | 8, original lit/unlit programs |
| Ambient occlusion | 1, original quality-1 generator normalized for native application |
| God rays | 4, original radial falloff; native mask and application remain |
| Sun lighting | 4, original direct-sun diffuse/specular block; native foliage block remains |
| Particle fog | 24 supported batched variants, original fog interpolation |
| Water fog | 8 supported variants, original fog; native water surface lighting remains |
| Point/spot lighting | 32 programs, original diffuse/specular and shadow sampling; native shadow-map generation remains |

Build the example under `nix develop`, then run:

```sh
cargo build --release --example lighting-021-pack
target/release/examples/lighting-021-pack \
  ports/three-maps-021-lighting.json DONOR_DB STOCK_DB OUT LIVE_PATCHES...
```

Pass the published patches in mounting order. Both archives are pinned by
hash, and each shader adapter refuses an unknown native/source program.
Every replacement records the current layered native file's hash. The pack
requires the existing School/Fort and Rudnya map packs, and leaves their
files alone. The content manifest, rather than a hand-built patch directory,
composes it with those packs and the environment edit lists.

The environment lists select a whole day map using `mie_fog_base_height`:

| Donor project | Marker |
| --- | --- |
| `level_01/day` | -32281 |
| `level_02/day` | -32195 |
| `level_05/day` | -32198 |

Every day-mode alias of a level uses its same marker. Original authored fog
inputs must be preserved when enabling it. Other scenes keep the native
shader instruction bodies. These markers follow the existing adapter's
`-32021 - map_id` convention and are runtime selectors, not visual tuning
parameters.

`tools/content/three_maps_021_environment.py` generates the two source edit
lists from the donor and canonical map packs. It verifies authored numeric
colour/intensity fields have not been calibrated in the packs, preserves
them, converts cloud coordinates/defaults, and supplies original defaults
for omitted height-ambient colours. Original constructor 0x215157 initializes
low RGBA at +178h to (0,0,0,1), high RGBA at +188h to (1,1,1,1); the optional
loader at 0x2157F7 leaves them alone when their keys are absent. Filling these
from the native map instead introduced its colours into Rudnya and School.

The separate executable hook restores multiplicative ambient-light semantics
above intensity 1. This pack addresses original fog arithmetic and the tone
curve, whose native dark-scene luminance floor differs from the original.
It changes no geometry, local light intensity, material finish or gameplay.
Probe shading, remaining native permutations and the
parts named in the table are not claimed to be transplanted. Visual acceptance is recorded in the
Skhid three-map review, not inferred from successful shader generation.

The native fallback check reads every built shader, resolves only the explicit
marker branches to their unmarked path, and compares all executable
instructions with the pinned native input. It permits only the additional
scratch registers/static-globals declaration (and AO's expanded static buffer)
needed by the guarded branch. Run it with:

```sh
python3 tools/content/check_021_shader_fallback.py \
  PORTER NATIVE_DB PACK ports/three-maps-021-lighting.json CACHE_DIR
```

All 88 resources passed against the stock archive used for this pack. This
checks native fallback instructions; final rendering review remains separate.

For local lights, `src/weather_fog_021/local_lights.tsv` pins every source/target
pair. The adapter verifies reflected resource names and used constant offsets
and sizes, remaps only binding slots, and replaces clip-XY/W input with native
screen UV reconstruction. Its original arithmetic, including shadow sampling,
is otherwise copied verbatim from the user's donor archive. Each guarded
branch returns before the unchanged native fallback.

An independent disassembly check compares all three marked branches against
each original point/spot program, permitting only these binding/input changes:

```sh
python3 tools/content/check_021_local_lights.py \
  PORTER DONOR_DB NATIVE_DB PACK ports/three-maps-021-lighting.json CACHE_DIR VKD3D_COMPILER
```

All 32 pairs passed; the final source reproduced the complete @3 pack exactly.
Clippy retained the 44 pre-existing library warnings and introduced none.
