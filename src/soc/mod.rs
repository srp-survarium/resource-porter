//! Shadow of Chernobyl map conversion.
//!
//! This is a fourth map-building path alongside `build-map`,
//! `build-target-native-map` and `build-stk2-map-slice`. It differs from all
//! three in where its meaning comes from: the source is a *compiled* X-Ray 1.x
//! level, so there is no editor project and no source material library, but the
//! geometry, its shader bindings and its physical materials are all present and
//! exact.
//!
//! ```text
//! level          -> shader table, per-visual geometry references
//! level.geom     -> shared vertex/index buffers with D3D9 declarations
//! level.cform    -> collision triangles with game-material ids
//! gamemtl.xr     -> game-material id -> name
//! textures/*.dds -> already in a target-compatible container
//! ```
//!
//! The converter regroups the world by shader, converts each group into a
//! target static surface, translates the collision material dictionary through
//! reviewed names, and clones target project/material/settings templates rather
//! than inventing target structures.
//!
//! Reading the packed `gamedata.db*` archives is deliberately out of scope:
//! they are encrypted and LZ-compressed, and unpacking them once with an
//! external tool keeps a proprietary key table out of this repository. Point
//! `--source-root` at an unpacked `gamedata` tree.

//! ```text
//! codec/    decode X-Ray formats            (no target knowledge, no I/O)
//! convert/  decide the v0.100b equivalent   (no I/O)
//! source    read an unpacked gamedata tree
//! manifest  declare what cannot be derived
//! build     emit the bundle and record it
//! ```

pub mod build;
pub mod codec;
pub mod convert;
pub mod manifest;
pub mod source;

pub use build::{SocMapBuildRecord, build_bundle};
pub use manifest::SocMapManifest;
