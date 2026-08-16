//! Turn decoded Shadow of Chernobyl data into v0.100b structures.
//!
//! Each submodule owns one boundary between the engines, and each is written so
//! that the *decision* is visible and reviewable rather than buried in the
//! emission step:
//!
//! | module | boundary |
//! | --- | --- |
//! | [`surface`] | many small visuals -> few merged target render surfaces |
//! | [`collision`] | `level.cform` -> the target collision family payloads |
//! | [`game_material`] | X-Ray physical material names -> v0.100b ids |
//! | [`render_material`] | X-Ray shader names -> v0.100b material families |
//! | [`animation`] | `bip01` motions -> clips on the v0.100b character rig |
//!
//! Nothing here writes a file or opens a database. The conversion result is
//! data that [`super::build`] emits and records.

pub mod animation;
pub mod collision;
pub mod game_material;
pub mod render_material;
pub mod spawn;
pub mod surface;
