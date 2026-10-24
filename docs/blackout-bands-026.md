# Blackout team armbands for 0.26g

The blue and the green team armband of S.T.A.L.K.E.R. Clear Sky / Call of
Pripyat multiplayer, on both upper arms of the native v0.26g player, for the
Blackout battery variant (no interface: teams are told apart by the band).
`src/blackout_bands.rs`. No item: the game adds the band to every player's
visual as a part.

| team | `base_model` | `part_name` | material |
| --- | --- | --- | --- |
| blue | `ported/blackout/band_blue` | `band` | `ported/blackout/band_blue` |
| green | `ported/blackout/band_green` | `band` | `ported/blackout/band_green` |

Donors: `meshes/actors/stalker_mp/mp_team_{blue,green}_1.ogf` and
`textures/act/act_mp_team_{blue,green}_1.dds` (Clear Sky and Call of Pripyat
ship the same bytes). The band is the models' `models\selflightl` child
mesh. The port keeps its width, the emblem's side and the band's cloth (cut
from the texture, centred on the emblem) and makes a new ring on the native
upper arm, over the sleeves of every torso the player can wear (stock and
released ports), rigid on `LeftArm` / `RightArm`. The normal map is flat.
The builder prints and writes (`report.json`) the gaps over every torso and
the least gap in the lobby and match clips.

```
cargo run --release --example build-blackout-bands -- \
    --source-root "Stalker Call of Pripyat" --target-db resources.db \
    --patch resources.db.patch_12 … --patch resources.db.patch_31 --output bundle \
    [--distal 0.88] [--cover 1]
vostok-port pack from-bundle --base-db resources.db --base-patch … --bundle bundle \
    --output blackout-bands-1 --id blackout-bands --version 1
```

`--distal` places the band's lower edge along the upper arm, `--cover` is
the quantile of the sleeves it goes over (1: all; lower is snugger on most
torsos and sinks into the bulkiest).
