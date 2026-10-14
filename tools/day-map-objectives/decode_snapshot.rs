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
                    "{{\"id\":{index},\"state\":\"free\",\"spotted_mask\":{spotted_mask},\"position\":[{x},{y},{z}]}}"
                );
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
