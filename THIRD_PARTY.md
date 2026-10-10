# Dependency and asset provenance

## Archive reader and writer

`deps/vostok-resources-db` is included as source so a fresh clone can build
without unpublished submodule commits. Its upstream is
[srp-survarium/vostok-resources-db](https://github.com/srp-survarium/vostok-resources-db).
The imported revision is `104ef7b46437aee95a4b85611fd424e3f73c6c32`,
which includes the local packer and overlay support used by this project.
The source authors are sheepfleece / sushi-shi. The included copy is licensed
under GPL-3.0-or-later with this repository; its original documentation is
preserved in the dependency directory.

Rust dependencies fetched from crates.io retain their respective licenses.
The root `Cargo.lock` records the versions used to build the porter.

## Included flower atlas

`assets/cs-training-camp-addons/wildflower-atlas.png` is a generated atlas used
by the Clear Sky meadow addon. Its provenance is recorded as
`imagegen:wildflower-atlas` by the addon builder. It is included under this
project's GPL-3.0-or-later license. It is separate from the externally supplied
game and mod textures used in local experiments.

## External game inputs

Survarium, S.T.A.L.K.E.R., F.E.A.R., Dirty Bomb and STK2 input files are supplied
outside this repository. Their names and resource paths identify supported
formats and conversions. Game assets retain their rights holders' terms,
including when copied or converted into bundles; the project license applies
to the contributors' tooling, manifests and documentation.
