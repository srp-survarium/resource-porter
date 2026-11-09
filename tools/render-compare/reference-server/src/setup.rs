// Renderer-only 0.21d bootstrap; layout from srp-021d match_start.rs/lobby.rs.
pub fn local_test(players: u8, local_slot: u8) -> Result<Vec<u8>, &'static str> {
    if players != 1 || local_slot != 0 { return Err("one spectator required"); }
    let project = std::env::var("SRP_REFERENCE_PROJECT").unwrap_or("level_06/evening".into());
    if !["level_03/evening", "level_03/rain", "level_06/evening", "level_06/rain"].contains(&project.as_str()) { return Err("unreviewed project"); }
    let picture = match project.as_str() { "level_06/evening" => 35, "level_06/rain" => 36, "level_03/evening" => 8, "level_03/rain" => 9, _ => unreachable!() };
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
