// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoders for the Jupiter EX formats the port reads.
//!
//! Everything here is pure: bytes in, structures out, `Result` on anything
//! unexpected. Reading files and choosing target equivalents happen elsewhere.

pub mod arch;
pub mod mat;
pub mod world;
