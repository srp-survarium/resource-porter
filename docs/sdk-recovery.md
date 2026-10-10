# Recovered Vostok SDK pipeline

This repository supplies the data-recovery and build backend for the older
STK2/X-Ray 2.0 editor. The editor remains a Visual Studio 2008 C++/CLI program;
the backend is a separate Rust executable so binary-format work is testable and
does not destabilize the recovered engine.

## Process boundary

`vostok-sdk-backend` reads one strict JSON request from stdin or `--request`
and writes JSON Lines events to stdout. Protocol and project formats are both
versioned independently. Unknown request fields, unsafe project names, path
traversal, duplicate variant IDs, altered preserved originals, and
unrepresentable edits fail closed.

Protocol 1 operations are:

| operation | result |
| --- | --- |
| `handshake` | backend version and capabilities |
| `decompile-map` | an atomic SDK project with preserved binaries, canonical editable configs, variant metadata, dependency inventory, and recovery reports |
| `validate-project` | validates manifest structure, original hashes, and edited config representability |
| `build-preview` | an atomic loose-resource overlay and build report |
| `roundtrip-resources` | semantic/byte round-trip evidence for selected binary configs |

The editor-side client launches one backend process per operation. It never
loads Rust through a C ABI. Requests are small JSON values and large data stays
in files, which keeps the recovered 2008 runtime isolated from modern code.

## Project layout

```text
project.xprj                         editor bootstrap
project.sdk.json                     authoritative manifest and dependency inventory
roundtrip-report.json                per-project binary evidence
.sdk/variants/<id>/
  client_project.original            immutable compiled input
  client_project.json                editable canonical tree
  server_project.original            immutable compiled input, when present
  server_project.json                editable canonical tree, when present
  client_project.lua                 exact source/debug companion, when shipped
  server_project.lua                 exact source/debug companion, when shipped
sources/terrain/                      reserved recovered terrain authoring data
sources/sounds/                       reserved recovered sound authoring data
```

Canonical configs retain named-field order and store every floating-point value
as raw IEEE-754 bits. This avoids ordinary JSON losing NaN payloads, signed
zero, or exact values. The original compiled files are kept separately and
hash-locked.

An unchanged canonical tree is rebuilt by copying its original file, giving
byte identity. An edited tree is deterministically encoded, parsed again, and
compared to the requested canonical tree before it is emitted. The build report
states which path was used for every file.

## Weather variants

Variants are one logical SDK project. `project.sdk.json` records each runtime
project and exact structural fingerprints shared by all variants. This keeps
day/evening/rain ownership explicit without guessing that similarly positioned
objects are identical. Full canonical trees remain per variant until a
lossless field-level override representation is proven.

## Running the recovered path

Build and inspect the backend:

```sh
cargo build --bin vostok-sdk-backend
target/debug/vostok-sdk-backend --request examples/sdk/level03-decompile.json
target/debug/vostok-sdk-backend --request examples/sdk/level03-build-preview.json
```

The training-map request is in
`examples/sdk/training-camp-decompile.json`. Request paths are deliberately
ordinary filesystem paths, so copies of the examples can point at another
owned `resources.db` without recompilation.

The recovered engine worktree adds a `Build Survarium Preview` project action.
Set `VOSTOK_SDK_BACKEND` to an absolute Windows-visible path when the executable
is not beside the editor DLL.

## Current evidence

The three retail level_03 variants decompile as one project. The current sample
contains 5,389 reachable resources and preserves 12 project/config-Lua files
byte-for-byte in an unchanged preview. Across all three variants the structural
intersection contains 1,183 render visuals, 1,790 collision objects, and 203
client objects. The day variant contains 1,233 render visuals, 1,858 collision
objects, 248 client objects, and 7 server objects.

The current `cs_training_camp` database also decompiles and rebuilds its client
and server projects byte-for-byte. Its dependency closure contains 2,721
resources with no unresolved references. It currently ships one runtime
variant; inventing two weather variants is intentionally outside recovery.

These generated demonstrations live under `.work/` and are not source of
truth. Re-run the checked-in requests to reproduce them.

## Engine build

The SDK worktree derives an isolated `SDK Release|Win32` solution and Ninja
graph. It does not enable editor projects in, or otherwise change, the retail
`Master Gold|Win32` matching graph. The generator also confines the Wine CLR
override to the SDK process.

```sh
nix develop --command python3 -m vostok.sdk_build --regen
nix develop --command python3 -m vostok.sdk_build editor
```

Graph generation is verified, and the untouched retail graph completes all 155
compile/library prerequisites before its final executable-only link encounters
the absent proprietary Scaleform `libgfxexpat.lib`. A complete editor binary
additionally requires the VS2008-era .NET Framework 3.5 SDK (`alink.dll` and
framework registry keys), the SDK's Release-native library configurations, and
a `resgen`/managed-resource stage for `.resx` inputs that the current graph
converter cannot emit. The backend rejects unsupported preview inputs rather
than inventing compatible resources.

The backend currently builds on the Linux recovery host. Packaging its Windows
`.exe` requires an i686 Windows Rust target/linker, which is not installed in
this pinned environment; the editor supports an explicit `VOSTOK_SDK_BACKEND`
path so that artifact can be supplied without changing editor code.
