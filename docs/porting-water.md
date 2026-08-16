# Porting water

Water is the clearest case in this pipeline of a surface whose appearance does
not live in its data. Everything the converter normally relies on — the mesh,
the UVs, the texture — is present and correct for X-Ray water, and reproducing
all three faithfully still produces something that does not look like water.
This is what was wrong, how it was established, and what replaced it.

## What the source actually contains

`mp_pool`'s water, measured with `soc-surface-stats`:

```
shader_0016_part_000  shader=effects\water  tris=7296
  y = -2.63..-2.63          one flat height
  up=6933  down=38  side=0  no vertical faces at all
  u = -1.0..2.0  v = -0.9..1.9
```

Three facts follow.

**It is a flat plane, not a volume.** There are no side faces. Any theory that
the "sides" of a water box were being drawn as its top is ruled out by the
geometry: there are no sides to draw.

**It is finely tessellated.** 7,296 triangles for a flat quad is absurd for a
static surface, and that is the first clue about the original renderer: the
mesh is subdivided because something moved it.

**Its UVs are nearly meaningless.** Three UV units across 42 world units is one
texture repeat every fourteen metres. No level artist chooses that for a
surface whose texture is meant to be seen.

The texture itself is fine. `water\water_studen` ("studen" — jelly, aspic) is
1024×1024 DXT5, `caps2 = 0` so a plain 2D image, not a cubemap: murky green
algae water, tileable, with a real alpha channel (0–255, mean 216) that is
opaque where the algae is and clear between. Nothing about it is wrong.

## Why it looked wrong anyway

X-Ray's water does not get its appearance from any of that. From
`res/gamedata/shaders/r2/water.vs`:

```hlsl
float4 P = v.P;
P = watermove(P);                                    // displace the vertices
o.tbase  = unpack_tc_base(v.uv, v.T.w, v.B.w);
o.tnorm0 = watermove_tc(o.tbase * W_DISTORT_BASE_TILE_0, P.xz, W_DISTORT_AMP_0);
o.tnorm1 = watermove_tc(o.tbase * W_DISTORT_BASE_TILE_1, P.xz, W_DISTORT_AMP_1);
```

The vertices are displaced as waves — which is what the 7,296 triangles are
for — and the visible detail comes from *two normal maps whose coordinates are
offset by world position and animated per frame*. The base texture supplies a
colour wash underneath. That is why a repeat every fourteen metres was
acceptable: nobody was ever meant to see the base texture's structure.

Port that faithfully and the motion is gone, leaving the colour wash magnified
fourteenfold. It reads as a few smooth green lobes, or — once tiled up to
compensate — as a regular grid of algae bands. Neither is water.

## What was tried first, and why it was wrong

Two attempts, both recorded because they are the tempting ones.

**Scaling the UVs.** A `family_uv_scale` manifest setting multiplying the
texture coordinates, set to 8 for the translucent family. This does exactly
what it says and does not help: it trades a texture too large to read for one
that visibly repeats. The field remains in the manifest because a genuinely
mis-tiled surface is a real defect class, but water is not one — using it here
was substituting a fudge factor for a missing feature.

**Choosing a better glass.** Water was classified `Translucent` and given the
`tiled/window/glass_tile` template, which pushes the *window pane* image into
seven texture roles at once — normals, fresnel, roughness, specular and
transparency. Switching to plain `tiled/window/glass` was a genuine improvement
and is kept for real glass, since that material uses `editor/default_nmap` and
`editor/default_spec` neutrals for the roles it does not want. But no glass
template makes a static texture look like water.

## The fix

v0.100b has its own water shader, unused by any converted map:
`material_instances/water/water_green.material`.

```
material.instance.base_material              forward_simple_water
material.forward.effect.effect_id            forward_default_simpe_water
material.forward.effect.wave_power           0.05
material.forward.effect.wave_fog_density     0.9
material.forward.effect.water_fog_color      [0.193, 0.239, 0.136, 1.0]
material.forward.effect.use_local_reflections true
material.forward.effect.texture_reflection   environment_probes/untitled_project/probe_50043953794066488
```

It animates, fogs by depth and reflects locally — the target's equivalent of
everything `water.vs` was doing. So the port stops converting water's
appearance and hands the geometry to the engine instead.

A `Water` render family now exists, with three properties worth understanding:

**It is selected by name, not by state.** The three X-Ray water shaders —
`effects\water`, `effects\waterstuden`, `effects\waterryaska` — are class
`S_SET`, priority 2, strict sorting: identical to every other blended overlay,
including `effects\wallmarkblend`. No recorded state separates them, so they
are lifted out by name in `default_name_overrides`, exactly as the wallmark is.
This is the second entry to earn a place in that deliberately tiny table.

**It binds nothing from the source.** No diffuse, no normal, no tiling. The
target material has no `g_stage` at all — it is forward-rendered — so there is
no diffuse slot to write into even if we wanted one. The source contributes
only the plane.

**Its reflection probe has to be repointed.** The material references a probe
from a project the shipped database does not contain, and a material sampling
a missing texture renders the engine's visible `missing texture` fallback. The
`water_reflection_probe` manifest setting supplies one that exists, defaulting
to `environment_probes/environment_probe`.

## Water is not solid

Rendering water correctly exposed the next defect immediately: the player
walked on its surface.

`level.cform` is a collision mesh for *everything* in the level, including
surfaces X-Ray never blocks movement with. It marks them with a flag:

```
materials\water   ["passable"]
materials\bush    ["passable", "suppress_shadows", "suppress_wallmarks"]
materials\fake    ["passable", "suppress_shadows", "suppress_wallmarks", "actor_obstacle"]
materials\death   ["passable", "suppress_shadows", "suppress_wallmarks", "injurious"]
```

Converting every triangle makes all of them solid. `mp_pool` gets two water
triangles the player stands on; Cordon gets **114,080 bush triangles**, turning
every bush into a wall.

`GameMaterial::blocks_movement` is X-Ray's own rule, with one exception:

```rust
!self.has(flags::PASSABLE) || self.has(flags::ACTOR_OBSTACLE)
```

`ACTOR_OBSTACLE` marks surfaces passable to the AI but not to the player, which
is how `materials\fake` implements an invisible wall — those stay solid. The
faces are dropped from the `cform` before anything downstream sees them, so the
material bindings and the split both operate on real collision only.

## Footstep sound: why it cannot be fixed from the map

Wading through the pool sounds like walking on tile, and the obvious fix does
not work. Recorded because it looks so plausible.

The sound exists and is wired. `game_materials/material.pairs` entry 46 is a
genuine actor footstep pair, and its collection ships:

```
pairs[46].mtrl_1 = "actor"   mtrl_1_id = 10
pairs[46].mtrl_2 = "water"   mtrl_2_id = 27
pairs[46].sound  = "material/water/step/water_steps_m"
```

So the tempting move is to give the submerged floor the `water` game material
through `game_material_settings`, and it was tried: 2,023 triangles in
`mp_pool`, correctly emitted, present in four of the nine archives. It changed
nothing, and `game/sources/step_manager.cpp` says why:

```cpp
physics::closest_ray_result ray_result = world.get_physics_world( )->ray_test( ... );
u16 const triangle_material_id  = ray_result.object->get_collision_group( );
material_pair const* const pair = world.get_game_material_manager( )
                                       .get_pair( triangle_material_id, foot_material_id );
```

Despite the variable's name, the material is taken from
`get_collision_group()`, which is the Bullet broadphase filter group:

```cpp
u16 bt_static_rigid_body::get_collision_group( ) const {
    return m_bt_body->getBroadphaseHandle( )->m_collisionFilterGroup;
}
```

The same class exposes `get_triangle_material(triangle_id, ...)` immediately
above, and `step_manager` does not call it. Footstep material is therefore a
property of the collision *object*, not of the surface under the foot, and
`game_material_settings` cannot influence it. Retail agrees: across
`level_03`'s 1,858 collision objects the only `cgroup` values are 8, 2 and 10,
with `cmask` 16, 4 and 20 — powers of two, physics filter categories, not
material identifiers.

What `game_material_settings` *does* drive is everything that resolves a
material per triangle: bullet impact sounds, decals and particles go through
`get_triangle_material`. Those are worth binding correctly, and are. Footsteps
are not reachable from map data in this build, and the reason is sharper than
"the group is not a material": `get_pair` is asymmetric --
`m_pairs[first][second]` -- and pair 46 is stored at `m_pairs[10=actor][27=water]`.
It fires only when the collision group is 10 *and* the player.s foot material
is 27. That foot material is a constant read once at creation
(`player_cook.cpp`: `root["foot_1st_view_material_id"]`) and never updated from
the world, so the only lever is the player config -- which would make every
footstep in every map a splash.

## What is still missing

The water plane is the target's, so it looks and behaves like the target's
water — which is not the same as S.T.A.L.K.E.R.'s. In particular the source's
own `water_studen` colour is discarded entirely: `water_fog_color` comes from
the donor material, so every ported map's water is the same green. Binding a
per-map fog colour sampled from the source texture would be a faithful
improvement and has not been done.

Reflections use one shared probe rather than a probe placed in the map, because
converted maps author no environment probes at all.

The water is a plane, not a volume, and that is visible from inside the pool:
the first-person model is drawn forward and never fogged, and it occludes the
water surface behind it, leaving an unfogged halo around a submerged boot.

This cannot be fixed from map data, and modelling the water as a box -- the
shape v0.1.1e uses -- would not fix it either. **The engine has no concept of
being underwater at all**: `swim`, `submerg`, `water_level`, `water_height`,
`underwater` and `in_water` appear nowhere in the source except one NPC
`die_in_water` sound collection. There is no water volume object, no water
level, and no camera-in-water test, so nothing exists to trigger a submerged
view. A box would also most likely be back-face culled from the inside; the
material instance exposes no cull flag, since that belongs to the base effect.

Underwater rendering is an engine change, not a resource change.
