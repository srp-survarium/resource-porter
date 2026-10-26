// SPDX-License-Identifier: GPL-3.0-or-later
//! `level.details`: X-Ray's detail objects (grass, flowers, ground litter).
//!
//! The layout is `xrRender/DetailFormat.h` (version 3, the same in Shadow of
//! Chernobyl and Clear Sky):
//!
//! ```text
//! chunk 0  header   u32 version, u32 object_count, i32 offs_x, i32 offs_z,
//!                   u32 size_x, u32 size_z
//! chunk 1  objects  one child chunk per object (ids 0..object_count), each a
//!                   `CDetail::Load` record: shader\0 texture\0 u32 flags,
//!                   f32 min_scale, f32 max_scale, u32 vertex_count,
//!                   u32 index_count, {float3 P; f32 u, v} vertices, u16 indices
//! chunk 2  slots    size_x * size_z `DetailSlot`s of 16 bytes, row-major in z
//! ```
//!
//! A slot is a 2 m square of ground. Its first eight bytes are bit fields
//! (low bit first): `y_base:12` (20 cm units from -200 m), `y_height:8`
//! (10 cm units), four 6-bit object ids (`0x3f` empty), then the lighting
//! terms `c_dir:4 c_hemi:4 c_r:4 c_g:4 c_b:4`. The last eight bytes are four
//! `u16` palettes, one per object id, of four 4-bit densities at the slot's
//! corners.

use anyhow::{Context, Result, bail, ensure};

use super::chunks::{Chunks, Cursor};

/// `DETAIL_VERSION`.
pub const VERSION: u32 = 3;
/// `DETAIL_SLOT_SIZE`, metres.
pub const SLOT_SIZE: f32 = 2.0;
/// `DetailSlot::ID_Empty`.
pub const EMPTY: u8 = 0x3f;
/// `DO_NO_WAVING`: the object does not sway (ground litter).
pub const NO_WAVING: u32 = 0x0001;

#[derive(Clone, Debug, PartialEq)]
pub struct DetailVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

/// One detail model (`CDetail`).
#[derive(Clone, Debug, PartialEq)]
pub struct DetailObject {
    pub shader: String,
    pub texture: String,
    pub flags: u32,
    pub min_scale: f32,
    pub max_scale: f32,
    pub vertices: Vec<DetailVertex>,
    pub indices: Vec<u16>,
}

impl DetailObject {
    pub fn waves(&self) -> bool {
        self.flags & NO_WAVING == 0
    }
}

/// One 2 m slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailSlot {
    pub bits: u64,
    pub palette: [u16; 4],
}

impl DetailSlot {
    pub fn y_base(&self) -> f32 {
        (self.bits & 0xfff) as f32 * 0.2 - 200.0
    }

    pub fn y_height(&self) -> f32 {
        ((self.bits >> 12) & 0xff) as f32 * 0.1
    }

    pub fn id(&self, part: usize) -> u8 {
        ((self.bits >> (20 + 6 * part)) & 0x3f) as u8
    }

    /// `c_dir`, `c_hemi`: the baked sun and sky visibility, 0..15.
    pub fn sun(&self) -> u8 {
        ((self.bits >> 44) & 0xf) as u8
    }

    pub fn hemi(&self) -> u8 {
        ((self.bits >> 48) & 0xf) as u8
    }

    /// Corner densities of one part, 0..15, in `DetailPalette` order
    /// (`a0` at the slot's minimum x and z, `a1` +x, `a2` +z, `a3` +x+z).
    pub fn densities(&self, part: usize) -> [u8; 4] {
        let p = self.palette[part];
        std::array::from_fn(|corner| ((p >> (4 * corner)) & 0xf) as u8)
    }

    pub fn is_empty(&self) -> bool {
        (0..4).all(|part| self.id(part) == EMPTY)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub offset: [i32; 2],
    pub size: [u32; 2],
    pub objects: Vec<DetailObject>,
    pub slots: Vec<DetailSlot>,
}

impl Details {
    /// The slot at database coordinates, and its world-space minimum corner
    /// (`DetailHeader::slot_min_x/z`).
    pub fn slot(&self, x: u32, z: u32) -> (&DetailSlot, [f32; 2]) {
        let slot = &self.slots[(z * self.size[0] + x) as usize];
        let min = [
            (x as i32 - self.offset[0]) as f32 * SLOT_SIZE,
            (z as i32 - self.offset[1]) as f32 * SLOT_SIZE,
        ];
        (slot, min)
    }

    /// The world slot coordinates `CDetailManager` seeds a slot's random
    /// streams with (`Slot::sx`, `Slot::sz`).
    pub fn world_slot(&self, x: u32, z: u32) -> [i32; 2] {
        [x as i32 - self.offset[0], z as i32 - self.offset[1]]
    }
}

pub fn parse(bytes: &[u8]) -> Result<Details> {
    let chunks = Chunks::new(bytes, "level.details");
    let mut header = Cursor::new(chunks.require(0)?, "level.details header");
    let version = header.u32()?;
    ensure!(
        version == VERSION,
        "level.details version {version}, expected {VERSION}"
    );
    let count = header.u32()?;
    let offset = [header.i32()?, header.i32()?];
    let size = [header.u32()?, header.u32()?];

    let mut objects = Vec::with_capacity(count as usize);
    for (index, chunk) in Chunks::new(chunks.require(1)?, "level.details objects")
        .iter()
        .enumerate()
    {
        let chunk = chunk?;
        ensure!(
            chunk.id as usize == index,
            "level.details object chunk {} out of order (expected {index})",
            chunk.id
        );
        objects
            .push(parse_object(chunk.payload).with_context(|| format!("detail object {index}"))?);
    }
    ensure!(
        objects.len() == count as usize,
        "level.details header declares {count} objects, the file has {}",
        objects.len()
    );

    let raw = chunks.require(2)?;
    let expected = size[0] as usize * size[1] as usize;
    ensure!(
        raw.len() == expected * 16,
        "level.details has {} slot bytes, a {}x{} grid needs {}",
        raw.len(),
        size[0],
        size[1],
        expected * 16
    );
    let slots: Vec<DetailSlot> = raw
        .chunks_exact(16)
        .map(|slot| DetailSlot {
            bits: u64::from_le_bytes(slot[..8].try_into().unwrap()),
            palette: std::array::from_fn(|part| {
                u16::from_le_bytes(slot[8 + 2 * part..10 + 2 * part].try_into().unwrap())
            }),
        })
        .collect();
    for (index, slot) in slots.iter().enumerate() {
        for part in 0..4 {
            let id = slot.id(part);
            if id != EMPTY && usize::from(id) >= objects.len() {
                bail!(
                    "detail slot {index} names object {id}; the file has {}",
                    objects.len()
                );
            }
        }
    }
    Ok(Details {
        offset,
        size,
        objects,
        slots,
    })
}

fn parse_object(payload: &[u8]) -> Result<DetailObject> {
    let mut cursor = Cursor::new(payload, "detail object");
    let shader = cursor.stringz()?;
    let texture = cursor.stringz()?;
    let flags = cursor.u32()?;
    let min_scale = cursor.f32()?;
    let max_scale = cursor.f32()?;
    let vertex_count = cursor.u32()? as usize;
    let index_count = cursor.u32()? as usize;
    ensure!(
        index_count % 3 == 0,
        "{index_count} indices is not a triangle list"
    );
    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        let position = cursor.float3()?;
        vertices.push(DetailVertex {
            position,
            uv: [cursor.f32()?, cursor.f32()?],
        });
    }
    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        let index = cursor.u16()?;
        ensure!(
            usize::from(index) < vertex_count,
            "index {index} past {vertex_count} vertices"
        );
        indices.push(index);
    }
    Ok(DetailObject {
        shader,
        texture,
        flags,
        min_scale,
        max_scale,
        vertices,
        indices,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = id.to_le_bytes().to_vec();
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend(payload);
        out
    }

    /// A one-quad object and a grid of slots, as `level.details` bytes.
    pub(crate) fn encode(
        offset: [i32; 2],
        size: [u32; 2],
        flags: u32,
        slots: &[DetailSlot],
    ) -> Vec<u8> {
        let mut header = Vec::new();
        for value in [VERSION, 1] {
            header.extend(value.to_le_bytes());
        }
        header.extend(offset[0].to_le_bytes());
        header.extend(offset[1].to_le_bytes());
        header.extend(size[0].to_le_bytes());
        header.extend(size[1].to_le_bytes());
        let mut object = b"details\\blend\0build_details\0".to_vec();
        object.extend(flags.to_le_bytes());
        object.extend(1.0_f32.to_le_bytes());
        object.extend(2.0_f32.to_le_bytes());
        object.extend(4_u32.to_le_bytes());
        object.extend(6_u32.to_le_bytes());
        for (p, uv) in [
            ([-0.2_f32, 0.0, 0.0], [0.0_f32, 1.0]),
            ([0.2, 0.0, 0.0], [1.0, 1.0]),
            ([0.2, 0.5, 0.0], [1.0, 0.0]),
            ([-0.2, 0.5, 0.0], [0.0, 0.0]),
        ] {
            for v in p.into_iter().chain(uv) {
                object.extend(v.to_le_bytes());
            }
        }
        for i in [0_u16, 1, 2, 0, 2, 3] {
            object.extend(i.to_le_bytes());
        }
        let mut raw = Vec::new();
        for slot in slots {
            raw.extend(slot.bits.to_le_bytes());
            for p in slot.palette {
                raw.extend(p.to_le_bytes());
            }
        }
        let mut out = chunk(0, &header);
        out.extend(chunk(1, &chunk(0, &object)));
        out.extend(chunk(2, &raw));
        out
    }

    /// A slot holding `id` in part 0 at full density, over ground at 0 m.
    pub(crate) fn full_slot(id: u8, y_base: f32, y_height: f32) -> DetailSlot {
        let base = ((y_base + 200.0) / 0.2).round() as u64;
        let height = (y_height / 0.1).round() as u64;
        let ids = u64::from(id)
            | (u64::from(EMPTY) << 6)
            | (u64::from(EMPTY) << 12)
            | (u64::from(EMPTY) << 18);
        DetailSlot {
            bits: base | (height << 12) | (ids << 20) | (7 << 44) | (9 << 48),
            palette: [0xffff, 0, 0, 0],
        }
    }

    #[test]
    fn decodes_header_objects_and_slot_fields() {
        let slot = full_slot(0, -1.0, 2.0);
        let bytes = encode(
            [1, 2],
            [2, 1],
            NO_WAVING,
            &[
                slot,
                DetailSlot {
                    bits: u64::MAX,
                    palette: [0; 4],
                },
            ],
        );
        let details = parse(&bytes).unwrap();
        assert_eq!(details.offset, [1, 2]);
        assert_eq!(details.objects.len(), 1);
        let object = &details.objects[0];
        assert_eq!(object.texture, "build_details");
        assert!(!object.waves());
        assert_eq!(object.indices, [0, 1, 2, 0, 2, 3]);
        let (slot, min) = details.slot(0, 0);
        assert!((slot.y_base() + 1.0).abs() < 1e-4);
        assert!((slot.y_height() - 2.0).abs() < 1e-4);
        assert_eq!([slot.id(0), slot.id(1)], [0, EMPTY]);
        assert_eq!([slot.sun(), slot.hemi()], [7, 9]);
        assert_eq!(slot.densities(0), [15; 4]);
        assert_eq!(min, [-2.0, -4.0]);
        assert_eq!(details.world_slot(1, 0), [0, -2]);
        assert!(details.slot(1, 0).0.is_empty());
    }

    #[test]
    fn rejects_truncated_slots_and_unknown_objects() {
        let slot = full_slot(0, 0.0, 1.0);
        let mut bytes = encode([0, 0], [1, 1], 0, &[slot]);
        bytes.truncate(bytes.len() - 1);
        // The slot chunk's declared size now runs past the end of the file.
        assert!(parse(&bytes).is_err());
        let bad = full_slot(5, 0.0, 1.0);
        assert!(parse(&encode([0, 0], [1, 1], 0, &[bad])).is_err());
    }
}
