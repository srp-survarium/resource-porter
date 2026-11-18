// Renderer-only 0.21d bootstrap; layout from srp-021d match_start.rs/lobby.rs.
pub fn local_test(players: u8, local_slot: u8) -> Result<Vec<u8>, &'static str> {
    if players != 1 || local_slot != 0 { return Err("one spectator required"); }
    let project = std::env::var("SRP_REFERENCE_PROJECT").unwrap_or("level_06/evening".into());
    let picture = picture(&project).ok_or("unreviewed project")?;
    let mut b = vec![picture];
    string(&mut b, &project);
    b.extend([2, 1, 5, 10]);
    b.extend(600u16.to_le_bytes()); b.extend(1u32.to_le_bytes());
    b.extend(0f32.to_le_bytes()); b.extend([5, 0, 10]); b.extend(0u16.to_le_bytes());
    for _ in 0..24 { b.extend(0f32.to_le_bytes()); }
    b.push(0); // squad
    b.extend([0, 0]); string(&mut b, "Reference");
    b.extend(10u32.to_le_bytes()); // no protection
    b.extend(0u32.to_le_bytes()); // empty equipment
    b.extend(0u32.to_le_bytes()); // no modifiers
    b.push(local_slot);
    Ok(b)
}
fn string(b: &mut Vec<u8>, s: &str) { b.push(s.len() as u8); b.extend(s.as_bytes()); }

/// The loading picture (`switch_to_level_loading`) of each 0.21d project, as
/// srp-021d's `match_start::MAPS` lists them (variants day, rain, evening).
fn picture(project: &str) -> Option<u8> {
    const MAPS: [(&str, [u8; 3]); 6] = [
        ("level_01", [0x01, 0x13, 0x14]), ("level_02", [0x03, 0x15, 0x16]),
        ("level_03", [0x05, 0x09, 0x08]), ("level_04", [0x0A, 0x17, 0x18]),
        ("level_05", [0x19, 0x1B, 0x1A]), ("level_06", [0x1D, 0x24, 0x23]),
    ];
    let (level, weather) = project.split_once('/')?;
    let variant = ["day", "rain", "evening"].iter().position(|w| *w == weather)?;
    MAPS.iter().find(|(l, _)| *l == level).map(|(_, p)| p[variant])
}
