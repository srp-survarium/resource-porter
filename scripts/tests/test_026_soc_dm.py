# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline checks of the X-Ray deathmatch package tooling."""
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, ROOT/path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SocDeathmatchTooling(unittest.TestCase):
    def test_new_materials_are_proven_against_their_native_templates(self):
        verify = load('tools/render-compare/verify_shader_variants.py', 'verify_shader_variants')
        manifest = {'donor_build': '0.10b-802', 'converted_materials': {
            'material_instances/ported/soc/mp_pool/tile.material': 'material_instances/tiled/floor/pol.material',
            'material_instances/flora/elka.material': 'material_instances/flora/elka.material'}}
        self.assertEqual(verify.template_paths(manifest), {
            'material_instances/ported/v010b/ported/soc/mp_pool/tile.material': 'material_instances/tiled/floor/pol.material',
            'material_instances/ported/v010b/flora/elka.material': 'material_instances/flora/elka.material'})

    def test_deathmatch_presets_carry_only_the_hook_and_shared_config(self):
        presets = load('tools/playable-map-pack/build_day_presets.py', 'build_day_presets')
        self.assertEqual(set(presets.NATIVE_KEYS), {'pool-soc-dm', 'training-camp-cs-dm', 'rembasa-cs-dm',
                                                    'sport-center-cs-dm', 'firestation-cs-dm'})
        self.assertTrue(set(presets.NATIVE_KEYS.values()) <= set(presets.DAY_KEYS))
        self.assertFalse(set(presets.NATIVE_KEYS) & set(presets.WEATHER) & set(presets.SSAO_PARAMETERS))

    def test_port_manifest_registers_five_deathmatch_maps_after_196(self):
        m = json.loads((ROOT/'ports/soc-cs-dm-to-026g.json').read_text())
        self.assertEqual([x['id'] for x in m['maps']], [197, 198, 199, 200, 201])
        projects = {x['id']: x['donor_project'] for x in m['maps']}
        self.assertEqual(projects, {197: 'soc_mp_pool', 198: 'cs_training_camp', 199: 'cs_mp_rembasa',
                                    200: 'cs_mp_sport_center', 201: 'cs_mp_firestation'})
        for item in m['maps']:
            self.assertEqual(item['mode'], 4)
            self.assertTrue(item['height_ambient'])
            self.assertTrue(item['deathmatch']['respawns'])
            self.assertTrue(all(len(p) == 4 for p in item['deathmatch']['respawns']))
            for ramp in item.get('walker_ramps', []):
                rise = ramp['to'][1]-ramp['from'][1]
                run = ((ramp['to'][0]-ramp['from'][0])**2+(ramp['to'][2]-ramp['from'][2])**2)**.5
                self.assertGreater(rise, 0)
                self.assertLessEqual(rise/run, 0.84, ramp['name'])  # at most 40 degrees, as the converter enforces
        pool = m['maps'][0]
        self.assertEqual(len(pool['deathmatch']['respawns']), 35)
        # The plank ramps and covers asked for on mp_pool and the sports center.
        self.assertTrue(all(r['planks'] for r in pool['walker_ramps']))
        self.assertTrue(all(r['planks'] for r in m['maps'][3]['walker_ramps']))
        for ramp in pool['walker_ramps']:
            rise = ramp['to'][1]-ramp['from'][1]
            run = ((ramp['to'][0]-ramp['from'][0])**2+(ramp['to'][2]-ramp['from'][2])**2)**.5
            self.assertLessEqual(rise/run, 0.578, ramp['name'])  # 30 degrees

if __name__ == '__main__':
    unittest.main()
