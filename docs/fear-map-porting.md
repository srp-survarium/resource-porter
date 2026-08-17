# Porting F.E.A.R. multiplayer maps to v0.100b

Why the lilith path is shaped the way it is. The *how* — commands, manifest
fields, failure modes — belongs in `the porting guide` alongside the SoC pipeline it
reuses; this file records what is different about a Jupiter EX source and the
judgements that difference forced.

The name honours the lineage: LithTech begat Jupiter, Jupiter begat Jupiter
EX, and this port begat Lilith.

## The source is friendlier than X-Ray, with one lie in it

A compiled F.E.A.R. world (`.World00p` version 113) carries everything in one
file: brush BSPs, physics shapes, one render mesh with declared vertex
layouts, a portal graph, and a plain-text object list. Materials (`.Mat00`)
name their shader by `.fx` path and their textures by full path — diffuse,
normal *and* specular per surface, where SoC gave a diffuse and sometimes a
`_bump`. Textures are stock DDS. The 27 shaders across all 24 multiplayer
worlds classify by path alone (`lilith-audit-shaders`), against the hundreds
of blender/priority combinations the SoC classifier reasons over.

The archives (`.Arch00`) mount in `Default.archcfg` order, later archives
shadowing earlier — patches and expansions re-ship whole files, so the
archcfg of the *last* expansion is the truth of what the game loads. Stored
paths nest under mount-prefix chains
(`Materials\Materials\City\Materials\City\Buildings\...`); a resource
reference matches a stored path's component-aligned *suffix*, with exact
matches winning.

The lie: the per-model **physics shapes look like collision and are not**.
Every brush model carries one — mesh soups, OBBs, spheres, capsules, with
mass and density — and converting them produced a map the player fell
through: 4.5 km below Office's spawn before the theory died. They are
rigid-body proxies. Nor are the **brush BSP polygons** enough on their own:
the engine's real collision is the solid-leaf BSP *tree*, and the stored
polygon soup has holes where the tree implied surfaces — one column inside
Office has no polygon at all where the floor visibly is. Nor is the render
mesh: it lacks the invisible ramp clips mappers laid over stairs (the
visible treads are unconverted prop models) and every other unseen seal.

What ships is therefore the **union** — render surfaces of the solid
families, plus the BSP polygons, plus the authored blocker fences — each
covering the others' gaps, coincident duplicates harmless.
`lilith-probe-ground` reports the result per column.

Three more findings from the playtests that got Office standing:

* **spawn position is the capsule's feet, and the capsule is 2.0–2.7 m
  tall.** The walk telemetry tracks floor heights exactly (feet), Office's
  3.5 m rooms rejected a spawn authored 1.5 m up — 2.0 m of headroom was
  not enough and the engine resolved the capsule up through the roof —
  while the SoC bath's 2.7 m of headroom always worked. So a usable spawn
  needs about 3.3 m of floor-to-ceiling clearance and is placed 0.55 m
  above the floor; `lilith-spawn-candidates` judges every authored
  `GameStartPoint` by exactly this and emits the adjusted height. Every
  world has usable points under this criterion — from 9 in Bedlam to 172
  in Asylum. (The interim "must be open to the sky" theory was this
  constraint seen through one map's ceilings; reversing the collision
  winding, the theory before that, changed nothing at all);
* **the invisible sealing hull must not become collision.** BSP polygons
  with flag bit `0x0004` are the sky lids and seals — the one bit every
  observed lid carries (DeadWood's `0x006e`, Streets' `0x0014`) and no
  observed floor or glass does. Converted, they put a ceiling over every
  outdoor map;
* **triangle size matters to the cooker.** Brush faces arrive as slabs two
  triangles wide; converted at that size the cooked collision behaved like a
  bad spatial hash — standable near parts of a slab, holes at its corners.
  Every emitted triangle is subdivided until no edge exceeds 4 m, the
  density the cooker grew up on. Blockers are emitted in both windings,
  since nothing about facing is proven either way.

## What the object list gives that SoC never had

The object section is self-describing: class name plus typed properties, in
plain text. Three consequences:

* **spawns are authored.** Every multiplayer world declares dozens of
  `GameStartPoint`s; the build records them all, and the SRP map profile
  picks from the record instead of ranking collision by flatness. The
  authored height is already ~1.5 m above the floor — no capsule-centre lift.
* **the lighting exists as data.** F.E.A.R. bakes no lightmaps; its light
  entities (`LightPoint`, `LightSpot`, with colour and radius) are the
  map's lighting, and are decodable for a future pass that emits them as
  v0.100b light objects. Until that pass, a converted map runs on the
  template's sun and skylight ramp like the SoC maps — indoor maps will read
  flat.
* **`WorldProperties` carries per-map ambient, sky ambient and fog** —
  likewise future manifest inputs rather than template inheritance.

## Judgements a reviewer should know about

* **84% of the render mesh is invisible.** F.E.A.R.'s stencil-shadow tech
  ships precomputed shadow volumes as ordinary surfaces under
  `shadowvolume.fxi`; Office keeps 23,600 visible triangles of its 150,082.
  Dropping them is not a loss — they never rendered as geometry.
* **sky geometry is dropped**, because the cloned template project keeps its
  own sky object; converted skybox brushes would draw a box around the map.
* **additive shaders take the emissive family** — `translucent\additive*`
  and `solid\emissive` are the same bright-on-black idiom as X-Ray's
  lightplanes, and the same additive `blend_mode` fix applies. `solid\
  emissive` materials bind `tEmissiveMap`, not `tDiffuseMap`.
* **`translucent\multiply` becomes a decal.** Blood and grime multiply-darken
  in the source; the target's alpha decal is an approximation, reviewed as
  acceptable.
* **merging is per material, not per shader.** F.E.A.R. batches per material
  already, and each material owns its own textures, so the port keeps that
  granularity — Office emits 207 materials where a per-shader merge would
  have collapsed them to a dozen and lost every texture but one.
* **one physical material.** F.E.A.R. hangs surface types off materials, not
  collision faces; every converted face is `concrete` until a reviewed
  translation exists. Footsteps will lie on metal catwalks.
* **units are centimetres**, scaled by 0.01 at conversion. The spawn heights,
  extents and probe output are all metres, like everything else the pipeline
  touches.

## Fidelity losses accepted, in the SoC port's terms

Dynamic world models — breakable glass, doors, hinged pictures, explosive
props — are converted as static geometry: their brushes render and collide,
and nothing ever moves or shatters. `Model00p` prop instances, particles,
sounds and the AI navmesh are not read. The three dangling material
references retail ships (`MetalCor04b`, `PipeMetalSep4`, `Wood_SL001` — in no
archive; the source engine shows its fallback there) fail the build closed
until reviewed.

## What is still missing, in the order it is worth attacking

One map of twenty-four is built. Everything below is known, not suspected.

* **the other 23 multiplayer worlds.** All decode and probe clean; none has
  a manifest, an SRP profile, or a spawn audit. Generate the manifests from
  one place like the SoC thirteen. Each map needs its spawn column checked
  with `lilith-probe-ground` for the open-sky property — outdoor maps
  (Streets, DeadWood, Docks…) should offer many candidates where Office had
  one. Maps that reference the three dangling materials will fail closed
  until each gets a reviewed substitute;
* **`Model00p` props are not converted, and stairs prove it matters.**
  Office's visible stair treads are prop models; only their invisible clip
  ramp carries the player, so you climb stairs you cannot see. Elevator
  cars are the same class. A `Model00p` decoder is the single largest
  fidelity gain left;
* **authored lights are decoded and then dropped.** Every world carries its
  `LightPoint`/`LightSpot` objects with colour and radius — F.E.A.R.'s
  entire lighting, since it bakes nothing — and the port emits none of
  them, so interiors run flat on the template's sun and skylight ramp.
  `WorldProperties` ambient and fog are likewise read and unused. Emitting
  these as v0.100b scene objects is designed nowhere yet;
* **nothing about the look has been compared against the source.** Specular
  maps exist per material and are not bound; `vTintColor`, `vBaseColor` and
  `fMaxSpecularPower` are parsed and ignored; emissive strength,
  glass, and the `murky_water_low` sewer water are all unreviewed in the
  client. The sky is level_03's, not F.E.A.R.'s;
* **every collision face is concrete.** Footsteps and impacts lie on metal,
  wood and glass until material `SurfaceFlags` get a reviewed translation
  to target physical materials;
* **respawns all land on one point.** The server project's respawn array is
  written entirely at `spawn_position`; the 47 authored start points are
  recorded in the build record and unused beyond choosing that one. Whether
  more of them are usable depends on the topmost-surface spawn behaviour,
  which is observed, not understood — the engine source around spawn
  placement has not been read;
* **the patch-era `.Objects00p` overrides are unexamined.** Some worlds ship
  newer object lists as sidecar files (`Office.Objects00p` in
  `FEAR_5.Arch00`); the port reads only the object section inside the
  `.World00p` it mounts. Divergence unmeasured;
* **one genuine geometry hole is known and unexplained**: the column at
  [67.5, -2.0] in Office has no surface in any converted source. It sits in
  the stairwell where prop stairs live; whether the real game covers it
  with a prop's collision or the BSP tree is unknown until the prop decoder
  exists.

## Validation state

Verified: all 24 multiplayer worlds decode end to end — world models,
physics, render, objects, streaming assets, with four section-total
tripwires and a terminal-padding check standing guard; every emitted Office
resource re-read from the packed database and hash-matched (`verify`).

Verified in the client, one walked session each with position telemetry:
thirteen of the twenty-four maps — Office, Asylum, Bedlam, Cafeteria,
Campus, Stockpile, Waterworks, Heliport, Factory, Docks, Train Yard
(Perseus Mandate), DeadWood and Streets. Every one spawned standing on its
audited point and held the player at floor level for the whole session,
with one defect found: Cafeteria has an open edge around [0.4, -30.4]
where no converted source carries geometry — the same missing-prop class
as Office's elevator shaft. The remaining eleven (Clone, Construction,
Depot, Evac, Facility, Forgotten Factory, HighTech, Labs, Museum,
Refinery, SpillKill) are built and verified but not yet walked.

The `there is no surface with id[5/6/7]` console message that accompanies
a session is the player character model's head-visibility toggle poking
subsurfaces the stock model lacks (`game/sources/player.cpp:1368`); it is
not a map defect.

Not verified: how any world *looks* against the source — texture
assignment, translucency, emissive strength — and everything the lighting
section above defers.
