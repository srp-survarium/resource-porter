# Porting Shadow of Chernobyl's ladder climb to v0.100b

What exists at each end, what the converter does, and what is still missing
before a player can climb anything.

## Why this is possible at all

v0.100b **has a ladder subsystem** — `game_core/ladder.{h,cpp}`,
`ladder_cook.cpp`, `landing_point.h`, and the strings and symbols are in the
retail client binary (`ladder_cook`, `main_animation`, `landing_points`,
`st_use_ladder`, `st_do_not_use_ladder_from_this_side`). A ladder is a
`usable_object` cooked from project config:

| field | meaning |
| --- | --- |
| `main_animation` | one `animation_class` path — the climb loop |
| `landing_points[]` | `position`, `rotation`, `start_animation`, `end_animation` |
| `collision_geometries` | what the player bumps into |
| `occlusion_geometries` | the "wrong side" blocker, optional |

The cook refuses to create the object when `main_animation` is missing or when
every landing point animation is empty. Nothing shipped uses it: no retail
project declares a `ladder` object and the database carries no climb clip. The
missing piece was never the code — it was the animation.

**And it cannot be used.** `use_ladder` is pure virtual on `collision_user`,
and every implementation is empty: `player::use_ladder` is one byte in the
retail binary — a bare `ret`, matched byte-exact against the reconstruction's
`NOT_IMPLEMENTED`, which compiles to nothing in MASTER_GOLD — and the other
override in `anchor_game_core.cpp` is `{ }`. So `ladder::use_initialize` calls
into nothing. A ladder placed in a map would cook, block movement, offer
`st_use_ladder`, and do nothing when used.

Nothing in the resources can fix that; it is missing client code. **Do not port
a ladder into a map expecting it to work**, and read this paragraph before
starting any follow-up: the animations below are real and correct, but there is
no ladder gameplay in v0.100b to spend them on.

## What each donor has

SoC ships the climb, under a prefix a keyword search does not find:
`ActorAnimation.cpp SActorState::CreateClimb` builds the set from `cl_`.

| motion | role | frames |
| --- | --- | ---: |
| `cl_run_fwd_0` | climb up | 26 |
| `cl_run_back_0` | climb down | 26 |
| `cl_run_ls_0` / `cl_run_rs_0` | shuffle sideways | 26 |
| `cl_idle_1` | hang idle | 26 |
| `cl_torso_0_aim_0` | torso layer | 26 |

They live in `meshes/actors/stalker_animation.omf`, `mp_stalker.omf`,
`stalker_animation_old.omf` and `barman_animation.omf`. There is no separate
mount or dismount clip — SoC simply enters and leaves the state.

The STK2 2011 leak has neither the animation nor the code: its complete
animation vocabulary is 799 stems of locomotion, turns, sits, camp idles and
weapon handling, and its engine tree (6,235 files) never mentions climbing.

## The two ends of the conversion

**Source.** `soc::codec::omf` decodes a motion bank: the partition table names
the bones and fixes their order in the key stream, and the motion chunks carry
quantised keys — rotations as four `s16` scaled by `1/32767`, translations as
`s8` or `s16` against a per-track size/init pair, with a single key standing in
for a channel that never moves. `soc::codec::skeleton` reads the actor `.ogf`
for the hierarchy and rest pose (`OGF_S_BONE_NAMES`, `OGF_S_IKDATA`).

**Target.** `skeleton_animation` writes the clip pair. See
`docs/engine-resource-primer.md` for the container; the shape is

```text
u16 bones_count, u8 event_channels_count, u8 animation_type
bones_count    * 0x48 bytes    -- nine pointer slots, overwritten on load
per bone, per channel: u32 knots_count, knots_count * (f32 knot, f32 value)
```

with nine channels per bone: translation xyz, rotation xyz **as Euler angles**,
scale xyz.

The pairs are not samples. They are a non-uniform cubic B-spline's knot vector
and control points, which the engine turns into per-span cubics on load. So the
writer reimplements the engine's own construction (`get_spline_params`,
`create_in_place_internals`) and solves for control points against *that* basis.
Fitting against a textbook basis would be a different curve. Measured residual
across the six clips: **7e-6**, in radians and metres.

Both engines sample at 30 fps (`SAMPLE_FPS`, `default_fps`) and both express
clip time in frames, so frame *n* of the source is frame *n* of the result and
nothing is resampled. One extra sample is appended to close the loop, because
X-Ray wraps frame N onto frame 0.

## The two rig differences that matter

Neither is optional; both are silent if skipped, because a clip that fits its
own wrong poses still reports a perfect residual.

**The rigs face opposite ways.** X-Ray's left limbs sit at +X with toes along
-Z; v0.100b's left limbs sit at -X with toes along +Z. The measured yaw is
**179.97 degrees**, solved from the rest poses rather than assumed, and every
transferred rotation is conjugated by it. Skip it and the port comes out with
its arms swapped.

**They rest in different poses.** SoC rests a stalker near a T-pose — hand
0.72 m out, 0.39 m above the hip — while v0.100b rests its character in an
A-pose, hand 0.54 m out and level with the hip. A rest-relative transfer
assumes the two rest poses *are* the same pose, so straight across, every
ported arm sits about 35 degrees low. `matched_rest` therefore swings the
target rest onto the source's first: parents-first, each mapped bone turns by
the minimal rotation that aims it at its child the way the source aims at the
matching one, and its subtree follows. Only orientation moves — bone lengths
stay the target's. The reference child is declared per bone in `REST_CHAIN`,
because a bone with several children (the chest has a neck and two clavicles)
has no single direction, and the hip is deliberately excluded: it is the root
of every chain, and aiming it at a thigh would rotate the whole character.

With both applied, every mapped bone of `cl_run_fwd_0` frame 0 lands within
7–15 cm of where the source pose puts it, which is the proportion difference
between a stalker and a Survarium character.

## Bone coverage

X-Ray's 45-bone `bip01` skeleton drives 43 of the target's 69 bones. The 26
that hold their rest frame have no source: the twist and roll bones, the knee
helpers, the shoulders, the ring and little fingers, and the `Pistol`/`Weapon`/
`Item` attachment points. X-Ray numbers its three fingers `finger0`/`1`/`2`,
mapped here to thumb, index and middle.

## Running it

```sh
SOC="$HOME/.local/share/Steam/steamapps/common/STALKER Shadow of Chernobyl"
TARGET=$(nix build --no-link --print-out-paths ../vostok#survarium.resources)/resources.db

vostok-port soc-probe-motions --source-root "$SOC" \
  --bank meshes/actors/mp_stalker.omf --prefix cl_
vostok-port soc-probe-motions --source-root "$SOC" \
  --bank meshes/actors/mp_stalker.omf --motion cl_run_fwd_0
vostok-port soc-probe-skeleton --source-root "$SOC" \
  --model meshes/actors/bandit/stalker_bandit_1.ogf --bank meshes/actors/mp_stalker.omf

vostok-port soc-retarget-motion --source-root "$SOC" --target-db "$TARGET" \
  --bank meshes/actors/mp_stalker.omf \
  --model meshes/actors/bandit/stalker_bandit_1.ogf \
  --motion cl_run_fwd_0 --compare-frame 0 \
  --output .work/clips/soc/ladder/cl_run_fwd_0
```

`--compare-frame` is the check that matters. A residual only says the curves
reproduce the poses they were given; the comparison says whether the right
poses were asked for, by printing where each mapped bone lands in both rigs.

## State of validation

Verified: all six motions decode; the emitted `.b-spline` re-parses byte-exact
under an independent parser with the same 70-bone table and stream layout retail
uses; `.bones_names` decodes to the retail schema at the same size; curve
residual 7e-6; frame-0 pose within 7–15 cm of the source across every mapped
bone.

Not done, and none of it is optional before anything can be climbed:

* the clips are written to files, not into a bundle at resource paths;
* no map declares a `ladder` object, so nothing binds `main_animation`,
  landing points, or collision geometry;
* mount and dismount clips do not exist in either donor and have to be
  authored or faked from `cl_idle_1`;
* `player::use_ladder` is unconfirmed in the retail binary;
* nothing has been seen in the client.
