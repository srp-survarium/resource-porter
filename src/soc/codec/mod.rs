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
//! | [`chunks`] | any | the shared `id`/`size`/payload container |
//! | [`level`] | `level`, `level.geom` | shader table, visual array, shared buffers |
//! | [`ogf`] | `level` visuals | OGF v4 headers and geometry references |
//! | [`vertex`] | `level.geom` | D3D9 vertex declarations and vertex decoding |
//! | [`cform`] | `level.cform` | collision triangles and their material ids |
//! | [`gamemtl`] | `gamemtl.xr` | physical-material id to name |

pub mod cform;
pub mod chunks;
pub mod gamemtl;
pub mod level;
pub mod ogf;
pub mod vertex;
