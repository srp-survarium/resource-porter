# A vocabulary for texture defects

Naming a defect precisely is most of the diagnosis: each term below points at a
different stage of the pipeline, so the word chosen decides where to look. The
distinctions that matter are *not* the ones that feel most obvious visually —
"distorted" covers at least four unrelated causes.

## The classes

| term | what it looks like | what it means | where the bug is |
| --- | --- | --- | --- |
| **smeared** | texture stretched into streaks, detail unreadable, often swirling; adjacent faces differ | the UV *gradient* across a triangle is wrong — vertices disagree about where they are in the texture | vertex decode: quantisation, the `s24_tc_base` remainder, the wrong UV set |
| **mis-tiled** | texture is sharp and square, but repeats too many or too few times | UV *scale* is off by a uniform factor | UV scale constant, or a material tiling parameter |
| **offset** | correct scale and orientation, pattern does not line up across a seam | UV *translation* is wrong | UV origin, atlas offset |
| **rotated / flipped** | correct texture, wrong orientation; text mirrored or sideways | UV axis convention — D3D vs GL V direction, or swapped u/v | vertex decode, or the target's UV convention |
| **wrong texture** | a different image entirely: a drawer wearing brickwork | the surface is bound to the wrong image | material binding, shader-table index |
| **wrong response** | albedo is right, but the surface behaves like another material — too shiny, wrong bumpiness, plastic | diffuse is ours, the other texture roles are not | template inheritance: normal/spec/roughness/detail still the donor's |
| **wrapped** | a localised patch of repeated mirrored slivers, zigzag or chevron | UVs cross a range boundary and wrap | quantisation range, or missing extension bits |
| **seam** | wrong only along the edge between two faces | the two faces disagree at shared vertices | merge/dedup of vertices across surfaces |
| **blocky / mushy** | detail missing, over-compressed, or lower resolution than expected | the image itself converted wrong | DDS mip trim, block format choice |
| **washed out / too dark** | colours right in hue, wrong in intensity | colour-space flag | the `.options` `srgb` byte, or the DDS trailing byte |
| **alpha wrong** | foliage opaque as a card, or a solid surface see-through | the render family was classified wrong | shader classification, alpha-tested vs opaque |
| **z-fighting** | two surfaces flickering against each other as you move | coincident geometry, not a texture defect at all | decals, or duplicated geometry |

## How to report one

Four things, and the first two matter most:

1. **Which term** from the table.
2. **Whole surface or part of it.** "The whole floor" and "one corner of the
   floor" have different causes — the second is usually *wrapped*, the first
   usually *smeared* or *mis-tiled*.
3. **Does it change with viewing angle?** If yes it is probably a material
   response or a mip problem, not UVs. UV defects are fixed to the surface.
4. **Stand still.** The match server logs position; `client_player_update`
   packets decode to a world coordinate, which ties the pixels to a specific
   surface and from there to the source shader that fed it.

A screenshot with the term in the filename is enough — the two together
localise almost anything.

## Worked example

Thirteen screenshots, 2026-08-16. Named "distorted" in nine of them, and the
name was right: the brick wall was correct while the angled soffit above it was
streaked, small props were correct while the pool floor swirled, and a sheet of
paper was readable except for a chevron patch in one corner.

*Smeared*, plus one *wrapped* — both pointing at the vertex decode rather than
at any binding. The cause was the `s24_tc_base` remainder being added as whole
quantisation steps instead of a fraction of one:

```rust
// wrong
(f32::from(read_i16(..)?) + f32::from(tangent_alpha)) * TC_SCALE
// right: D3DCOLOR reaches the shader normalised to [0,1]
(f32::from(read_i16(..)?) + unorm8(tangent_alpha)) * TC_SCALE
```

X-Ray's own vertex shader settles it — `unpack_tc_base(v.tc, v.T.w, v.B.w)`
where `T`/`B` are `D3DCOLOR`, so `du = alpha/255`. The error reached 0.249
texture units and differed at every vertex, which is exactly what "smeared"
describes and what "wrong texture" would not have.

Had the same screenshots been called *wrong texture*, the search would have
started at material bindings and found nothing.
