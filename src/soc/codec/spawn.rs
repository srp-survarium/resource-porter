// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode the spawn points an X-Ray multiplayer level declares.
//!
//! A converted map inherits no spawn points, and neither of the obvious
//! substitutes works. The centre of the map's bounding box is not ground -- for
//! `mp_pool` it is the bottom of the swimming pool. The widest flat area in the
//! collision mesh *is* ground, but these levels are carved out of larger
//! single-player maps: `testers_mp_darkvalley` has collision across
//! x = 84..480 while its arena is a fraction of that, so the widest floor can
//! sit outside the playable area entirely.
//!
//! The level already knows the answer. Shadow of Chernobyl's `level.spawn`
//! carries the map's multiplayer team bases. Clear Sky's `level.game` carries
//! explicit actor respawn points, grouped by team and game-mode mask.
//!
//! Only the object header is decoded here. A full `CSE_Abstract` carries
//! per-class packed properties this port has no use for, but every object
//! begins the same way, which is enough to find the ones that matter.

use anyhow::{Result, bail};

use super::chunks::{Chunks, Cursor};

/// The section name of a multiplayer team base.
pub const TEAM_BASE_SECTION: &str = "zone_team_base";

/// One spawned object's header.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnPoint {
    /// Section (class) name, e.g. `zone_team_base`.
    pub section: String,
    /// Instance name, e.g. `level_prefix_zone_team_base_0000`.
    pub name: String,
    pub position: [f32; 3],
}

/// One actor respawn decoded from `level.game`'s `RPOINT_CHUNK`.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorRespawn {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub team: u8,
    /// Bit mask of X-Ray multiplayer game IDs.
    pub game_type: u16,
}

/// `POINT_BASE + ptRPoint` from `Common/LevelGameDef.h`.
const RPOINT_CHUNK: u32 = 0x2000;
const ACTOR_SPAWN: u8 = 0;

/// Every actor respawn in `level.game`, in authored file order.
///
/// Clear Sky appends one reserved byte to the documented 28-byte actor record;
/// the retail reader consumes the common prefix and ignores the remainder, so
/// this decoder deliberately does the same.
pub fn actor_respawns(game: &[u8]) -> Result<Vec<ActorRespawn>> {
    let Some(points) = Chunks::new(game, "level.game").find(RPOINT_CHUNK)? else {
        return Ok(Vec::new());
    };

    let mut found = Vec::new();
    for chunk in Chunks::new(points, "level.game rpoints").iter() {
        let chunk = chunk?;
        let mut record = Cursor::new(chunk.payload, "level.game rpoint");
        if record.remaining() < 28 {
            bail!(
                "level.game rpoint {} has {} bytes; expected at least 28",
                chunk.id,
                record.remaining()
            );
        }
        let position = record.float3()?;
        let rotation = record.float3()?;
        let team = record.u8()?;
        let point_type = record.u8()?;
        let game_type = record.u16()?;
        if point_type == ACTOR_SPAWN {
            if !position
                .iter()
                .chain(&rotation)
                .all(|value| value.is_finite())
            {
                bail!(
                    "level.game rpoint {} contains a non-finite vector",
                    chunk.id
                );
            }
            found.push(ActorRespawn {
                position,
                rotation,
                team,
                game_type,
            });
        }
    }
    Ok(found)
}

/// Every team base the level declares, in file order.
pub fn team_bases(spawn: &[u8]) -> Vec<SpawnPoint> {
    objects(spawn, TEAM_BASE_SECTION)
}

/// Every object of one section, in file order.
///
/// The file is scanned for the section name rather than walked as a chunk
/// tree: the object list is a packet stream whose per-class payloads this
/// module deliberately does not model, so there is no reliable way to step
/// from one record to the next. A match is accepted only if the bytes that
/// follow it parse as a header -- a printable name, then a finite position --
/// which rejects the occasional coincidental hit inside binary payload.
pub fn objects(spawn: &[u8], section: &str) -> Vec<SpawnPoint> {
    // CSE_Abstract::Spawn_Read, after the M_SPAWN marker:
    //   stringZ section, stringZ name, u8 gameid, u8 rp, vec3 position
    const GAME_ID_AND_RESPAWN_POINT: usize = 2;

    let needle: Vec<u8> = section
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .collect();

    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = find(&spawn[at..], &needle) {
        let start = at + offset;
        at = start + needle.len();

        let Some(end) = spawn[at..].iter().position(|byte| *byte == 0) else {
            break;
        };
        let Ok(name) = std::str::from_utf8(&spawn[at..at + end]) else {
            continue;
        };
        if !name.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            continue;
        }

        let position_at = at + end + 1 + GAME_ID_AND_RESPAWN_POINT;
        let Some(bytes) = spawn.get(position_at..position_at + 12) else {
            continue;
        };
        let mut position = [0.0_f32; 3];
        for (axis, chunk) in bytes.chunks_exact(4).enumerate() {
            position[axis] = f32::from_le_bytes(chunk.try_into().expect("four bytes"));
        }
        if !position.iter().all(|value| value.is_finite()) {
            continue;
        }

        found.push(SpawnPoint {
            section: section.to_owned(),
            name: name.to_owned(),
            position,
        });
    }
    found
}

/// Decode the team bases, or explain that the level declares none.
pub fn require_team_bases(spawn: &[u8], level: &str) -> Result<Vec<SpawnPoint>> {
    let bases = team_bases(spawn);
    if bases.is_empty() {
        anyhow::bail!("{level}: level.spawn declares no {TEAM_BASE_SECTION}");
    }
    Ok(bases)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(section: &str, name: &str, position: [f32; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(section.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&[7, 0]); // gameid, respawn point
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn rpoint(position: [f32; 3], rotation: [f32; 3], team: u8, kind: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in position.into_iter().chain(rotation) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[team, kind]);
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.push(0); // Clear Sky's reserved trailing byte.
        bytes
    }

    #[test]
    fn decodes_clear_sky_actor_respawns_and_skips_items() {
        let mut points = Vec::new();
        points.extend(chunk(0, &rpoint([32.927, -0.543, -32.941], [0.0; 3], 1, 0)));
        points.extend(chunk(1, &rpoint([2.167, 24.026, 3.957], [0.0; 3], 1, 2)));
        points.extend(chunk(2, &rpoint([4.089, -1.194, 39.012], [0.0; 3], 2, 0)));
        let game = chunk(RPOINT_CHUNK, &points);

        let respawns = actor_respawns(&game).unwrap();
        assert_eq!(respawns.len(), 2);
        assert_eq!(respawns[0].position, [32.927, -0.543, -32.941]);
        assert_eq!(respawns[0].team, 1);
        assert_eq!(respawns[0].game_type, 4);
        assert_eq!(respawns[1].position, [4.089, -1.194, 39.012]);
        assert_eq!(respawns[1].team, 2);
    }

    /// The two bases `testers_mp_darkvalley` declares, at the offsets and
    /// values decoded from the retail file.
    #[test]
    fn decodes_the_team_bases_of_a_multiplayer_level() {
        let mut spawn = vec![0xAB; 32];
        spawn.extend(record(
            TEAM_BASE_SECTION,
            "level_prefix_zone_team_base",
            [233.06, 1.88, -246.13],
        ));
        spawn.extend([0x11; 40]);
        spawn.extend(record(
            TEAM_BASE_SECTION,
            "level_prefix_zone_team_base_0000",
            [333.43, 1.79, -215.36],
        ));

        let bases = team_bases(&spawn);
        assert_eq!(bases.len(), 2);
        assert_eq!(bases[0].name, "level_prefix_zone_team_base");
        assert_eq!(bases[0].position, [233.06, 1.88, -246.13]);
        assert_eq!(bases[1].name, "level_prefix_zone_team_base_0000");
        assert_eq!(bases[1].position, [333.43, 1.79, -215.36]);
    }

    /// The section name appearing inside binary payload must not be mistaken
    /// for a record: the bytes after it do not parse as a header.
    #[test]
    fn rejects_a_coincidental_match_in_payload() {
        let mut spawn = Vec::new();
        spawn.extend_from_slice(TEAM_BASE_SECTION.as_bytes());
        spawn.push(0);
        spawn.extend([0x80, 0xFF, 0x01, 0x7F]); // not a printable name
        spawn.push(0);
        spawn.extend([0u8; 16]);
        assert!(team_bases(&spawn).is_empty());
    }

    #[test]
    fn reports_a_level_that_declares_none() {
        let error = require_team_bases(&[0u8; 64], "testers_mp_pool")
            .unwrap_err()
            .to_string();
        assert!(error.contains("declares no zone_team_base"), "{error}");
    }
}
