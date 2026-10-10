// SPDX-License-Identifier: GPL-3.0-or-later
//! X-Ray 1.x map conversion, initially built for Shadow of Chernobyl.
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
//! The source may be an unpacked `gamedata` tree, a Shadow of Chernobyl
//! installation with root-level `gamedata.db*`, or Clear Sky/Call of Pripyat's
//! split archive layout.

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
