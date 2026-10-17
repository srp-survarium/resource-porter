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

## The closed pose (pack version 2)

Version 1 only turned the visor 58 degrees about the screws. The owner saw
three faults: the grille sat over the forehead, not the eyes; the front of
the dome, which the raised visor covers, was near black (darkness baked
into the texture); and the tabs' native copy left the raised arm's stem
standing on the side. Version 2:

* **The grille on the eyes.** The grille's centre is where its texels are on
  the plate (UV 0.095, 0.5). A turn alone cannot put it on the native head's
  eyes (the `LeftEye`/`RightEye` bones, y 1.6743): 86 degrees would, but the
  plate's lower half then cuts the nose. So the visor turns 82 degrees and
  the plate then slides 3.5 cm forward and as far down as puts the grille's
  centre at the eyes' height (0.8 cm). The slide takes the plate whole from
  8 cm from the screws and the arms not at all within 4.5 cm, so the arms
  lengthen between and stay on the screws. The plate is 8.8 mm clear of the
  native player head (`black_market_01_head_s`) at its closest.
  `eye-line-sweep.png` in the gallery: 66 to 90 degrees, each with the slide
  that puts the grille on the eyes at least 6 mm clear.
* **The forehead.** Lowered, the plate's top is under the brow (the shell's
  front edge, y 1.748): a flap of 16 triangles closes the gap from just
  inside the plate's top to under the brow, in the dome's lit metal.
* **The hinge.** The tabs turn with the visor (a copy, 1 mm outside); the
  native tabs' triangles stay in place as the fill of their old footprint,
  on copies of their vertices flat inside the outline and drawn from the
  hinge disc's dark ring, so the shell around them keeps its texture and no
  raised stem is left.
* **The baked darkness.** The helmet has its own diffuse texture,
  `textures/character/human/edge_assault_05/edge_assault_05_helmet_closed`
  (DXT1, the native options), and its own material naming it.
  `scripts/keeper-closed-026-texture.py` makes it from the native one: the
  dome, the brow wedge, the visor's underside and the arms' inner faces are
  lifted to the shell's lit level (lit metal cloned from the dome's middle
  where they were dark, a gain elsewhere); every other texel is native.

Nothing else changes: the normal, specular and fresnel maps are the native
ones, and the model's other files are copied as they are but its settings,
which name the new material.

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
python3 -I scripts/keeper-closed-026-texture.py \
    --model <native edge_assault_05_helmet_s/converted_model> \
    --texture …/edge_assault_05_helmet.dds --output work/diffuse.png
# ImageMagick runs sandboxed in the working folder: run from a folder that
# holds both the texture and the pack.
cargo run --release --example build-keeper-closed-026 -- \
    --base-db RETAIL/resources.db --base-patch …patch_12 --base-patch …patch_14 \
    --base-patch …patch_17 --diffuse work/diffuse.png --output packs/keeper-closed-2
python3 -I scripts/render-keeper-closed-026-icon.py \
    --open-model <native edge_assault_05_helmet_s/converted_model> \
    --closed-model packs/keeper-closed-2/files/models/character/human/edge/edge_assault_05_helmet_closed.skinned_model/render/edge_assault_05_helmet_s/converted_model \
    --texture …/edge_assault_05_helmet.dds --normal-map …_nmap.dds --specular-map …_smap.dds \
    --closed-texture packs/keeper-closed-2/files/textures/character/human/edge_assault_05/edge_assault_05_helmet_closed.dds \
    --lobby-swf <flash_movies/lobby_menu.swf players have> \
    --output packs/keeper-closed-2/icons/r14c06.png --work icon-work
vostok-port pack check --pack packs/keeper-closed-2 --base-db … --base-patch …
```

The icon script finds the camera of the native helmet's icon (`[15, 3]`) by
its silhouette, grades its render of the open helmet to that icon's
colours, and renders the closed one the same way.
