#!/usr/bin/env python3
"""Stage a private 0.21d renderer, retaining the original resource archive."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from prepare_client import prepare


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--donor-game', required=True, type=Path)
    ap.add_argument('--dxvk-dir', required=True, type=Path)
    ap.add_argument('--bugtrap', required=True, type=Path)
    ap.add_argument('--stopped-prefix', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    a = ap.parse_args()
    tools = Path(__file__).resolve().parent
    a.output = a.output.resolve()
    prefix = a.stopped_prefix.resolve()
    if a.output.exists() or a.output.is_relative_to(prefix):
        ap.error('output must be new and outside the prefix')
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            entries = (proc/'environ').read_bytes().split(b'\0')
        except OSError:
            continue
        if ('WINEPREFIX='+str(prefix)).encode() in entries:
            ap.error('template Wine prefix is active')
    binary = a.output/'client/binaries/x86'
    binary.mkdir(parents=True)
    for name in ('resources', 'resources.db'):
        (a.output/'client'/name).symlink_to((a.donor_game/name).resolve(strict=True))
    for name in ('d3d11.dll', 'dxgi.dll'):
        shutil.copyfile(a.dxvk_dir/name, binary/name)
    shutil.copyfile(a.bugtrap, binary/'bugtrap.dll')
    shutil.copyfile(tools/'camera_hook.dll', binary/'camera_hook.dll')
    digest = prepare(a.donor_game/'binaries/x86/survarium.exe', binary/'survarium-capture.exe', '021d')
    shutil.copytree(prefix, a.output/'wineprefix', symlinks=True)
    shutil.copytree(tools/'reference-server', a.output/'server-source',
                    ignore=shutil.ignore_patterns('target'))
    target = a.output/'server-source/target/release'
    target.mkdir(parents=True)
    shutil.copyfile(tools/'reference-server/target/release/match-server-021d', target/'match-server-021d')
    (target/'match-server-021d').chmod(0o755)
    (a.output/'sessions.txt').write_text('1 0\n')
    (a.output/'prepare.json').write_text(json.dumps({
        'build': '021d', 'source_game': str(a.donor_game.resolve()),
        'exe_sha256': digest,
        'hook_sha256': hashlib.sha256((binary/'camera_hook.dll').read_bytes()).hexdigest(),
        'source_abi': {'camera_rva': '16e5c2', 'director_register': 'eax',
                       'inverse_view_offset': 4, 'projection_offset': 68},
    }, indent=2)+'\n')


if __name__ == '__main__':
    main()
