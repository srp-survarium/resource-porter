//! Read a native checkpoint body using the existing SRP snapshot decoder.
//! Usage: decode-battery-snapshot-json FILE TIME_MS PLAYERS BATTERIES
use survarium::v026g::match_msgs::snapshot::{self, Battery};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: FILE TIME_MS PLAYERS BATTERIES".into());
    }
    let time: u32 = args[2].parse()?;
    let players: usize = args[3].parse()?;
    let count: usize = args[4].parse()?;
    if !(1..=20).contains(&players) || !(1..=10).contains(&count) {
        return Err("invalid diagnostic counts".into());
    }
    let body = std::fs::read(&args[1])?;
    let decoded =
        snapshot::decode(time, &body, players, 2, count).ok_or("invalid native snapshot")?;
    let gather = decoded
        .rules
        .ok_or("no decoded rules")?
        .gather
        .ok_or("no gather rule")?;
    if gather.batteries.len() != count {
        return Err("battery count mismatch".into());
    }
    // The library exposes positions only. Locate the already-validated gather
    // payload and retain its native second transform for collision footprints.
    let mut bases = Vec::new();
    if gather
        .batteries
        .iter()
        .all(|b| matches!(b, Battery::Free { .. }))
    {
        let rules = decoded.sections.rules;
        for start in 0..rules.len().saturating_sub(8) {
            if rules[start..start + 4] != gather.seed.to_le_bytes()
                || rules[start + 4..start + 6] != gather.team_points
                || rules[start + 6..start + 8] != ((1u16 << count) - 1).to_le_bytes()
            {
                continue;
            }
            let mut at = start + 8;
            let mut candidate = Vec::new();
            for battery in &gather.batteries {
                let Some(record) = rules.get(at..at + 167) else {
                    break;
                };
                if record[4..6] != [255, 255] {
                    break;
                }
                let matrix: [f32; 16] = std::array::from_fn(|i| {
                    f32::from_le_bytes(record[70 + i * 4..74 + i * 4].try_into().unwrap())
                });
                let Battery::Free { position, .. } = battery else {
                    unreachable!()
                };
                if matrix[12..15] != *position || !matrix.iter().all(|v| v.is_finite()) {
                    break;
                }
                candidate.push([
                    [matrix[0], matrix[1], matrix[2]],
                    [matrix[4], matrix[5], matrix[6]],
                    [matrix[8], matrix[9], matrix[10]],
                ]);
                at += 167 + usize::from(record[166]);
            }
            if candidate.len() == count {
                bases.push(candidate);
            }
        }
        if bases.len() != 1 {
            return Err("ambiguous native battery transforms".into());
        }
    }
    print!(
        "{{\"time_ms\":{time},\"players\":{players},\"battery_count\":{count},\"seed\":{},\"team_points\":[{},{}],\"batteries\":[",
        gather.seed, gather.team_points[0], gather.team_points[1]
    );
    for (index, battery) in gather.batteries.iter().enumerate() {
        if index != 0 {
            print!(",");
        }
        match battery {
            Battery::Free {
                spotted_mask,
                position: [x, y, z],
            } => {
                if ![x, y, z].iter().all(|v| v.is_finite()) {
                    return Err("non-finite battery position".into());
                }
                print!(
                    "{{\"id\":{index},\"state\":\"free\",\"spotted_mask\":{spotted_mask},\"position\":[{x},{y},{z}]"
                );
                if let Some(all) = bases.first() {
                    print!(",\"body_basis\":{:?}", all[index]);
                }
                print!("}}");
            }
            Battery::Carried => print!("{{\"id\":{index},\"state\":\"carried\"}}"),
            Battery::InContainer(id) => {
                print!("{{\"id\":{index},\"state\":\"container\",\"container_id\":{id}}}")
            }
        }
    }
    println!("]}}");
    Ok(())
}
