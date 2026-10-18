#!/usr/bin/env python3
"""Skhid revision 20's two UI fixes over the released movies, and nothing else.

Usage: ui_rev20_swf.py HUD_SWF LOBBY_MENU_SWF OUT_DIR

- OUT_DIR/hud.swf: HUD_SWF (the released one: patch 14's) with
  exo_oxygen_hud_swf.py's edit: the Duty / Freedom exoskeletons' oxygen
  slot (ids 110 / 113) shows helmet 1800's / 1808's lobby icon, greyed
  when empty as the native tanks are. The icons are cut from LOBBY_MENU_SWF.
- OUT_DIR/lobby_menu.swf: LOBBY_MENU_SWF (the released one: patch 19's)
  with the Matches window's names of levels 9-13 (matches_levels(), as
  v011e_ui_swf.py on rev20-v011-maps-5 has it); no 0.11e loading picture.
Both fail on any byte they do not recognise.
"""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import abc_patch
import exo_oxygen_hud_swf as exo
from exo_oxygen_hud_swf import read, write, class_abc

# The Matches window (srp's lobby movie, BottomPanel.matchesNameTable()) names a match's map by its level
# (match_making_params.map_id) from a table of levels 1-8 in its five languages (English, Ukrainian,
# Russian, German, Polish); any other level shows as "#<level>". Levels 9-13 as the loading screen
# names them (st_level_<n>_name: the native 0.26 name, the port's source version in brackets).
# 12 and 13 (the v0.11 maps) are unused until those maps ship.
MATCHES_LEVELS = [
    ('9', ['London', 'Лондон', 'Лондон', 'London', 'Londyn']),
    ('10', ['Vostok Radar Station (v0.21)', 'РЛС "Восток" (v0.21)', 'РЛС "Восток" (v0.21)', 'RS Vostok (v0.21)', 'Stacja Radarowa "Vostok" (v0.21)']),
    ('11', ['Mamayev Kurgan (v0.21)', 'Мамаїв курган (v0.21)', 'Мамаев курган (v0.21)', 'Mamaew kurgan (v0.21)', 'Kurhan Mamaja (v0.21)']),
    ('12', ['Chemical Plant (v0.11)', 'Хімзавод (v0.11)', 'Химзавод (v0.11)', 'Chemiebetrieb (v0.11)', 'Zakłady chemiczne (v0.11)']),
    ('13', ['Vostok Radar Station (v0.11)', 'РЛС "Восток" (v0.11)', 'РЛС "Восток" (v0.11)', 'RS Vostok (v0.11)', 'Stacja Radarowa "Vostok" (v0.11)']),
]


def matches_levels(tags):
    """BottomPanel.matchesNameTable(): the `levels` object literal (`"8":[Cologne x5]` then `newobject 8`)
    gets MATCHES_LEVELS before its newobject, whose count becomes 13. One method body, constants appended."""
    U = abc_patch.u30
    i, prefix, abc = class_abc(tags, 'survarium/controls/BottomPanel')
    cologne = abc.find_string('Most w Kolonii')
    body = abc.body_with(lambda b: any(x[1] == 0x2c and x[2][0] == cologne for x in abc_patch.instructions(bytes(b['code']))))
    ins = abc_patch.instructions(bytes(body['code']))
    k = [j for j, x in enumerate(ins) if x[1] == 0x2c and x[2][0] == cologne]
    assert len(k) == 1 and ins[k[0] + 1][1] == 0x56 and ins[k[0] + 1][2] == [5], 'Cologne is not the table\'s last row'
    new = ins[k[0] + 2]
    assert new[1] == 0x55 and new[2] == [8], 'the levels table is not 8 rows (already patched?)'
    code = b''
    for key, names in MATCHES_LEVELS:
        assert len(names) == 5
        code += bytes([0x2c]) + U(abc.string(key))
        for n in names:
            code += bytes([0x2c]) + U(abc.string(n))
        code += bytes([0x56]) + U(5)
    abc.insert(body, new[0], code, 2 * len(MATCHES_LEVELS))
    at = new[0] + len(code)
    assert body['code'][at:at + 2] == bytes([0x55, 8])
    body['code'][at + 1] = 8 + len(MATCHES_LEVELS)
    tags[i][1] = prefix + abc.bytes()


def main(hud, lobby, out_dir):
    out = Path(out_dir); out.mkdir(parents=True, exist_ok=True)
    exo.main(hud, lobby, out / 'hud.swf')
    version, head, tags = read(lobby)
    matches_levels(tags)
    write(out / 'lobby_menu.swf', version, head, tags)
    print(f'lobby_menu.swf: Matches window levels {", ".join(k for k, _ in MATCHES_LEVELS)}')


if __name__ == '__main__':
    main(*sys.argv[1:])
