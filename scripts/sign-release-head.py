#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Sign the file table of a `pack release` head region with the Skhid key.

Used as `pack release --sign-command "python3 scripts/sign-release-head.py
--signer <skhid-srp>/srp-026g/client-patches/sign_resources_026g.py"`; the
release appends `<head> <signature offset>`. The private key is read from
its default location (or --key) and never copied anywhere.
"""
import argparse
import importlib.util
from pathlib import Path


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--signer', type=Path, required=True, help='sign_resources_026g.py')
    ap.add_argument('--key', type=Path, help='private key (default: the signer\'s default)')
    ap.add_argument('head', type=Path)
    ap.add_argument('offset', type=int)
    a = ap.parse_args()
    spec = importlib.util.spec_from_file_location('sign_resources_026g', a.signer)
    S = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(S)
    with open(a.head, 'rb') as handle:
        _, table, _ = S.read_table(handle)
    start = 24 + len(table)
    if start != a.offset:
        raise SystemExit(f'signature slot is at {start}, the release expects {a.offset}')
    S.sign_file(str(a.head), str(a.key or S.DEFAULT_KEY))
    with open(a.head, 'rb') as handle:
        _, table, signature = S.read_table(handle)
    end = start + len(signature)
    if S.which_key(table, signature) != 'skhid':
        raise SystemExit('the signed table does not verify with the Skhid key')
    # A changed table must not verify.
    changed = bytearray(table)
    changed[len(changed) // 2] ^= 0x01
    if S.which_key(bytes(changed), signature) is not None:
        raise SystemExit('a changed table still verifies')
    print(f'signed {a.head} [{start}, {end}) with the Skhid key')


if __name__ == '__main__':
    main()
