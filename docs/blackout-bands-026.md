# Blackout team armbands for 0.26g

A plain cloth armband, blue (team 0) or red (team 1), on both upper arms of
the native v0.26g player, for the Blackout battery variant (no interface:
teams are told apart by the band). `src/blackout_bands.rs`. No item: the
game adds the band to every player's visual as one more part, next to the
torso.

Every torso model has its own band, an offset shell of its own sleeve: the
sleeve's outer skin between 0.45 and 0.67 of the way from the shoulder joint
to the elbow (60 mm), cut there and moved 4 mm out, each vertex skinned as
the sleeve point it was made from, with the torso's own skeleton and bind
pose. So it deforms with the sleeve through shoulder and elbow.

Naming, for a torso part whose `base_model` is `<B>` (as the torso item
configs and `gameplay/bodyparts/default` write it):

| | |
| --- | --- |
| model | `ported/blackout/<B, '/' as '_'>_blue`, `..._red` (`models/<that>.skinned_model`) |
| `part_name` | `band` |
| materials | `ported/blackout/band_blue`, `ported/blackout/band_red` (shared) |

e.g. `character/human/edge/edge_hunter_01_torso_s1` ->
`ported/blackout/character_human_edge_edge_hunter_01_torso_s1_red`.
The bundle's `torsos.tsv` lists the torso records that have bands.

```
cargo run --release --example build-blackout-bands -- \
    --target-db resources.db \
    --patch resources.db.patch_12 … --patch resources.db.patch_31 --output bundle
vostok-port pack from-bundle --base-db resources.db --base-patch … --bundle bundle \
    --output blackout-bands-5 --id blackout-bands --version 5 [--look ...]
```

`--look` picks the cloth; the geometry is the same in all:

| look | pack | cloth |
| --- | --- | --- |
| `plain` | 2 | solid blue (28,64,178) / red (176,26,26), plain cloth material, flat normal map |
| `cloth` | 3 | the game's grey woven fabric (`camo/scavenger/fabric_grey`: diffuse, weave normal map, roughness map) tiled at its own weave and tinted to the clean colours, on the Scavenger jacket's material (`scavengers_01_torso`) |
| `worn` | 4 | worn navy (38,64,122) / brick red (146,50,36) with grain, fading and dust, plain cloth material |
| `worn-cloth` | 5 (default) | the grey fabric tinted to the worn colours, with the fading and dust |

The builder reports each torso's band, the torsos it skips (no sleeve around
the upper arm) and the band's distance to its sleeve in the bind pose and in
the AK match idle and run, the AK lobby idle and a pistol lobby idle.

Pack 1 was one ring for all torsos, taken from S.T.A.L.K.E.R. Clear Sky /
Call of Pripyat multiplayer (`soc::codec::skinned` reads their actor
models): the hull of every sleeve, 70 cm around, low on the arm and rigid on
the upper-arm bone, it stood centimetres off the sleeves.
