// SPDX-License-Identifier: GPL-3.0-or-later
//! Protocol implementation for the community-patched v0.21d build 2010.
//! See docs/v021d/SERVER-PLAN.txt for implementation and validation status.

pub mod coordinator;
pub mod setup;
pub mod snapshot;
pub mod state;
pub mod transport;
pub mod wire;
