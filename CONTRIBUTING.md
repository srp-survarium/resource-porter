# Contributing

Build and check changes with a stable Rust toolchain and C linker, or enter
the provided environment with `nix develop`:

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo test --locked --manifest-path deps/vostok-resources-db/Cargo.toml
```

Keep changes scoped to a source format or conversion contract. Add meaningful
regression coverage for decoder and conversion bugs using synthetic fixtures;
unit tests should run without retail game data. Keep output resources under
`.work/` and retain provenance and hash verification when changing emitters.

Manifests in `ports/` describe reviewed target templates and overrides. When
changing one, identify the source and target builds, the observed defect, and
whether validation covered decoding, packed resources or a client playtest.
Do not treat a successful hash check as a rendering test.

Issue reports should include the command, manifest, source/target version,
error or visual defect, and relevant log excerpts. Remove account details,
credentials and personal paths before sharing logs. See the
[texture defect vocabulary](docs/texture-defect-vocabulary.md) for visual
reports.

Supply game assets locally instead of attaching databases, extracted textures,
models or executables to changes. Keep source notices and dependency
provenance. Contributions to this project use GPL-3.0-or-later.
