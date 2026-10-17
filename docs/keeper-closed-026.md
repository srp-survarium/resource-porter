# The "Keeper" helmet, closed (v0.26g)

The Fringe Settlers' "Keeper" helmet (`edge_helmet_5`, dictionary 202) has
its visor raised. The owner asked for a closed one beside it (2026-10-17):
a second item, dictionary **1820**, `edge_helmet_5_closed`. `src/keeper_026.rs`
writes it as a content pack, `keeper-closed`.

## Where a closed visor comes from

No build has one. `edge_assault_05_helmet` is the same model from 0.26g to
0.28d0 (byte for byte); 0.31a2 and 0.46c2 only recompress it, and 0.31a2
gives `edge_helmet_5` another model (`edge_defender_helmet`, a hood). The
model has one surface, `edge_assault_05_helmet_s` (870 vertices of 36 bytes,
all on the head bone), no other LOD, no morph and no sibling model.

The visor is six UV islands of that surface (130 vertices): the plate, its
rim and underside and the two arms, all above the brow (y > 1.735) and in
front (z > 0). Its arms end in the two teardrop tabs of the shell's side
band, each on a screw (vertices 163 and 492, y 1.7235, z 0.0117): the hinge.
The closed pose turns the visor down about the screws' axis, by 58 degrees.
The tabs share their outline with the shell's hinge discs, so the tabs that
turn are a copy, 1 mm outside the native ones, which stay as the fixed leaf
of the hinge. The wedge under the raised plate (island 12) stays with the
shell. Nothing else changes: the texture and the material are the native
ones, and the model's other files are copied as they are.

At 58 degrees the plate stands over the eyes and the nose, 6.8 mm clear of
the native player head (`black_market_01_head_s`) at its closest (52: 16.7
mm, 60: 3.3 mm). It does not reach the mouth and chin.

## The item

The native config with its own name, icon and model; price (42 750),
seller (the Fringe Settlers at rank 9, by srp's rule from the item level),
weight, drop weight (20) and armour are the native helmet's. As the native
helmets that close over the face (`edge_helmet_6`, `blackmarket_helmet_5`,
`renesanse_helmet_4`/`6`), it takes the mask slot
(`data.additional_model_slots = ["mask"]`) and protects the face as the head
(`hit_params.face`, the `brain` values). A mask under the closed visor would
show through it (the Keeper mask by 45 mm).

| | |
| --- | --- |
| Name key | `st_edge_helmet_5_closed_name`: the native name and "(closed)" in the six languages |
| Description | the native helmet's (`st_edge_helmet_5_desc`) |
| Icon | `[14, 6]`, rendered by `scripts/render-keeper-closed-026-icon.py` |
| Model | `models/character/human/edge/edge_assault_05_helmet_closed.skinned_model` |

## Building

```
cargo run --release --example build-keeper-closed-026 -- \
    --base-db RETAIL/resources.db --base-patch …patch_12 --base-patch …patch_14 \
    --base-patch …patch_17 --output packs/keeper-closed-1
python3 -I scripts/render-keeper-closed-026-icon.py \
    --open-model <native edge_assault_05_helmet_s/converted_model> \
    --closed-model packs/keeper-closed-1/files/models/character/human/edge/edge_assault_05_helmet_closed.skinned_model/render/edge_assault_05_helmet_s/converted_model \
    --texture …/edge_assault_05_helmet.dds --normal-map …_nmap.dds --specular-map …_smap.dds \
    --lobby-swf <flash_movies/lobby_menu.swf players have> \
    --output packs/keeper-closed-1/icons/r14c06.png --work icon-work
vostok-port pack check --pack packs/keeper-closed-1 --base-db … --base-patch …
```

The icon script finds the camera of the native helmet's icon (`[15, 3]`) by
its silhouette, grades its render of the open helmet to that icon's
colours, and renders the closed one the same way.
