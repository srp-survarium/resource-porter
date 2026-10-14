//! Decode native checkpoint event counters; no client state is changed.
use survarium::v026g::match_msgs::snapshot::{self, event};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let body = std::fs::read(&path)?;
        let decoded = snapshot::decode(0, &body, 2, 2, 10).ok_or("invalid snapshot")?;
        let stats = decoded.statistics.ok_or("missing statistics")?;
        print!("{{\"file\":\"{}\",\"players\":[", path);
        for (i, p) in stats.players.iter().enumerate() {
            if i > 0 { print!(","); }
            print!("{{\"slot\":{i},\"deaths\":{},\"suicides\":{},\"artefacts\":{}}}", p.total(event::DEATH), p.total(event::SUICIDE), p.total(event::ARTEFACT));
        }
        println!("]}}");
    }
    Ok(())
}
