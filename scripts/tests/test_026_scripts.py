# SPDX-License-Identifier: GPL-3.0-or-later
"""Asset-free checks for build failures and launcher process ownership."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]


def load_launcher():
    spec = importlib.util.spec_from_file_location('run_026', REPO / 'scripts/run-026-map.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='porter 026 ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / 'target.db'
        self.donor = self.root / 'donor.db'
        self.target.write_bytes(b'target')
        self.donor.write_bytes(b'donor')
        self.output = self.root / 'release'
        self.log = self.root / 'commands.jsonl'
        self.porter = self.root / 'fake porter'
        self.porter.write_text(f'#!{sys.executable}\n' + '''
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ['PORTER_TEST_LOG'], 'a') as log:
    log.write(json.dumps(args) + '\\n')
if args[0] == os.environ.get('PORTER_TEST_FAIL'):
    sys.exit(7)
if args[0].startswith('build-'):
    pathlib.Path(args[args.index('--output')+1]).mkdir(parents=True)
if args[0] == 'apply':
    pathlib.Path(args[args.index('--output-db')+1]).write_bytes(b'packed')
if args[0] == 'verify':
    assert pathlib.Path(args[args.index('--db')+1]).read_bytes() == b'packed'
''')
        self.porter.chmod(0o755)

    def run_build(self, profile, fail='', donor=True):
        command = [sys.executable, str(REPO / 'scripts/build-026-maps.py'), profile,
                   '--target-db', str(self.target), '--output', str(self.output),
                   '--porter', str(self.porter)]
        if donor:
            command += ['--donor-db', str(self.donor)]
        return subprocess.run(command, env=dict(os.environ, PORTER_TEST_LOG=str(self.log),
                              PORTER_TEST_FAIL=fail), capture_output=True, text=True)

    def commands(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_profiles_build_pack_and_verify_with_paths_containing_spaces(self):
        for profile, builder in [('fog-school', 'build-survarium026-fog'),
                                 ('021-maps', 'build-survarium021-maps')]:
            with self.subTest(profile=profile):
                self.output = self.root / profile
                self.log.unlink(missing_ok=True)
                result = self.run_build(profile)
                self.assertEqual(result.returncode, 0, result.stderr)
                commands = self.commands()
                self.assertEqual([c[0] for c in commands], [builder, 'apply', 'verify'])
                self.assertIn(str(self.target), commands[0])
                self.assertEqual(self.target.read_bytes(), b'target')
                self.assertEqual('--donor-db' in commands[0], profile == '021-maps')

    def test_failure_stops_before_later_steps(self):
        for fail, count in [('build-survarium026-fog', 1), ('apply', 2), ('verify', 3)]:
            with self.subTest(fail=fail):
                self.output = self.root / fail
                self.log.unlink(missing_ok=True)
                result = self.run_build('fog-school', fail)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(self.commands()), count)
                self.assertNotIn('Playable database:', result.stdout)

    def test_missing_donor_and_existing_output_are_rejected_before_build(self):
        self.assertNotEqual(self.run_build('021-maps', donor=False).returncode, 0)
        self.assertFalse(self.log.exists())
        self.output.mkdir()
        marker = self.output / 'keep'
        marker.write_text('previous release')
        self.assertNotEqual(self.run_build('fog-school').returncode, 0)
        self.assertFalse(self.log.exists())
        self.assertEqual(marker.read_text(), 'previous release')


class LauncherTests(unittest.TestCase):
    def test_cleanup_stops_only_owned_server_and_prefixes(self):
        launcher = load_launcher()
        owned = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        try:
            prefixes = [Path('/tmp/porter-test-prefix-1'), Path('/tmp/porter-test-prefix-2')]
            with patch.object(launcher.subprocess, 'run') as wine:
                launcher.stop(owned, prefixes, '/test/wineserver')
            self.assertIsNotNone(owned.poll())
            self.assertIsNone(unrelated.poll())
            self.assertEqual([call.args[0] for call in wine.call_args_list],
                             [['/test/wineserver', flag] for _ in prefixes for flag in ('-k', '-w')])
            self.assertEqual([call.kwargs['env']['WINEPREFIX'] for call in wine.call_args_list],
                             [str(prefix) for prefix in prefixes for _ in range(2)])
        finally:
            for process in [owned, unrelated]:
                if process.poll() is None:
                    process.terminate()
                process.wait(timeout=5)

    def test_failed_start_does_not_stop_unstarted_prefixes(self):
        launcher = load_launcher()
        with patch.object(launcher.subprocess, 'run') as wine:
            launcher.stop(None, [], '/test/wineserver')
        wine.assert_not_called()


if __name__ == '__main__':
    unittest.main()
