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

/// A 0.26g-layout clip `b` with `clip`'s bone curves: the header's type and
/// its event channels (headers and data) as `b` has them, each bone slot and
/// event header pointing at its curve and data again. `clip` drives as many
/// bones as `b`, in `b`'s order (its `.bones_names` stays).
pub fn with_curves(b: &[u8], clip: &crate::skeleton_animation::Clip) -> Result<Vec<u8>> {
    let h = clip_header(b)?;
    ensure!(
        clip.bones.len() == h.bones,
        "the clip drives {} bones, its curves {}",
        h.bones,
        clip.bones.len()
    );
    let bones = clip.bones.len();
    let old_data = 4 + h.bones * BONE_RECORD + h.events * EVENT_HEADER_026;
    // The old curves end where the first event channel's data starts.
    let mut events_at = b.len();
    for event in 0..h.events {
        let header = 4 + h.bones * BONE_RECORD + event * EVENT_HEADER_026;
        events_at = events_at.min(header + u32_at(b, header)? as usize);
    }
    ensure!(
        events_at >= old_data,
        "an event channel's data is inside the curves"
    );
    let data = 4 + bones * BONE_RECORD + h.events * EVENT_HEADER_026;
    let mut curves = vec![];
    let mut starts = vec![];
    for track in &clip.tracks {
        for channel in &track.channels {
            ensure!(
                channel.knots.len() == channel.points.len() && channel.knots.len() >= 5,
                "a curve of {} knots and {} points",
                channel.knots.len(),
                channel.points.len()
            );
            starts.push(data + curves.len());
            curves.extend_from_slice(&u32::try_from(channel.knots.len())?.to_le_bytes());
            for (knot, point) in channel.knots.iter().zip(&channel.points) {
                curves.extend_from_slice(&knot.to_le_bytes());
                curves.extend_from_slice(&point.to_le_bytes());
            }
        }
    }
    let mut out = Vec::with_capacity(data + curves.len() + b.len() - events_at);
    out.extend_from_slice(&u16::try_from(bones)?.to_le_bytes());
    out.extend_from_slice(&b[2..4]);
    for (slot, start) in starts.iter().enumerate() {
        let at = 4 + slot * 8;
        out.extend_from_slice(&u32::try_from(start - at)?.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    let new_events_at = data + curves.len();
    for event in 0..h.events {
        let old = 4 + h.bones * BONE_RECORD + event * EVENT_HEADER_026;
        let new = 4 + bones * BONE_RECORD + event * EVENT_HEADER_026;
        let target = old + u32_at(b, old)? as usize - events_at + new_events_at;
        out.extend_from_slice(&u32::try_from(target - new)?.to_le_bytes());
        out.extend_from_slice(&b[old + 4..old + EVENT_HEADER_026]);
    }
    out.extend_from_slice(&curves);
    out.extend_from_slice(&b[events_at..]);
    ensure!(
        event_names(&out)? == event_names(b)?,
        "the clip lost its event channels"
    );
    Ok(out)
}

/// A 0.26g-layout clip's event channel `name`: its (knot, value) pairs, or
/// `None` when the clip has no such channel.
pub fn event_channel(b: &[u8], name: &str) -> Result<Option<Vec<(f32, u8)>>> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    for (event, found) in event_names(b)?.iter().enumerate() {
        if found != name {
            continue;
        }
        let header = base + event * EVENT_HEADER_026;
        let at = header + u32_at(b, header)? as usize;
        let knots = usize::from(*b.get(header + 8).context("truncated event header")?);
        let domains = usize::from(*b.get(header + 10).context("truncated event header")?);
        ensure!(
            knots == domains,
            "{name}: {knots} knots and {domains} values"
        );
        return Ok(Some(
            (0..knots)
                .map(|k| -> Result<(f32, u8)> {
                    let t = f32::from_le_bytes(
                        b.get(at + 4 * k..at + 4 * k + 4)
                            .context("truncated event knots")?
                            .try_into()?,
                    );
                    Ok((
                        t,
                        *b.get(at + 4 * knots + k)
                            .context("truncated event values")?,
                    ))
                })
                .collect::<Result<_>>()?,
        ));
    }
    Ok(None)
}
