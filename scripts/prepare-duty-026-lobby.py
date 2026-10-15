#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage a private local srp-026g lobby with additive converted equipment.

Copies sources and updates the embedded dictionary and item parameters. Every
new account in this isolated server starts with the main bundle equipped. Never
point this test server at an existing account database or public endpoint.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shutil


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--srp-root', type=Path, required=True)
    p.add_argument('--bundle', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--inventory-bundle', type=Path, action='append', default=[],
                   help='Already installed equipment to grant without equipping')
    a = p.parse_args()
    root, out = a.srp_root.resolve(), a.output.resolve()
    if out.exists():
        p.error('--output must be new')
    cfg_script = root / 'scripts/gen-data-026g/bincfg.py'
    spec = importlib.util.spec_from_file_location('bincfg', cfg_script)
    bc = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(bc)
    dictionary = a.bundle / 'overlay/gameplay/db_static_dictionaries'
    items = bc.load(dictionary)['items_dict']
    metadata = json.loads((a.bundle / 'equipment.json').read_text())
    for bundle in a.inventory_bundle:
        metadata.extend(dict(entry, equipped=False) for entry in
                        json.loads((bundle / 'equipment.json').read_text()))
    assert len({e['dict_id'] for e in metadata}) == len(metadata), 'duplicate equipment IDs'
    params_path = Path('crates/survarium/src/v026g/item_params/table.rs')
    params = (root / params_path).read_text()
    additions = []
    for entry in metadata:
        item = next(i for i in items if i['dict_id'] == entry['dict_id'])
        category = item['cfg_name'].split('/')[-2]
        template_path = entry.get('template', f'gameplay/items/armour/{category}/renesanse_{category}_6.options')
        template = next(i for i in items if i['cfg_name'] == template_path)
        pattern = rf'^    ItemParams \{{ dict_id: {template["dict_id"]},.*$'
        rows = re.findall(pattern, params, re.M)
        if len(rows) != 1:
            p.error(f'unrecognized item parameters for {template["dict_id"]}')
        row = rows[0].replace(f'dict_id: {template["dict_id"]},', f'dict_id: {entry["dict_id"]},', 1)
        if 'weight' in entry:
            row, count = re.subn(r'(?<!\w)weight: [^,]+,', f'weight: {float(entry["weight"])},', row)
            assert count == 1
        for field in ('item_level', 'match_making_level'):
            if field in entry:
                row, count = re.subn(rf'(?<!\w){field}: \d+,', f'{field}: {int(entry[field])},', row)
                assert count == 1
        if 'default_modifications' in entry:
            mods = ', '.join(str(v) for v in entry['default_modifications'])
            row, count = re.subn(r'default_modifications: &\[[^]]*\]', f'default_modifications: &[{mods}]', row)
            assert count == 1
        additions.append(row)
    pos = params.rfind('];')
    if pos < 0:
        p.error('unrecognized item parameter table')
    params = params[:pos] + '\n'.join(additions) + '\n' + params[pos:]
    account_path = Path('crates/session/src/v026g/account.rs')
    account = (root / account_path).read_text()
    marker = '        account\n    }\n\n    /// A new account whose first kit'
    if account.count(marker) != 1:
        p.error('SRP account constructor changed')
    equipped = [e for e in metadata if e.get('equipped', True)]
    equipment = ', '.join(f'({e["slot"]}usize, {e["dict_id"]}u16)' for e in equipped)
    covered = sorted({slot for e in metadata for slot in e.get('additional_slots', [])})
    clear_slots = ''.join(f'        account.kits[0].slots[{slot}] = None;\n' for slot in covered)
    grant = f'''        // Local Duty preview only; this workspace has its own account store.
        // Enter the normal lobby state: disabling tutorial messages leaves
        // the native client with all equipment controls disabled.
        account.tutorial_done = true;
        account.extras.tutorial_flags |= super::tutorial::flag::SKIPPED;
        account.premium_money = 5_000;
        for item in &dictionary.items {{
            if (item.cfg_name.starts_with("gameplay/items/armour/")
                || item.cfg_name.starts_with("gameplay/items/artefacts/")
                || item.category == 26)
                && ![{', '.join(str(e['dict_id']) for e in equipped)}].contains(&item.dict_id)
            {{
                account.add_item(item.dict_id, if item.is_stack {{ 1 }} else {{ FULL_CONDITION }});
            }}
        }}
{clear_slots}        for (slot, dict_id) in [{equipment}] {{
            let item_id = account.add_item(dict_id, FULL_CONDITION);
            account.kits[0].slots[slot] = Some(KitSlot {{ item_id, count: 1 }});
        }}
'''
    account = account.replace(marker, grant + marker)
    shutil.copytree(root, out, ignore=shutil.ignore_patterns('target', '.git', '.work', '__pycache__', 'data', 'logs'))
    (out / params_path).write_text(params)
    (out / account_path).write_text(account)
    # Shop ranks that differ from srp's item-level rule, as (seller, rank).
    prices_path = Path('crates/survarium/src/v026g/prices.rs')
    prices = (root / prices_path).read_text()
    prices_anchor = '        // --- Weapons and armour ---\n'
    if prices.count(prices_anchor) != 1:
        p.error('SRP price exceptions changed')
    sold = ''
    for entry in metadata:
        if 'sellers' in entry:
            item = next(i for i in items if i['dict_id'] == entry['dict_id'])
            sellers = ', '.join(f'({int(s)}, {int(level)})' for s, level in entry['sellers'])
            sold += f'        ("{item["name"]}", &[{sellers}]),\n'
    (out / prices_path).write_text(prices.replace(prices_anchor, prices_anchor + sold))
    # A compound outfit owns its additional armor slots. The Flash paperdoll
    # disables those cells; enforce the same rule on inventory relocations.
    rules_path = Path('crates/session/src/v026g/rules.rs')
    rules = (root / rules_path).read_text()
    anchor = '            let count = if is_stack {'
    if rules.count(anchor) != 1:
        p.error('SRP relocation rules changed')
    linked_rules = ''
    for entry in metadata:
        extra = entry.get('additional_slots', [])
        if not extra:
            continue
        slots = ', '.join(str(v) + 'usize' for v in extra)
        linked_rules += f'''            if owned.dict_id == {entry['dict_id']} {{
                for slot in [{slots}] {{ slots[slot] = None; }}
            }} else if [{slots}].contains(&target)
                && slots[{entry['slot']}].and_then(|s| account.item(s.item_id))
                    .is_some_and(|item| item.dict_id == {entry['dict_id']})
            {{
                slots[{entry['slot']}] = None;
            }}
'''
    rules = rules.replace(anchor, linked_rules + anchor)
    for entry in metadata:
        extra = entry.get('additional_slots', [])
        if not extra:
            continue
        test_anchor = '    #[test]\n    fn an_item_moves_to_the_inventory_and_back()'
        assert rules.count(test_anchor) == 1
        tests = f'''    #[test]
    fn {'duty_compound_suit_moves_as_one_item' if entry['dict_id'] == 1805 else 'compound_item_' + str(entry['dict_id']) + '_moves_as_one_item'}() {{
        let dictionary = Dictionary::shared();
        for slot in [{', '.join(str(v) + 'u8' for v in extra)}] {{
            let mut account = account();
            let suit = account.inventory.iter().find(|i| i.dict_id == {entry['dict_id']}).unwrap().clone();
            if !account.kits[0].slots[{entry['slot']}].is_some_and(|s| s.item_id == suit.item_id) {{
                move_items(&mut account, 1, &[relocation(&suit, INVENTORY_SLOT, {entry['slot']}, 1)], NOW).unwrap();
            }}
            let weight = account.kit_weight(&account.kits[0]);
            let stock = account.inventory.iter().find(|i| i.dict_id != suit.dict_id && dictionary.fits(i.dict_id, slot)).unwrap().clone();
            move_items(&mut account, 1, &[relocation(&stock, INVENTORY_SLOT, slot, 1)], NOW).unwrap();
            assert!(account.kits[0].slots[{entry['slot']}].is_none());
            assert_eq!(account.kits[0].slots[usize::from(slot)].unwrap().item_id, stock.item_id);
            move_items(&mut account, 1, &[relocation(&suit, INVENTORY_SLOT, {entry['slot']}, 1)], NOW).unwrap();
            for covered in [{', '.join(str(v) + 'usize' for v in extra)}] {{ assert!(account.kits[0].slots[covered].is_none()); }}
            assert!((account.kit_weight(&account.kits[0]) - weight).abs() < 0.001);
            move_items(&mut account, 1, &[relocation(&suit, {entry['slot']}, INVENTORY_SLOT, 1)], NOW).unwrap();
            for covered in [{', '.join(str(v) + 'usize' for v in [entry['slot'], *extra])}] {{ assert!(account.kits[0].slots[covered].is_none()); }}
        }}
    }}

'''
        rules = rules.replace(test_anchor, tests + test_anchor)
    (out / rules_path).write_text(rules)
    shutil.copyfile(dictionary, out / 'resources/binary-config-examples/db_static_dictionaries_026g')
    for bundle in [*a.inventory_bundle, a.bundle]:
        modifiers = bundle / 'overlay/gameplay/items/modifications/modifications.options'
        if modifiers.exists():
            shutil.copyfile(modifiers, out / 'resources/binary-config-examples/modifications_026g.options')
    (out / 'duty-staging.json').write_text(json.dumps({
        'srp_source': str(root), 'bundle': str(a.bundle.resolve()),
        'dictionary_sha256': hashlib.sha256(dictionary.read_bytes()).hexdigest(),
        'equipment': metadata,
    }, indent=2) + '\n')
    print(out / 'Cargo.toml')


if __name__ == '__main__':
    main()
