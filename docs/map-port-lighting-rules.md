# Map porting and lighting: the rules

The rules every map port to 0.26g follows: the owner's decisions and the
conversion contracts behind them. Per-map details stay in their own documents
([0.21d maps](survarium-021-to-026.md), [Chemical 0.11e](factory-chemical-011e-day.md),
[S.T.A.L.K.E.R. maps](soc-map-porting.md)); this page is the checklist a port
is held to. Give it to every worker that ports a map or touches its look.

## How a look is judged

- **Judge it in the stock client.** Players run the released exe with no camera
  hooks or render presets. A look that depends on a hook (`SRP_LEGACY_SKY`,
  `PROBE_DIFFUSE`, `SKYLIGHT_UNDERLAY`, ...) is not what players see. Hooks are
  for investigation only.
- **Prove a fix in the game.** Capture ours against native 0.26 at the same
  coordinates (the same location exists in 0.26 for every Survarium port), fix,
  capture again, repeat until it looks like native. Inferring from material
  names or values is not proof. Each fix gets a gallery: before | native | after.
- **One game client on the GPU at a time.** The machine has one 8 GB card; a
  second client crashes both. Captures wait while the owner plays.
- **Approved lighting stays.** Once the owner approves a map's lighting, the
  sun, ambient, probe placement and multipliers are not retuned. Surface
  faults are fixed in the surfaces, and converter defects in derived lighting
  data are fixed as defects (below).

## Materials

- **Native finish.** Where 0.26 has a material at the same path, the port takes
  0.26's gloss, Fresnel, specular and finish maps; it keeps the donor's colour
  and normal maps. In 0.26g `constant_roughness` is really gloss.
- **Native blend tuning.** A material that blends a second layer by 0.26's
  height-and-mask technique takes native's blend tuning too
  (`vertex_blended_falloff`, strength, factor, tiled-mask scale, rotation and
  mask texture), but only where its technique and mask match native's.
  Donor falloff 1 against native's 10–15 smears the second layer over the
  whole surface (the wet, mottled floor look).
- **Reviewed native textures.** A donor texture that is the same art as 0.26's
  but stronger (normal maps 1.5–10× native's) or that lacks the alpha heights
  0.26's blend reads is replaced by 0.26's own, listed in the manifest's
  `reviewed_native_textures` with a reason. Check that no material alpha-tests
  on an added alpha.
- **Texture dedup.** A donor texture that is byte-identical to a native one, or
  has the identical top mip, uses the native file.
- **Exceptions are listed.** Any deliberate difference from native
  (`finish_exceptions`, e.g. the cabinets' Fresnel 0.05 the owner chose on
  2026-10-17) names the material it applies to and the reason; the build fails
  when it matches nothing.

## Probes

- **Diffuse cubes are irradiance.** 0.26 lights each pixel from
  `<probe>_diffuse.dds` along its normal and expects an irradiance cube (a
  face's brightest texel 1.1–1.3× its mean). A donor without diffuse cubes gets
  the cosine-weighted average of the probe's own radiance (`probe_irradiance`),
  never its mip tail: a mip tail keeps windows and sky at 3.5–8× the mean, and
  every normal-mapped bump catches a hot spot (crumpled-foil floors, white
  blotches on metal). The v0.11 release uses `probe_diffuse_gain` 1.25 to keep
  the brightness the owner approved.
- **Sphere probe falloff.** 0.26 weighs a sphere probe between
  `radius − width − 0.05` and `0.98 · radius`. A donor probe with a reversed
  interval weighs 0 everywhere and lights nothing; it is opened to width
  `1 + 0.02 · radius`, as native globals are. Valid probes are untouched.
- **No missing probes.** A probe reference without its cubemaps renders NaN
  black. Every build checks `missing_probe_references=0`.
- **Outliers are reviewed.** A probe setting outside anything native uses
  (e.g. specular 5 where native is ≤ 1) gets a `probe_overrides` entry with a
  reason, matching exactly one probe.

## What a map ships with

- **Loading screen:** a Survarium port uses its source build's original
  picture; when the source build has none for that level, the first build that
  has one (Chemical v0.11 uses `Level4_day`, identical in 0.20e, 0.21d and
  0.26). A S.T.A.L.K.E.R. port uses cinematic renders of the map with no HUD
  and no characters. Never the template project's picture.
- **Minimap:** the map's own, in native style.
- **Name:** the original name plus the version in parentheses, e.g. "Vostok
  Radar Station (v0.21)", in all six languages, in the loading names and in
  the lobby's Matches table (`BottomPanel.matchesNameTable()`).
- **Ammo crates:** at native 0.26's exact spots for the same location, with
  native's rotation and its zone, box and decal; only the height snaps to the
  ported floor. Source-only props in the way are removed, not the crate moved.
- **Battery containers:** where the source map has them, unless the owner
  places them. No chargers (mode 3).
- **Spawns:** at least 10 per team. A point must not be inside collision and
  must not stand over a hole (a player capsule from `default.player`, the
  collision the player walks on, rays round the point). Nothing else about a
  spawn is a rule.

## Shipping

- A content patch is built over every released patch in `release/patches.tsv`
  on main; check the released revisions and tags before naming one.
- A pack never undoes another release's edits: a file another patch replaced
  (`hud.swf`, `lobby_menu.swf`, the dictionary, localizations) is edited on
  top of the newest released version, and `pack check` reports drift.
