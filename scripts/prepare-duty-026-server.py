#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage an isolated SRP direct-match server that equips the complete Duty exoskeleton.

The upstream checkout is read only. Build the staged workspace with Cargo;
SRP_026_DUTY=1 enables the equipment for this local test server only.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil

PATCH = '''
        // Local additive Duty test equipment, built by resource-porter.
        if std::env::var("SRP_026_DUTY").as_deref() == Ok("1") {
            for player in &mut roster {
                for slot in 0..7 { player.profiles[0].slots[slot] = None; }
                for (slot, dictionary_id) in [(3usize, 1805u16)] {
                    let inventory_index = u16::try_from(player.inventory.len())?;
                    player.inventory.push(InventoryItem {
                        id: u32::from(inventory_index) + 1,
                        dictionary_id,
                        value: ItemValue::NonStackable { value: 10000, field_0c: 0, field_10: 0 },
                        switches: vec![], attributes: vec![],
                    });
                    player.profiles[0].slots[slot] = Some(Slot { inventory_index, amount: 1 });
                }
            }
        }
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--srp-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root, out = args.srp_root.resolve(), args.output.resolve()
    if out.exists():
        parser.error('--output must be new')
    members = ['crates/survarium', 'crates/vostok', 'servers/match-server-026']
    server_path = Path('servers/match-server-026/src/server.rs')
    original = (root / server_path).read_text()
    if original.count('let roster = (0..players)') != 1 or original.count('        let options = MatchOptions {') != 1:
        parser.error('SRP server constructor changed; review the equipment insertion')
    source = original.replace('let roster = (0..players)', 'let mut roster: Vec<Player> = (0..players)')
    source = source.replace('        let options = MatchOptions {', PATCH + '        let options = MatchOptions {', 1)
    cargo = (root / 'Cargo.toml').read_text()
    cargo, count = re.subn(r'members = \[.*?\]', 'members = ' + json.dumps(members), cargo, count=1, flags=re.S)
    if count != 1:
        parser.error('unrecognized SRP workspace manifest')
    out.mkdir(parents=True)
    for member in members:
        shutil.copytree(root / member, out / member)
    (out / 'Cargo.toml').write_text(cargo)
    shutil.copyfile(root / 'Cargo.lock', out / 'Cargo.lock')
    (out / server_path).write_text(source)
    (out / 'duty-staging.json').write_text(json.dumps({
        'srp_source': str(root),
        'server_source_sha256': hashlib.sha256(original.encode()).hexdigest(),
        'equipment': {'3': 1805},
        'enable': 'SRP_026_DUTY=1',
    }, indent=2) + '\n')
    print(out / 'Cargo.toml')


if __name__ == '__main__':
    main()
