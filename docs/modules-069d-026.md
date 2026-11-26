# 0.69d0's modules for 0.26g's guns (`port-modules`)

`vostok-port port-modules` makes, from 0.69d0's `gameplay/game.options` and models, what skhid-srp's
weapon attachments need of content (its `srp-026g/docs/notes/attachments.md`): each gun on 0.69d0's
model in two bodies, 0.69d0's modules as static models, the modules table, and a check of all of it
against 0.69d0 itself. It takes the place of `remodel-weapons-069d` (which makes the whole bodies
alone) and of the parked `port-muzzles`.

```text
vostok-port port-modules --id weapon-modules \
    --donor-spec <skhid-srp>/srp-026g/scripts/remodel-069/remodel-069d.json \
    --items <skhid-srp>/srp-026g/scripts/remodel-069/items.tsv \
    --ids <skhid-srp>/srp-026g/scripts/weapon-modules/ids.tsv \
    --donor donor.db --target-db resources.db --target-patch resources.db.patch_12 ... \
    --output bundle [--only <0.69d0 item>]... [--renders <folder>]
```

`donor.db` is 0.69d0's weapon models, materials and textures with `gameplay/game.options` (skhid-srp's
`scripts/weapon-modules/make_pack.sh` makes it). The bundle is one for `pack from-bundle`;
`bundle/modules-069d.json` is the record: each gun's plan, its measured shift, its look against the
live gun, and every row of the check.

## What 0.69d0's data says (`donor_069d/catalog.rs`, `plan.rs`)

A gun is a base mesh with slots; every other part is a module in a slot, its own parts too
(`default_modules`). No gun is named in the code: everything below is read from the tables.

- **Own parts.** The gun's modules attach in 0.69d0's order, each in the slot its entry names; its
  `slots_to_add` and `slots_to_override` then hold for the next. Which of them the body shows is the
  sidecar's (`separable` leaves optics and a premium item's extras to the kit).
- **Kinds.** A grip carries the left hand's sixteen locators (and is no forend); a muzzle device goes
  in muzzle threads only and brings no slot; an optic moves the eye and is of 0.69d0's scope groups.
  A suppressor is none of them and is never ported.
- **Fits.** A module `module_to_item_links` gives the gun, of a kind, with an item of ours (`--ids`).
  It goes in the first slot it names that the gun with its own parts has; else in the first it names
  that carriers bring: linked modules of no kind (a rail bracket, a tactical forend, a barrel, a
  dovetail mount), the shortest chain, and among equals the one 0.69d0 links first. Neither: a
  refusal with its reason in the record.
- **Second sights** (table version 4, the weapon kit's slot 6). An optic of the fits whose slots name
  `angled_collim_short_l` or `_r` (0.69d0's DeltaPoint Pro and RSA-S) is a second sight too, a fit
  a side: on each side the gun has that slot or carriers bring it, by the same chain rule (the NcStar
  MPR45 offset rail on the gun's `angled_picatinnyrail_l` or `_r`; on the M4A1 and the L85A2 their
  tactical forend first, which brings the angled rails). A gun 0.69d0 links such a carrier and that
  has none of its slots (the FAMAS and the MPR45) is refused one, with the reason. Its row is the
  optic's dict id as a second sight, with its side slots alone; its eye there is the aim point the
  slot adds (`aim_3`, `aim_4`, at the gun's eye plane: `plane_slot: body_aim`), its reticle's slot
  (`reticle_2`, `reticle_3`) an add; the same optic's scope row keeps its other slots. Its aimed
  model is the scope's (one an optic, the reticle at the optic's `aim`). The second sights' rows and
  their carriers come after every other row of the table.
- **A premium or camouflage item** whose own links fit nothing takes the fits of its base gun: the
  item of the longest prefix of its name 0.69d0 links modules to.
- **Exceptions.** `exceptions.<item>` gives a module other models on that gun, and other slots: its
  `slots_to_attach_to` entries lie over the module's own, place by place, and a module in a slot
  brings what every entry naming that slot says. LIKELY, read so because 0.69d0 links the PKP its
  own foregrip, which has a slot there only if the rail bracket still brings `picatinny_grip` on
  the PKP and the RPD.
- **Stock parts.** An own part a fit can displace: a fit or a carrier goes in its slot, closes it
  (`slots_to_disable`), folds it (`slots_to_change_param` 1, and the part has a `model_param1`), or
  it stands on a slot such a part brings.

`cargo run --example plan-modules-069d -- <game.options> <sidecar>` prints every gun's plan.

## The two bodies (`donor_069d.rs`, `Body`)

- `whole`, at the sidecar's `model_path`: the gun with every own part its body shows, as
  `remodel-weapons-069d` made it. Drawn when nothing is fitted.
- `bare`, at `<model_path>_bare`: the same without the stock parts. The same skeleton, bind pose and
  locators.

Both lie on the live model's mesh by one **measured shift** (`fit.rs`): a trimmed nearest-point fit
of the remodel's vertices on the live model's; under 1 mm it is none, else it is applied to 0.1 mm.
The sidecar carries no offset (a typed one is refused). The record has the shift, the gap before and
after, and what each body has that the other lacks.

## Module models and the table (`modules.rs`)

- A module's model is 0.69d0's static model as it is (positions and UVs untouched, 0.26g's vertex
  layout), at `models/weapons/modules_069/<path under weapons/modules/>.model`; its materials are
  made as the guns' (the same files where a gun bakes the same module). A part's origin is the frame
  of the slot it goes in.
- The table is `weapon_modules::encode`'s bytes (skhid-srp's crate, by git revision in
  `Cargo.toml`), at `gameplay/weapon_modules.srpm`: a gun record for every live item of `--items`
  (its dict id from the players' dictionary), its slots, its aimed idle pose (its config's
  `aimed_idle` clip, else `idle`'s, at its start), its stock parts, carriers and fits, its bare
  body's path; a row a module and role, a row of its own for an exception's models or rules.
- **Locators.** A gun's slot is its locator as it is: the game evaluates a record as `half turn x
  offset x bone`, and 0.69d0 draws a module at a gun's locator the same way (`socket.rs`). A
  module's own slot, eye and fire-point locators are written plainly in its model's space (a
  forend's rails lie on its mesh at their positions, the left one at -x); the table's rule for a
  part on a part is `locator x half turn x parent`, so such a locator `L` is written as `half turn x
  L`. A grip's hand locators are written as a gun rig's (the left hand at +x, as on a gun's
  half-turned bones) and go in as `L x half turn`: the wrist's record is then `L x the grip's
  offset`, what the gun's own hand locator would be there.

## The check

For every gun and every fit alone, `weapon_modules::resolve` on the written table and the bundle's
models, against 0.69d0: its own base model in its own bind pose, each module at 0.69d0's locator by
the engine's arithmetic, a module on a module at that one's locator. Compared in mesh space, ours
less the gun's shift: every vertex of every part drawn, the optic's frame and eye, the fire point,
the wrist; that the same parts are drawn; limit 0.5 mm. A second sight is checked on each side
(`second_sight` names it in the record): its parts, its frame (`second sight`), that the gun's own
eye stays the eye, and its eye (`second eye`) against 0.69d0's `aim_3`/`aim_4` slot after
`get_slot_transform`'s plane rule (the origin along its line to the plane of the gun's `body_aim`),
with `second eye line` its line alone; its reticle's centre is a row of `centres` with its side. A failure is a row of the record and a
`FAIL` line; nothing is fixed per gun. With `--renders`, each comparison is also a picture
(`<item>/<module>.png`: 0.69d0 left, ours right, from the side and from above), and each gun
against its live model is `<item>/_look.png`.
