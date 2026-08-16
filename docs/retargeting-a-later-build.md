# Retargeting a later Survarium build

Everything this port does is aimed at **v0.100b, build 802**. Several of its
compromises are not properties of the port or of Shadow of Chernobyl — they are
properties of that specific build, and a later client may simply not have them.

This is the re-test list: what was found to be missing or fixed-in-place, how it
was established, and the cheapest way to check whether a new target still
behaves that way. Each entry names the evidence so the check can be repeated
rather than taken on trust.

The habit that produced all of it: **read the engine before forming a theory**.
`../vostok/sources` is reconstructed and readable, and every item below was
settled in minutes there after hours of guessing at it from the outside.

## Footstep sound does not follow the surface

**Status in 802:** the material under your feet is ignored.

`game/sources/step_manager.cpp` receives a `closest_ray_result` carrying
`triangle_index` and `is_shape_index`, and then does not use them:

```cpp
get_pair( ray_result.object->get_collision_group( ), foot_material_id );
```

`get_collision_group()` returns Bullet's broadphase filter group
(`m_collisionFilterGroup`), not a material. `game_core/sources/bullet.cpp` on
the same struct does it correctly:

```cpp
u16 game_material_id = body->get_triangle_material( ray_result.triangle_index, ray_result.is_shape_index );
get_pair( m_bullet_material->id( ), m_collided_material->id( ) );
```

So the capability exists and footsteps just miss it. `step_manager::on_step` is
the only footstep emitter in the tree.

**Consequence:** binding `water` to submerged collision does nothing audible.
This was tried, shipped correctly, changed nothing, and was reverted.

**Re-test:** grep the new build for `get_triangle_material` and check whether
`step_manager` is among the callers. If it is, a converted map's footsteps will
follow its materials with no porter change at all — the bindings are already
emitted.

## There is no underwater state

**Status in 802:** absent entirely.

`swim`, `submerg`, `wading`, `water_level`, `water_height`, `underwater` and
`in_water` appear nowhere in the source except one NPC `die_in_water` sound
collection. No water volume object type, no water level, no camera-in-water
test.

**Consequence:** standing in a pool produces an unfogged halo around the
first-person model, which is drawn forward and occludes the water surface
behind it. Modelling water as a box — the shape v0.1.1e uses — does **not** fix
this, and would most likely be back-face culled from inside; the material
instance exposes no cull flag, that belongs to the base effect.

**Re-test:** the same grep. If a water volume object type appears in a later
build's project schema, converting X-Ray's water plane into one becomes the
right shape, and both the halo and the footstep problem resolve together.

## The map's only ambient is a world-height ramp

**Status in 802:** `stage_ambient_lighting` starts at
`set_ambient_multiplier( 0.0f )`, and the level template authors no ambient
volumes and no usable probes, so sky light is all there is:

```cpp
set_ps_constant( m_c_skylight_parameters1,
    float4( skylight_lower, skylight_upper - skylight_lower, skylight_power, 0.0f ) );
```

`skylight_lower`/`skylight_upper` are world heights read from the post-process
material, and `level_03_day` sets `0..5` for terrain running y = -0.6..3.97.

**Consequence:** every converted map emits its own post-process material with
that band re-anchored under its own geometry. See `the porting guide`.

**Re-test:** check whether the new build's post-process schema still exposes
`skylight_lower`/`skylight_upper` at `material.post_process`, and whether its
level templates author `ambient_volume` or usable `environment_probe` objects.
If probes exist, converted maps could carry real indirect light instead of a
single ramp — the biggest available lighting improvement.

## Water is procedural, and its probe is missing

**Status in 802:** `material_instances/water/water_green.material` is a real
water shader (`forward_simple_water`: wave power, depth fog, local reflections)
but has no `g_stage`, so it takes no diffuse, and it references
`environment_probes/untitled_project/probe_50043953794066488`, which the
shipped database does not contain.

**Consequence:** the `Water` family binds nothing from the source and repoints
the probe via the `water_reflection_probe` manifest setting.

**Re-test:** whether the referenced probe exists, and whether a later water
material accepts a diffuse or a per-map fog colour. `water_fog_color` currently
comes from the donor, so every ported map's water is the same green.

## No character animations at all

**Status in 802:** 1,406 animation files, every one a weapon animation.

**Consequence:** ladders are not ported. Cordon has 14 `obj_climable` entities
and `mp_rostok` 31, and X-Ray's climbing is animation-driven, so there is
nothing to play. `flags::CLIMABLE` is decoded and recorded but unused.

**Re-test:** if a later build ships character animations, ladders become worth
porting — the source data is already decoded.

## Project reference conventions

**Status in 802:** the two arrays of a client project name the same model
differently.

```
render_visuals[].lib_name     props/light/green_lamp_with_wire
collision_objects[].lib_name  resources/models/props/light/green_lamp_with_wire.model/collision
```

Writing the render shape into the collision array made every converted map fail
to cook and crash on the unchecked result. `model_reference_like` reproduces
whatever conventions the template value shows — `resources/` root, `models/`
prefix, `.model` extension, sub-resource suffix — so it should survive a schema
change, but the *set* of arrays and their field names may not.

**Re-test:** dump a level project from the new build and compare its top-level
keys and per-entry fields against `ProjectBindings` in the manifest. The port
declares every path it writes, so a mismatch is a manifest edit rather than a
code change.

## Collision is many small models

**Status in 802:** the largest collision mesh anywhere in the retail database is
6,911 triangles, and a map's collision is dozens of per-object models.

**Consequence:** `collision::split` bisects a level-sized `cform` spatially and
each piece rides inside a render archive.

**Re-test:** whether the new build ships larger collision models. This was
never proven to be a hard limit — the crash it was written for turned out to be
the `lib_name` bug — so the splitter may be unnecessary rather than merely
conservative. It is kept because it matches retail's shape and because Cordon's
1,375,470 triangles in one mesh is untested at any build.
