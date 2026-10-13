#!/usr/bin/env python3
"""Separate wall albedo, ambient lighting and specular composition residuals."""
import argparse
import json
import math
from pathlib import Path
import struct

from audit_full_diffuse_composition import unpack
from audit_translucency_auxiliary import rgb

REGIONS = {'yard-south': [350, 260, 450, 430],
           'factory-interior': [980, 180, 1050, 320]}


def add(stats, name, values):
    entry = stats.setdefault(name, {'samples': 0, 'sum': [0., 0., 0.]})
    entry['samples'] += 1
    for c, value in enumerate(values):
        entry['sum'][c] += value


def finish(stats):
    return {name: {'samples': st['samples'], 'rgb_mean': [v/st['samples'] for v in st['sum']]}
            for name, st in stats.items()}


def audit(original, captures, pose):
    def read(folder, suffix):
        return (folder/(pose+'.bmp'+suffix)).read_bytes()
    od, nd = read(original, '.composition-draw.rt00.bin'), read(captures, '.composition-draw.rt03.bin')
    oa, na = read(original, '.composition-draw.rt02.bin'), read(captures, '.composition-draw.rt01.bin')
    oe, ne = read(original, '.composition-draw.rt03.bin'), read(captures, '.sun-encoded-2.rt00.bin')
    oam, nam = read(original, '.sun-before.rt00.bin'), read(captures, '.ambient-encoded.rt00.bin')
    oo = read(original, '.composition-output.rt00.bin')
    no, approx = [read(captures, f'.composition-output.rt{i:02d}.bin') for i in (2, 0)]
    parameters, normal = read(captures, '.composition-draw.rt02.bin'), read(original, '.composition-draw.rt01.bin')
    os, ns = read(original, '.composition-draw.rt04.bin'), read(captures, '.composition-draw.rt05.bin')
    shadow = read(captures, '.composition-draw.rt06.bin')
    if any(len(b) != 1280*720*4 for b in (oa, na, oe, ne, oam, nam, oo, no, approx, parameters, normal, os, ns)):
        raise ValueError('incomplete full-resolution buffers')
    if any(len(b) != 1280*720*2 for b in (od, nd, shadow)):
        raise ValueError('incomplete depth or shadow buffers')
    debug = struct.unpack_from('<4f', read(captures, '.composition-draw.cb0.bin'), 24*16)
    if debug[:2] != (1., 1.):
        raise ValueError('composition debug multipliers must be one')
    stats, spec_stats = {}, {}
    counts = {'exact_depth_samples': 0, 'identical_albedo_rgb_bytes': 0,
              'source_final_differs_from_ambient': 0, 'target_final_differs_from_ambient': 0,
              'shadow_dielectric_samples': 0, 'source_replay_outside_one_ulp_channels': 0,
              'target_replay_outside_one_ulp_channels': 0}
    rect = REGIONS[pose]
    for y in range(rect[1], rect[3]):
        for x in range(rect[0], rect[2]):
            i = y*1280+x
            if od[2*i:2*i+2] != nd[2*i:2*i+2] or not 0 < struct.unpack_from('<e', od, 2*i)[0] < 499.75:
                continue
            counts['exact_depth_samples'] += 1
            counts['identical_albedo_rgb_bytes'] += oa[4*i:4*i+3] == na[4*i:4*i+3]
            counts['source_final_differs_from_ambient'] += oe[4*i:4*i+4] != oam[4*i:4*i+4]
            counts['target_final_differs_from_ambient'] += ne[4*i:4*i+4] != nam[4*i:4*i+4]
            a = [(v/255)**2.2 for v in oa[4*i:4*i+3]]
            b = [((v/255+.055)/1.055)**2.4 for v in na[4*i:4*i+3]]
            gamma22 = [(v/255)**2.2 for v in na[4*i:4*i+3]]
            le, ln = [[8*v*v for v in rgb(data, i)] for data in (oe, ne)]
            outo, outn, outa = [unpack(struct.unpack_from('<I', data, i*4)[0]) for data in (oo, no, approx)]
            # Remainders include final R11G11B10 rounding, so do not call them
            # exact specular. The guarded calculation below uses input buffers.
            so = [o-aa*l for o, aa, l in zip(outo, a, le)]
            sn = [o-bb*l for o, bb, l in zip(outn, b, ln)]
            values = {'source_linear_albedo': a, 'target_linear_albedo': b,
                      'source_lighting': le, 'target_lighting': ln,
                      'source_output': outo, 'target_output': outn,
                      'source_specular_plus_rounding': so, 'target_specular_plus_rounding': sn,
                      'signed_lighting_contribution': [bb*(n-o) for bb, n, o in zip(b, ln, le)],
                      'signed_albedo_contribution': [(bb-aa)*o for aa, bb, o in zip(a, b, le)],
                      'signed_specular_rounding_contribution': [n-o for n, o in zip(sn, so)]}
            for name, value in values.items():
                add(stats, name, value)
            alternatives = {'full': outn, 'approximation': outa,
                            'source_albedo_only': [aa*l+s for aa, l, s in zip(a, ln, sn)],
                            'native_albedo_gamma22_only': [aa*l+s for aa, l, s in zip(gamma22, ln, sn)],
                            'source_lighting_only': [bb*l+s for bb, l, s in zip(b, le, sn)],
                            'source_specular_rounding_only': [bb*l+s for bb, l, s in zip(b, ln, so)]}
            for name, value in alternatives.items():
                add(stats, 'absolute_error_'+name, [abs(n-o) for n, o in zip(value, outo)])
            # Here both exact specular colors simplify to scalar amounts:
            # source non-metallic tint is white, target lerp weight saturates
            # to one, target intensity is one, and native sun visibility is 0.
            if (oa[i*4+3] & 127) > 25.4 or shadow[i*2] or parameters[i*4+3] != 255 or 1.04-4*(parameters[i*4]/255)**2 < 1:
                continue
            counts['shadow_dielectric_samples'] += 1
            packed = struct.unpack_from('<I', normal, i*4)[0]
            amount = (((packed >> 27) & 31)/31)**2.2 if oa[i*4+3] < 128 else 1.
            source_spec = [8*v*v*amount for v in rgb(os, i)]
            target_spec = unpack(struct.unpack_from('<I', ns, i*4)[0])
            add(spec_stats, 'source_exact_specular', source_spec)
            add(spec_stats, 'target_exact_specular', target_spec)
            add(spec_stats, 'source_gloss', [((packed >> 20) & 127)/127]*3)
            add(spec_stats, 'target_gloss', [parameters[i*4+1]/255]*3)
            for label, albedo, light, spec, output in (
                    ('source', a, le, source_spec, outo), ('target', b, ln, target_spec, outn)):
                predicted = [aa*l+s for aa, l, s in zip(albedo, light, spec)]
                errors = [abs(v-o) for v, o in zip(predicted, output)]
                add(spec_stats, label+'_replay_absolute_error', errors)
                for c, (expected, error) in enumerate(zip(predicted, errors)):
                    ulp = 2**(max(-14, math.floor(math.log2(expected))) - (5 if c == 2 else 6)) if expected > 0 else 2**(-19 if c == 2 else -20)
                    counts[label+'_replay_outside_one_ulp_channels'] += error > ulp+1e-7
    return {'pose': pose, 'rectangle': rect, 'counts': counts,
            'all_exact_depth': finish(stats), 'guarded_shadow_dielectrics': finish(spec_stats)}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('captures', type=Path)
    p.add_argument('--original', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    rows = [audit(a.original, a.captures, pose) for pose in REGIONS]
    result = {'scope': 'Two predefined static wall regions, every exact-depth pixel. Counterfactual component substitutions diagnose error sources, not proposed runtime replacements or fitted gains. Signed decomposition uses target albedo for lighting error and source lighting for albedo error; specular remainders include output quantization. Exact specular arithmetic is separately verified on the guarded shadow-dielectric subset.',
              'captures': str(a.captures.resolve()), 'original': str(a.original.resolve()), 'views': rows}
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'report': str(a.output), 'counts': [r['counts'] for r in rows]}))
    if any(row['counts'][k] for row in rows for k in ('source_replay_outside_one_ulp_channels', 'target_replay_outside_one_ulp_channels')):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
