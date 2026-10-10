// SPDX-License-Identifier: GPL-3.0-or-later
//! F.E.A.R. (LithTech Jupiter EX) map conversion.
//!
//! A fifth map-building path, shaped like [`crate::soc`]: the source is a
//! *compiled* Jupiter EX world, so there is no editor project and no source
//! material library, but the geometry, its material bindings and its physics
//! shapes are all present and exact.
//!
//! ```text
//! *.Arch00        -> layered archives, mounted in Default.archcfg order
//! *.World00p      -> render surfaces, physics shapes, object list, in one file
//! *.Mat00         -> shader path plus diffuse/normal/specular DDS bindings
//! Materials/*.dds -> already in a target-compatible container
//! ```
//!
//! Differences from the SoC path that shape the code:
//!
//! * the world file carries its own object list in plain text-property records,
//!   so multiplayer spawn points (`GameStartPoint`) and authored lights are
//!   decoded rather than inferred;
//! * materials name their textures directly — there is no separate shader
//!   library to consult, and classification keys on the material's `.fx` path;
//! * LithTech world units are centimetres where the target's are metres, so
//!   conversion scales all geometry by [`SCALE`].
//!
//! The module names honour the engine's lineage: LithTech begat Jupiter,
//! Jupiter begat Jupiter EX, and this port begat Lilith.
//!
//! ```text
//! codec/    decode Jupiter EX formats        (no target knowledge, no I/O)
//! source    mount an installed archive set
//! ```

pub mod build;
pub mod codec;
pub mod convert;
pub mod probe;
pub mod source;

/// World units per target metre: Jupiter EX maps are authored in centimetres.
///
/// The convention is LithTech's, not measured fact — treat it as unverified
/// until a converted doorway stands about two metres tall in the client.
pub const SCALE: f32 = 0.01;
