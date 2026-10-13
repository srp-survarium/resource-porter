#!/usr/bin/env python3
"""Launch the isolated native 0.26 Chemical Plant check after reserving the GPU."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('root', type=Path)
args = parser.parse_args()
root = args.root.resolve()
runtime = root / 'runtime'
prefix = runtime / 'wineprefix'
exe = runtime / 'client/binaries/x86/survarium-capture.exe'
assert hashlib.sha256(exe.read_bytes()).hexdigest() == '5eb0fc97b983ecc5d98282444076c7dc8180798429c58c4ac35849cb110c993a'
assert (root / 'resources.db').is_file()
wine = Path('/nix/store/kd22mbkmwsnvfyn1w14r6xcwpw9fnwzv-wine-wow64-staging-11.8/bin/wine')
def win(p):
    return 'Z:' + str(p).replace('/', '\\')
env = {k: v for k, v in os.environ.items() if not k.startswith('SRP_LEGACY_')}
env.update(WINEPREFIX=str(prefix), DISPLAY=':0', WINEDEBUG='-all',
           WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
           VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json',
           SRP_CAPTURE_BUILD='026g', SRP_CAPTURE_CONTROL=win(runtime/'camera.txt'),
           DXVK_FRAME_RATE='60')
env.pop('VK_DRIVER_FILES', None)
env.pop('SRP_CAPTURE_BUFFERS', None)
# A copied prefix has never been launched by this script. Refuse a second launch.
state = runtime / 'launch-state.json'
if state.exists():
    raise SystemExit('This runtime was already launched; stop it and use a new runtime directory.')
config = prefix / 'drive_c/users/sheep/Documents/Survarium/user.cfg'
lines = config.read_text().splitlines()
config.write_text('\n'.join(line for line in lines if line.split()[:1] != ['ui']) + '\nui off\n')
with (runtime/'server.log').open('w') as log:
    server = subprocess.Popen(['/home/sheep/Projects/survarium/srp/target/debug/match-server-026',
        '--bind','127.0.0.1:2253','--players','1','--start-players','1','--map','193',
        '--snapshots','--checkpoints','--recover','--clock-sync'],
        stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
with (runtime/'launch.log').open('w') as log:
    client = subprocess.Popen([str(wine),win(exe),'-localization=english','-splash_screen',
        '-server=127.0.0.1:2253','-mt_allocator=768','-hash_check_interval_in_ms=50'],
        env=env,cwd=exe.parent,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,
        start_new_session=True)
state.write_text(json.dumps({'server_pid':server.pid,'client_pid':client.pid,'prefix':str(prefix),
    'map':193,'ui':'off','shader_overrides':{},'resources':str(root/'resources.db')},indent=2)+'\n')
print(state.read_text())
