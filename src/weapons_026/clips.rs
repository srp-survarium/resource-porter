// SPDX-License-Identifier: GPL-3.0-or-later
//! Animation clips (`.b-spline`) in 0.26g's layout.
//!
//! A clip is a memory image the engine fixes up in place
//! (`skeleton_animation.rs` has the whole layout): a 4-byte header (bones,
//! event channels, type), nine 8-byte pointer slots a bone, one header an
//! event channel, then the curves, the event knots and names. Every later
//! build we port from (0.29a3, 0.31a2, 0.46c2, 0.69d0) writes an event
//! channel's header in **12** bytes: its data's offset, a zero word and the
//! counts. 0.26g writes **16**: the same and a trailing pointer word the engine
//! fills on load. Nothing else differs: converted, the AK-12's shared clips
//! of 0.69d are 0.26g's byte for byte apart from the slots the engine
//! overwrites (resource-porter 9314977).
//!
//! The offsets are relative to the slot that holds them; the engine
//! overwrites the bone slots and the event pointers on load, so their values
//! do not decide what plays (patch 19's 0.29a3 clips keep the 12-byte
//! offsets unshifted and play). [`events_12_to_16`] shifts them so that every
//! slot still resolves to its curve, and [`engine_words_cleared`] compares two
//! clips with those words cleared.

use anyhow::{Context, Result, ensure};

/// `bi_spline_bone_animation_baked`: nine pointer slots of 8 bytes a bone.
pub const BONE_RECORD: usize = 0x48;
pub const EVENT_HEADER_LATER: usize = 12;
pub const EVENT_HEADER_026: usize = 16;

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).context("truncated u32")?.try_into()?,
    ))
}

/// A clip's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipHeader {
    pub bones: usize,
    pub events: usize,
    pub animation_type: u8,
}

pub fn clip_header(b: &[u8]) -> Result<ClipHeader> {
    ensure!(b.len() >= 4, "b-spline shorter than its header");
    Ok(ClipHeader {
        bones: usize::from(u16::from_le_bytes([b[0], b[1]])),
        events: usize::from(b[2]),
        animation_type: b[3],
    })
}

/// A later build's clip (12-byte event headers) in 0.26g's layout (16).
/// A clip without event channels is the same in both.
pub fn events_12_to_16(b: &[u8]) -> Result<Vec<u8>> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    let data = base + h.events * EVENT_HEADER_LATER;
    ensure!(b.len() >= data, "b-spline shorter than its records");
    if h.events == 0 {
        return Ok(b.to_vec());
    }
    let shift = (4 * h.events) as u32;
    let mut out = Vec::with_capacity(b.len() + shift as usize);
    out.extend_from_slice(&b[..4]);
    for slot in 0..h.bones * 9 {
        let at = 4 + slot * 8;
        let offset = u32_at(b, at)?;
        ensure!(
            at + offset as usize + 4 <= b.len(),
            "bone slot {slot} points outside the clip"
        );
        out.extend_from_slice(&(offset + shift).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    for event in 0..h.events {
        let at = base + event * EVENT_HEADER_LATER;
        let offset = u32_at(b, at)?;
        ensure!(
            u32_at(b, at + 4)? == 0,
            "event channel {event}'s header is not a 12-byte one"
        );
        ensure!(
            at + offset as usize <= b.len(),
            "event channel {event} points outside the clip"
        );
        // Header `event` moves by 4 * event, its data by 4 * events.
        out.extend_from_slice(&(offset + shift - 4 * event as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&b[at + 8..at + 12]);
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&b[data..]);
    for slot in 0..h.bones * 9 {
        let at = 4 + slot * 8;
        let old = at + u32_at(b, at)? as usize;
        let new = at + u32_at(&out, at)? as usize;
        let pairs = u32_at(b, old)? as usize;
        ensure!(
            b.get(old..old + 4 + pairs * 8) == out.get(new..new + 4 + pairs * 8),
            "bone slot {slot} lost its curve"
        );
    }
    Ok(out)
}

/// A 0.26g-layout clip with the words the engine overwrites on load set to
/// zero: each bone slot and each event header's offset and pointer words.
/// Two clips that play the same are equal after it.
pub fn engine_words_cleared(b: &[u8]) -> Result<Vec<u8>> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    let end = base + h.events * EVENT_HEADER_026;
    ensure!(b.len() >= end, "b-spline shorter than its records");
    let mut out = b.to_vec();
    out[4..base].fill(0);
    for event in 0..h.events {
        let at = base + event * EVENT_HEADER_026;
        out[at..at + 8].fill(0);
        out[at + 12..at + 16].fill(0);
    }
    Ok(out)
}

/// The names of a 0.26g-layout clip's event channels.
pub fn event_names(b: &[u8]) -> Result<Vec<String>> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    (0..h.events)
        .map(|event| {
            let header = base + event * EVENT_HEADER_026;
            let at = header + u32_at(b, header)? as usize;
            let knots = usize::from(*b.get(header + 8).context("truncated event header")?);
            let name = b.get(at + 5 * knots..).context("truncated event name")?;
            let end = name
                .iter()
                .position(|c| *c == 0)
                .context("unterminated event name")?;
            Ok(String::from_utf8(name[..end].to_vec())?)
        })
        .collect()
}
