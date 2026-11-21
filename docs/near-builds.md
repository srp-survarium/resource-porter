# Maps of builds near 0.26g (0.29 to 0.45)

`vostok-port build-survarium-near-map` makes a content pack of a map as one of these builds has
it, in one pass over the donor's own projects. The builds share 0.26g's config, model and surface
formats and most of its files by name, so nothing is re-authored: the work is telling what players
already have from what the donor changed, and writing the project the way 0.26g reads it.

```sh
vostok-port build-survarium-near-map --manifest ports/survarium-044a5-london-to-026g.json \
  --donor donor.list --shipped players.list --output <packs>/london-v042-1
```

`donor.list` names the donor build's archives, `players.list` what players have (the stock
`resources.db`, then every offered patch in order), one a line, bottom first. The pack's
`report.json` says what each rule did; `pack.json`'s notes say what was left out by decision.

A pack the manifest `requires` is part of what players have: two packs may not add the same path,
so a map built after another lists that pack's files too (`players.list` with, last, a content patch
that composes it). London (v0.32) requires London (v0.42): 1 230 files both donors add (the same
g-buffer programs, flora and level models) are London (v0.42)'s, the 1 022 the same bytes are
shared and the 208 that differ go in under `ported/v032a`.

London (v0.42) (`ports/survarium-044a5-london-to-026g.json`, donor 0.44a5) and London (v0.32)
(`ports/survarium-032a4-london-to-026g.json`, donor 0.32a4) are the two manifests. A manifest
names the level, the namespace imports go under, the kinds left out by decision, and each project:
the donor's folder, the port's, its weather layer, and for a new map its `maps_dict` id and mode
config. A project without `map` replaces one players have (its row and config stay).

## The four steps (`src/near_build/`)

1. **The project's form** (`schema`). 0.32 introduced the editor form: objects carry ids and
   refer to one another by id. 0.26g to 0.31 refer by name. The two forms were compared on objects
   matched by `full_name` between 0.31a2's and 0.32a4's London; `game_object_type` is the same in
   both.
   - `lib_name` loses `base:`; a particle's is its `particle_name`, an animated model's its
     `model_name`, a light's `point_light` or `spot_light`; statics name their model in `lib_name`.
   - A link (`path` of ids, with or without `__type`) becomes `{ name }`; a table of links a list.
   - The members of a placed group (a lamp's light, a fire's barrel and flame) are all called
     `<folder>/<name>/<name>`, alike in every copy of the group. The old form calls them
     `<folder>/<group>/<name>`; the group's own name is not in the project, so it is made: its
     first member's name, numbered when taken. Names are unique again.
   - A battery point's `other_list` links whole groups; the old form lists each member by kind
     (`attached_objects`: spawners, spawn boxes, boxes, statics, groups, cables;
     `attached_c_objects`: the groups' statics, lights, particles, sounds, animated models).
   - A kind left out by decision takes its group along and what only that group links
     (`without_kinds`): an anomaly's zones, shapes, effects and artefact places are members of
     its group. A fire's damage zone stands in no anomaly's group and stays.
2. **The files** (`files`). A donor file players lack is added at its own path. One they have in
   the same form is theirs: the same bytes; for a texture the same image (`image`, the 0.69d
   port's test); for a render surface the same vertices and triangles (0.44 writes the same
   triangles in another order, and adds statistics chunks 110 to 113 that 0.26g's surfaces do not
   have); for a material or a model's settings the same values once names are resolved. One they
   have in another form is imported under the manifest's namespace, and what names it names the
   import. A model's `hq_collision` is not carried: 0.26g's executable has no reader for it.
3. **The fields** (`fields`). 0.26g's own projects are the definition: an object keeps the fields
   0.26g's objects of its kind have, in 0.26g's number types, and loses the rest (each reported);
   a field every 0.26g object of the kind has takes 0.26g's one value. A kind 0.26g has no object
   of is left out (`portal_system`, `primitive_geometry`, `sound_environment`, `winner_point`).
   The environment's `properties` are the donor's first key frame, each setting some 0.26g
   project sets; `layer` is `default`, the environment's the weather's.
4. **The rest** (`dependency`). Particles, sounds, animations and sky textures the project needs
   and players lack are added as the donor has them; one players have in another form stays
   theirs (counted in the report).

## Materials and shader programs

A material keeps the stage and effect fields 0.26g's materials have for its effect, and the fields
skhid's client patches make 0.26g read (`ports/near-build/client-fields-026g.tsv`, the manifest's
`client_fields`): patch 28's uv scrolling (`use_uv_scrolling`, the four `*_map_scroll_*_direction`) and
patch 169's forward effects (`use_ttransparency_dxt1` of forward_default and particle_lighting,
`use_uv_tiling` with its four `*_map_tiling_*_factor`, `apply_distortion_to_emissive_only`). Before
that table the converter dropped them as fields stock materials lack, so a donor's scrolling sand,
tiled fires and DXT1 cut-outs became blank quads. Its switches
(`use_*`, techniques, modes) select shader programs, one set for each kind of mesh the stage is
compiled for (`with_static_meshes`, `with_vegetation`, ...). 0.26g cannot compile a program: it
asks the archive for it, and stops with an assert ("cannot find shader") when it is not there.

- **A switch set players' materials do not use** is not known to have its programs. By default
  such a material takes the largest set players' materials use that adds no switch
  (`report.json`: `degraded`). `--keep-switch-sets` keeps the donor's sets instead: a build for
  listing what the game then asks for (its asserts in a headless load).
- **`--programs <file>`** supplies the programs a file names (any text holding
  `shaders/sm_4_0/<family>/<key>` paths, such as that load's asserts) from the donor's own blobs
  (`programs`).
  - A key has one value for each shader macro in the order the executable fills them, `_` where
    the family's mask (`shaders/masks`, data) does not read the macro; the vertex input type takes
    two characters from ten. The orders are in `ports/near-build/shader-key-order-<build>.tsv`,
    read from each executable (`fill_shader_configuration_macros`; 0.26g 0x3C9B80) and checked
    against every key of the build's archives: 0.26g has 67 macros, 0.32a4 and 0.44a5 68 (one
    more, `CONFIG_ANGLE_DEPENDANT_ALPHA_TEST`, and another order).
  - The donor's program for a 0.26g key is the one at the same values, with the donor's own
    macros off. Of the 7 908 g-buffer programs players have, both donors have 5 648 under the
    translated key, 3 144 of them with the same code.
  - A program is supplied only when 0.26g can feed it: every texture and sampler it binds and
    every constant it reads is one a program of the family that players have binds or reads. A
    constant the donor declares longer passes when the code reads no byte past 0.26g's length
    (the donors' `specular_color_parameter` is a float4 for 0.26g's float3, and their g-buffer
    programs read its first component only). Of the donors' g-buffer programs players lack,
    0.26g can feed 798 of 0.32a4's and 1 032 of 0.44a5's; the others read `scene_slomo` or the
    parallax scale and scrolling factors, which 0.26g does not set.
- **`--donor-programs`** supplies, by the same gate, every program of the donor's in the
  manifest's families (`shader_keys.families`: `gbuffer_pass.ps` and `vertex_base.vs`, what a
  material and its mesh select) that players lack: whatever key 0.26g computes for the donor's own
  materials and meshes, the donor computed too. A vertex shader's family is not in `shaders/masks`;
  it reads the macros its keys have values at. London (v0.42) gets 1 038 of 0.44a5's 1 420 (1 032
  g-buffer programs and six vertex shaders: a mesh of vertex type 2 or 11 with wind motion 3, which
  no mesh of players' has; the first headless load stopped on one). Wind motion selects programs,
  so it is part of a switch set. The forward families are not named here: the shared pack below
  supplies them for every map at once.
- **The client fields' switches select programs** the pack `forward-effects` ships with patch 169,
  so a material's other switches are judged without them (players' sets alike) and the client
  fields are kept as the donor has them.
- **`build-shader-macro-pack`** makes that pack (`ports/forward-effects-026g.json`): the macros the
  patch sets (CONFIG_USE_OLTA, CONFIG_USE_FUZZINESS) appended to the masks of `forward_base.ps` and
  `offscreen_particle_lighting.ps` (`config_append` to `shaders/masks`), every program of players'
  two families again under its wider key (the new macros 0), and every program of the donors'
  forward, particle lighting and z-shifted vertex families players then lack, by the gate above
  with `constant_tile_uv` 16 bytes (the patch sends a float4 where 0.26g sent a float2). Where both
  donors have a key, the code is compared (the include records keep each source file's time): the
  same code is one program; other code is the first donor's (0.44a5's), and the report lists each
  (`donors_differ`: 448 of the 870 keys both have, e.g. an exp2 fade scale). The London manifests
  `require` it, so their programs are composed first.
- **What 0.26g has no field for, even patched,** is listed (`switches_not_in_026g`): today
  `use_angle_attenuation_falloff` (CONFIG_ANGLE_DEPENDANT_ALPHA_TEST, a macro 0.26g's key order has
  not); no material of either London turns it on.

`examples/near-build-programs.rs` prints those counts for a family; `near-build-reads.rs` what a
program reads; `near-build-survey.rs` and `shader-programs.rs` what a donor's map needs against
what players have; `near-build-lab.rs` reads one path through a layered archive list.
