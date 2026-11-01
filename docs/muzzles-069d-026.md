# Muzzle devices of 0.69d0 for v0.26g

`muzzles_069d` (`vostok-port port-muzzles`) ports 0.69d0's flash hiders and muzzle brakes as
items of 0.26g's weapon kit and fits them to our guns. skhid-srp's client patch 90 draws them
(its `docs/notes/muzzle-devices.md` is the spec); the rules are the module's own docs.

## Build

```sh
# 0.69d0's files: game.options, five languages, every module model under muzzlebrakes/,
# firearrestors/, barrels/ with materials and textures, every weapon's skinned render folder.
vostok-port extract-bundle --db full069.split $(sed 's/^/--prefix /' donor_paths.txt) --output TREE
cargo run --release --example pack-donor-tree -- TREE/overlay donor.db
# The manifest from game.options and our dictionary (the device ids stay in muzzle-devices.tsv):
scripts/muzzle-devices-manifest.py game.leaves dict.leaves ports/muzzle-devices-guns.tsv \
    ports/muzzle-devices.tsv ports/muzzle-devices-069d.json [<gun>...]
# The bundle (run from the output's folder: the texture encoder sees only its working folder):
vostok-port port-muzzles --manifest ports/muzzle-devices-069d.json --donor donor.db \
    --target-db STOCK --target-patch ... --output B
scripts/render-muzzle-icons.py --bundle B --lobby-swf <players' lobby_menu.swf>
vostok-port pack from-bundle ...   # skhid-srp's scripts/muzzle-devices/make_pack.sh does all
```

The bundle: `overlay/`, `port-manifest.json` (each gun's cut and fit numbers, each device's
stats and guns), `muzzle-edits.json` (the content-edit list giving each gun its stock device),
`texts.json` (the devices' 0.69d0 names), `renders/<gun>.png` (the gun with its stock device in
orange, then its muzzle bare, with the stock device, and with each device it takes in blue).

## What is left out

`ports/muzzle-devices-guns.tsv` says why for each gun: fused brakes (AK-12, vz. 58), integral
or built-in suppressors (VSS, VAL, VSK-94, VSSK, MP7 SD, 9A-91, KRISS Vector), meshes or barrels
that are not 0.69d0's (Vepr-12 premium, SPAS-12, Benelli M4, Uzi), guns no device fits. Devices:
no suppressor, by name or by a suppressor's shot effect, and no Zenit DTK-4 (a closed can).
