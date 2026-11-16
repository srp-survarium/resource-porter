// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoders for the Shadow of Chernobyl on-disk formats.
//!
//! Everything in this module is a pure, allocation-only decode of bytes that
//! X-Ray 1.x wrote. Nothing here knows the target engine exists, touches the
//! filesystem, or makes a porting decision — that separation is what lets the
//! decoders be tested against synthetic containers and reused by tools other
//! than the map builder.
//!
//! | module | file | role |
//! | --- | --- | --- |
//! | [`archive`] | `gamedata.db*` | the packed container and its file table |
//! | [`lzhuf`] | `gamedata.db*` | adaptive-Huffman LZSS, used by the file table |
//! | [`cipher`] | `gamedata.db*` | the shipped file-table obfuscation |
//! | [`chunks`] | any | the shared `id`/`size`/payload container |
//! | [`level`] | `level`, `level.geom` | shader table, visual array, shared buffers |
//! | [`ogf`] | `level` visuals | OGF v4 headers and geometry references |
//! | [`vertex`] | `level.geom` | D3D9 vertex declarations and vertex decoding |
//! | [`cform`] | `level.cform` | collision triangles and their material ids |
//! | [`gamemtl`] | `gamemtl.xr` | physical-material id to name |
//! | [`blender`] | `shaders.xr` | shader name to blender class id |
//! | [`omf`] | `meshes/**/*.omf` | skeletal motion banks and their bone names |
//! | [`skeleton`] | `meshes/actors/**/*.ogf` | actor bone hierarchy and rest pose |

pub mod archive;
pub mod blender;
pub mod cform;
pub mod chunks;
pub mod cipher;
pub mod details;
pub mod gamemtl;
pub mod level;
pub mod lzhuf;
pub mod ogf;
pub mod omf;
pub mod skeleton;
pub mod spawn;
pub mod texture_metadata;
pub mod vertex;
