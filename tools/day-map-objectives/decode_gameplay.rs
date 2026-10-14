//! Summarise battery-mode gameplay state from native 0.26g checkpoints.
//! Usage: decode-gameplay PLAYERS BATTERIES CHECKPOINT.bin ...
//! Files are `extract_snapshots.py` output (`match026g_<peer>_<time>_battery.bin`).
//! Build against the SRP `survarium` crate (v026g snapshot decoder); see
//! docs/investigation/day-map-gameplay-20261014.md. Prints one JSON line per file.
use survarium::v026g::match_msgs::snapshot::{self, Battery};

const FIRST_PICKUP: usize = 19;
const PUT_IN_CONTAINER: usize = 20;
const SOLO_DELIVERY: usize = 24;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: decode-gameplay PLAYERS BATTERIES CHECKPOINT.bin ...";
    let players: usize = args.next().ok_or(usage)?.parse()?;
    let batteries: usize = args.next().ok_or(usage)?.parse()?;
    if !(1..=20).contains(&players) || !(1..=16).contains(&batteries) {
        return Err("invalid counts".into());
    }
    for path in args {
        let name = path.rsplit('/').next().unwrap_or(&path);
        let time: u32 = name.split('_').nth(2).ok_or("unnamed checkpoint")?.parse()?;
        let body = std::fs::read(&path)?;
        let decoded = snapshot::decode(time, &body, players, 2, batteries)
            .ok_or_else(|| format!("{path}: invalid native checkpoint"))?;
        let mut out = format!(
            "{{\"file\":\"{name}\",\"time_ms\":{time},\"crc\":{}",
            snapshot::crc(&body)
        );
        if let Some(rules) = &decoded.rules {
            out += &format!(
                ",\"state\":{},\"start\":{}",
                rules.timelimit.state,
                rules
                    .timelimit
                    .start
                    .map_or("null".into(), |s| s.to_string())
            );
            let kd: Vec<_> = rules
                .kills_deaths
                .iter()
                .map(|k| format!("[{},{},{}]", k.kills, k.deaths, k.online))
                .collect();
            out += &format!(",\"kills_deaths\":[{}]", kd.join(","));
            if let Some(respawn) = &rules.respawn {
                let times: Vec<_> = respawn
                    .times
                    .iter()
                    .map(|(slot, t)| format!("[{slot},{t}]"))
                    .collect();
                // Respawn point checker occupancy, in project config order.
                let occupied: Vec<_> = respawn
                    .points
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| **p != [0, 0])
                    .map(|(i, p)| format!("[{i},{},{}]", p[0], p[1]))
                    .collect();
                out += &format!(
                    ",\"respawn_points\":{},\"respawn_times\":[{}],\"occupied_respawn_points\":[{}]",
                    respawn.points.len(),
                    times.join(","),
                    occupied.join(",")
                );
            }
            if let Some(gather) = &rules.gather {
                let items: Vec<_> = gather
                    .batteries
                    .iter()
                    .map(|b| match b {
                        Battery::Carried => "{\"state\":\"carried\"}".to_string(),
                        Battery::InContainer(id) => {
                            format!("{{\"state\":\"container\",\"container_id\":{id}}}")
                        }
                        Battery::Free { position, .. } => format!(
                            "{{\"state\":\"free\",\"position\":[{},{},{}]}}",
                            position[0], position[1], position[2]
                        ),
                    })
                    .collect();
                let containers: Vec<_> = gather
                    .containers
                    .iter()
                    .map(|c| format!("{{\"users\":{:?},\"items\":{:?}}}", c.users, c.items))
                    .collect();
                out += &format!(
                    ",\"gather_team_points\":[{},{}],\"batteries\":[{}],\"containers\":[{}]",
                    gather.team_points[0],
                    gather.team_points[1],
                    items.join(","),
                    containers.join(",")
                );
            }
        }
        if let Some(statistics) = &decoded.statistics {
            let tail = statistics.tail;
            out += &format!(
                ",\"team_points\":[{},{}],\"team_victory_items\":[{},{}],\"winner_team\":{}",
                tail.team_points[0],
                tail.team_points[1],
                tail.team_victory_items[0],
                tail.team_victory_items[1],
                tail.winner_team
            );
            let events: Vec<_> = statistics
                .players
                .iter()
                .map(|p| {
                    format!(
                        "{{\"first_pickup\":{},\"put_in_container\":{},\"solo_delivery\":{}}}",
                        p.total(FIRST_PICKUP),
                        p.total(PUT_IN_CONTAINER),
                        p.total(SOLO_DELIVERY)
                    )
                })
                .collect();
            out += &format!(",\"player_events\":[{}]", events.join(","));
        }
        println!("{out}}}");
    }
    Ok(())
}
