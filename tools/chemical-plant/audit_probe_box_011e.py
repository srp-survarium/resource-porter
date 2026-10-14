#!/usr/bin/env python3
"""Verify the cubic box-falloff block across all frozen 011e permutations."""
import argparse
import hashlib
import json
from pathlib import Path
from audit_probe_sphere_011e import instructions, bits


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    rows = json.loads((args.evidence/'probe-extracted.json').read_text())
    results = []
    for row in rows:
        if row['key'].replace('_', '')[-1] != '1':
            continue
        code = (args.evidence/(row['name']+'.dxbc')).read_bytes()
        assert hashlib.sha256(code).hexdigest() == row['sha256']
        ins = instructions(code)
        matches = []
        for i in range(len(ins)-9):
            block = ins[i:i+10]
            if [v[1] for v in block] != [0, 50, 14, 56, 50, 56, 50, 52, 56, 56]:
                continue
            assert block[3][2][0] & (1 << 13), 'falloff fraction must saturate'
            assert bits(-2) in block[4][2] and bits(3) in block[4][2]
            assert bits(1) in block[6][2] and bits(0) in block[7][2]
            matches.append({'first_word': block[0][0], 'last_word': block[-1][0]})
        assert len(matches) == 1, (row['name'], matches)
        results.append({'shader': row, 'block': matches[0]})
    assert len(results) == 8
    report = {'verified_unique_box_variants': results,
              'equation': 'q=world_to_outer_probe*world_position; t=saturate((abs(q)*outer-inner)/(outer-inner)); alpha=product(max(1-t*t*(3-2*t),0))',
              'scope': 'PS attenuation only; CPU bounds, mask, AO and final image are separate contracts.'}
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print('Verified eight distinct source011e box-falloff permutations.')


if __name__ == '__main__':
    main()
