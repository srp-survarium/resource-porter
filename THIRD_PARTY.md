# Dependency and asset provenance

## Archive reader and writer

The archive code is the `resources-db` crate of
[srp-survarium/skhid-srp](https://github.com/srp-survarium/skhid-srp)
(`srp-026g/crates/resources-db`), a git dependency pinned to a commit in
`Cargo.toml`. It began as srp-survarium/vostok-resources-db (archived), whose
copy this repository used to carry under `deps/`. The source authors are
sheepfleece / sushi-shi; it is licensed under GPL-3.0-or-later.

Rust dependencies fetched from crates.io retain their respective licenses.
The root `Cargo.lock` records the versions used to build the porter.

## Included flower atlas

`assets/cs-training-camp-addons/wildflower-atlas.png` is a generated atlas used
by the Clear Sky meadow addon. Its provenance is recorded as
`imagegen:wildflower-atlas` by the addon builder. It is included under this
project's GPL-3.0-or-later license. It is separate from the externally supplied
game and mod textures used in local experiments.

## External game inputs

Survarium, S.T.A.L.K.E.R., F.E.A.R. and STK2 input files are supplied
outside this repository. Their names and resource paths identify supported
formats and conversions. Game assets retain their rights holders' terms,
including when copied or converted into bundles; the project license applies
to the contributors' tooling, manifests and documentation.
