//! Decode the spawn points a Shadow of Chernobyl multiplayer level declares.
//!
//! A converted map inherits no spawn points, and neither of the obvious
//! substitutes works. The centre of the map's bounding box is not ground -- for
//! `mp_pool` it is the bottom of the swimming pool. The widest flat area in the
//! collision mesh *is* ground, but these levels are carved out of larger
//! single-player maps: `testers_mp_darkvalley` has collision across
//! x = 84..480 while its arena is a fraction of that, so the widest floor can
//! sit outside the playable area entirely.
//!
//! The level already knows the answer. `level.spawn` carries the map's own
//! multiplayer team bases, placed by the people who built it.
//!
//! Only the object header is decoded here. A full `CSE_Abstract` carries
//! per-class packed properties this port has no use for, but every object
//! begins the same way, which is enough to find the ones that matter.

use anyhow::Result;

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
