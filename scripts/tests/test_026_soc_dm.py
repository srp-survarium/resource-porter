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

    def test_minimap_projection_follows_the_hud_setprops_math(self):
        mm = load('tools/playable-map-pack/render_minimap.py', 'render_minimap')
        # survarium.controls.Minimap with _centerPlayer: the engine passes
        # (x, -z); the bitmap pixel under the player is px - map_ox.
        def hud(x, z, size, pos_x, pos_y, w, h):
            cx, cy = w / (2 * size), h / (2 * size)
            map_ox, map_oy = (pos_x - size) * cx, (-pos_y - size) * cy
            return x * cx - map_ox, -z * cy - map_oy
        # Native level_03 entry (size 100, posX 14, posY -42.196, 512 px):
        # its centre is world (14, -42.196) and +z is up.
        self.assertEqual(mm.world_to_pixel(14, -42.196, 100, 14, -42.196, 512, 512), (256.0, 256.0))
        self.assertLess(mm.world_to_pixel(14, 0, 100, 14, -42.196, 512, 512)[1], 256)
        for x, z in [(-58.6, -36.5), (-100.2, -60.7), (-17.0, -12.3)]:
            for a, b in zip(mm.world_to_pixel(x, z, 73.143, -58.632, -36.539, 512, 512),
                            hud(x, z, 73.143, -58.632, -36.539, 512, 512)):
                self.assertAlmostEqual(a, b, places=6)
        # The play area is centred and fits the tab map (1:1 into 460 px).
        entry = mm.hud_entry((-100.26, -60.79, -17.01, -12.29))
        self.assertEqual((entry['posX'], entry['posY']), (-58.635, -36.54))
        for x, z in [(-100.26, -60.79), (-17.01, -12.29)]:
            u, v = mm.world_to_pixel(x, z, entry['size'], entry['posX'], entry['posY'], 512, 512)
            self.assertTrue(0 <= u <= 512 and 0 <= v <= 512)
        big = mm.hud_entry((-114.67, -258.08, 109.58, 50.92))
        self.assertLessEqual(308.997 * 512 / (2 * big['size']), mm.TAB_AREA + 1e-6)

    def test_own_hud_keys_and_names_are_new_and_unique(self):
        m = json.loads((ROOT/'ports/soc-cs-dm-to-026g.json').read_text())
        native = {'default', 'test', 'level_03_evn', 'level_03_rain'} | {f'level_0{i}' for i in range(1, 10)}
        keys = [x['hud_key'] for x in m['maps']]
        self.assertEqual(keys, [x['donor_project'] for x in m['maps']])
        self.assertFalse(set(keys) & native)
        self.assertEqual(len(set(keys)), len(keys))
        self.assertTrue(all(x['map_name'].strip() for x in m['maps']))
        tags = load('tools/playable-map-pack/hud_minimaps/swf_tags.py', 'swf_tags')
        import struct, zlib
        def swf(*tags_):
            body = b'\0' + b'\0\x18\x01\0'          # empty RECT, rate, 1 frame
            for code, payload in tags_:
                body += struct.pack('<HI', (code << 6) | 0x3f, len(payload)) + payload
            body += b'\0\0'
            return b'CWS\x0a' + struct.pack('<I', 8 + len(body)) + zlib.compress(body)
        a = swf((9, b'bg'), (82, b'\0\0\0\0survarium/controls/Minimap\0x'), (76, b'old'))
        b = swf((9, b'bg'), (35, b'img'), (82, b'\0\0\0\0survarium/controls/Minimap\0y'), (76, b'new'))
        missing, added = tags.compare(a, b)
        self.assertEqual(sorted(c for c, _ in missing), [76, 82])
        self.assertEqual(sorted(c for c, _ in added), [35, 76, 82])

    def test_loading_screens_follow_the_native_26_in_spec_order(self):
        ll = load('tools/playable-map-pack/lobby_loading.py', 'lobby_loading')
        m = json.loads((ROOT/'ports/soc-cs-dm-to-026g.json').read_text())
        ids = ll.loading_ids(m)
        self.assertEqual(ids, {197: 26, 198: 27, 199: 28, 200: 29, 201: 30})
        self.assertEqual(ll.loading_ids({'maps': [{'id': 5}, {'id': 6, 'loading_image': 'x.jpg'}]}), {6: 26})
        for item in m['maps']:
            self.assertTrue((ROOT/item['loading_image']).is_file(), item['loading_image'])
        import numpy as np
        from PIL import Image
        # Crop keeps the centre at the native 1920 x 798; the grade is
        # deterministic and lands on the native day-image statistics.
        rng = np.random.default_rng(1)
        src = Image.fromarray((rng.random((1080, 1920, 3)) ** 2 * 255).astype(np.uint8))
        img = ll.crop(src)
        self.assertEqual(img.size, (1920, 798))
        a, b = ll.grade(img), ll.grade(img)
        self.assertEqual(a.tobytes(), b.tobytes())
        lum = np.asarray(a, np.float32) / 255 @ ll.LUMA
        p5, p50, p95 = np.percentile(lum, [5, 50, 95])
        self.assertAlmostEqual(p50, ll.TARGET['p50'], delta=0.03)
        self.assertLess(p5, p50)
        self.assertLess(p95, ll.TARGET['white'])

if __name__ == '__main__':
    unittest.main()
