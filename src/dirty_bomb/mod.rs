// SPDX-License-Identifier: GPL-3.0-or-later
//! Dirty Bomb (UE3 package 868/licensee 9) map source family.
//!
//! This path deliberately starts at the cooked-package boundary.  A package
//! must pass the same summary, LZO, name/import/export and tagged-property
//! checks that the retail UE3 loader applies before later conversion code is
//! allowed to see an actor, mesh or texture.

pub mod build;
pub mod convert;
pub mod material;
pub mod model;
pub mod package;
pub mod probe;
pub mod scene;
pub mod source;
pub mod static_mesh;

/// Dirty Bomb inherits UE3's character/world convention of two centimetres
/// per Unreal unit (an 88-unit standing pawn is approximately 1.76 metres).
pub const SCALE: f32 = 0.02;
