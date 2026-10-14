#!/usr/bin/env python3
"""Prepare an isolated original 0.20e renderer reference without launching it."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess


def prepare(donor, package, output, server_source, hook):
    output.mkdir(parents=True, exist_ok=False)
    binary = output/'client/binaries/x86'
    binary.mkdir(parents=True)
    for name in ('resources', 'resources.db'):
        (output/'client'/name).symlink_to(donor/name)
    for name in ('bugtrap.dll', 'd3d11.dll', 'dxgi.dll'):
        shutil.copy2(package/'client/binaries/x86'/name, binary/name)
    shutil.copy2(hook, binary/'camera_hook.dll')
    script = Path(__file__).resolve().parents[1]/'render-compare/prepare_client.py'
    spec = importlib.util.spec_from_file_location('prepare_client', script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    digest = module.prepare(donor/'binaries/x86/survarium.exe', binary/'survarium-capture.exe', '020e')
    prefix = output/'wineprefix'
    subprocess.run(['tar', '-xf', str(package/'wineprefix.tar.gz'), '-C', str(output)], check=True)
    profile = prefix/'drive_c/users/sheep/Documents/Survarium/user.cfg'
    lines = profile.read_text().splitlines()
    lines = [line for line in lines if not line.startswith(('ui ', 'account_name ', 'account_digest '))]
    profile.write_text('\n'.join(lines+['ui on'])+'\n')
    server = output/'server-source'
    shutil.copytree(server_source, server, ignore=shutil.ignore_patterns('target', '.git'))
    # PDB-backed 020e layout differs from021:21*u16 score weights, no
    # match-level field, protection field, item modifications or trailing slot.
    # A spectator-only empty equipment profile avoids version-specific item IDs.
    (server/'src/setup.rs').write_text('''// Original020e renderer-only bootstrap; not a gameplay server.
// match_options::deserialize VA0x1e3c6f; profile VA0x1e3ace.
pub fn local_test(players: u8, local_slot: u8) -> Result<Vec<u8>, &'static str> {
    if players != 1 || local_slot != 0 { return Err("reference requires one spectator"); }
    let mut b = vec![22];
    string(&mut b, "level_03/day");
    b.extend([2, 1, 5, 10]);
    b.extend(600u16.to_le_bytes()); b.extend(1u32.to_le_bytes());
    b.extend(0f32.to_le_bytes()); b.extend([5, 0, 10]);
    for _ in 0..21 { b.extend(0u16.to_le_bytes()); }
    b.push(0); // squad
    b.extend([0, 1]); string(&mut b, "Reference");
    b.extend(0u32.to_le_bytes()); // no equipment slots
    b.extend(0u32.to_le_bytes()); // no profile modifiers
    Ok(b)
}
fn string(b: &mut Vec<u8>, s: &str) { b.push(s.len() as u8); b.extend(s.as_bytes()); }
''')
    main = server/'src/main.rs'
    text = main.read_text()
    old = '0x41 if b.len() == 4 && u32_at(b, 0) == Some(peer.session) => {'
    assert old in text
    # The original direct spectator uses session0, then ffffffff admission.
    new = ('0x41 if b.len() == 4 && (u32_at(b, 0) == Some(peer.session) || '
           '(peer.session == 0 && u32_at(b, 0) == Some(u32::MAX))) => {')
    main.write_text(text.replace(old, new))
    (output/'sessions.txt').write_text('1 0\n')
    report = {'scope': 'Renderer reference only; original resources are linked read-only by convention.',
              'source_game': str(donor.resolve()), 'exe_sha256': digest,
              'hook_sha256': hashlib.sha256(hook.read_bytes()).hexdigest(),
              'port': 2264, 'server_source': str(server_source.resolve()),
              'source_abi': {'camera_rva': '16312c', 'director_register': 'eax',
                             'match_options_va': '1e3c6f', 'profile_va': '1e3ace'}}
    (output/'prepare.json').write_text(json.dumps(report, indent=2)+'\n')
    return report


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('donor_game', type=Path); p.add_argument('private_package', type=Path)
    p.add_argument('output', type=Path); p.add_argument('server_source', type=Path)
    p.add_argument('hook', type=Path)
    a = p.parse_args()
    print(json.dumps(prepare(a.donor_game, a.private_package, a.output, a.server_source, a.hook), indent=2))
