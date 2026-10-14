#!/usr/bin/env python3
"""Launch the prepared private original020e renderer reference on port2264."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('reference', type=Path)
p.add_argument('--wine', required=True)
a = p.parse_args()
r = a.reference.resolve()
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
    probe.bind(('127.0.0.1', 2264))
def win(path): return 'Z:'+str(path).replace('/', '\\')
env = {k: v for k, v in os.environ.items() if not k.startswith(('SRP_LEGACY_', 'SRP_CAPTURE_'))}
env.update(WINEPREFIX=str(r/'wineprefix'), DISPLAY=':0', WINEDEBUG='-all',
           WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
           VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json',
           DXVK_FRAME_RATE='60', SRP_CAPTURE_BUILD='020e', SRP_CAPTURE_CONTROL=win(r/'camera.txt'))
env.pop('VK_DRIVER_FILES', None)
server_path = r/'server-source/target/release/match-server-021d'
with (r/'server.log').open('w') as log:
    server = subprocess.Popen([str(server_path), str(r/'sessions.txt'), '2264', '1'],
                              env=env, cwd=r, stdout=log, stderr=subprocess.STDOUT,
                              stdin=subprocess.DEVNULL, start_new_session=True)
with (r/'client.log').open('w') as log:
    client = subprocess.Popen([a.wine, win(r/'client/binaries/x86/survarium-capture.exe'),
                               '-client=127.0.0.1:2264', '-spectator', '-localization=english',
                               '-no_splash_screen', '-minimum_resources_memory_size=768'],
                              env=env, cwd=r/'client/binaries/x86', stdout=log,
                              stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                              start_new_session=True)
state = {'server_pid': server.pid, 'client_pid': client.pid, 'prefix': str(r/'wineprefix'),
         'port': 2264, 'scope': 'original020e renderer reference only'}
(r/'state.json').write_text(json.dumps(state, indent=2)+'\n')
print(json.dumps(state))
