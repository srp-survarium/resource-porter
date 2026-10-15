#!/usr/bin/env python3
"""Shine: specular in composition, original 0.11e vs port, per view, by original probe coverage and material class.
Original specular part = composition - albedo^2.2*(D+(1-|D|)*lpv) (exact reconstruction).
Port specular part (no sun/scatter) = composition - sRGB(albedo)*accum_diffuse, valid where port shadow.x==0 and opaque.
usage: OUT=json shine.py ORIG_POSES PORT_DIR VIEW..."""
import json, os, sys
import numpy as np
from PIL import Image
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from buffers import r26, r28, r24
L = np.array([.2126, .7152, .0722])
def lum(a): return a@L if a.ndim == 3 else a
def srgb(x): return ((np.abs(x)+.055)/1.055)**2.4
def slots(base, stage, fmt, width=1280):
    return [e['slot'] for e in json.load(open(f'{base}.{stage}.targets.json')) if e.get('format') == fmt and e.get('width') == width and 'skipped' not in e]
od, pdir, views = sys.argv[1], sys.argv[2], sys.argv[3:]
out = {}
for v in views:
    o = f'{od}/{v}.bmp'; c = f'{pdir}/{v}.bmp'
    cd = 'composition-draw'; s26 = slots(c, cd, 26); s28 = slots(c, cd, 28)
    araw = np.fromfile(o+'.composition-draw.rt02.bin', np.uint8).reshape(720, 1280, 4)
    galb_o = araw[..., :3]/255.; aw = araw[..., 3].astype(int); f0_o = (aw & 127)/127.; flag_o = aw >> 7
    nrm = np.fromfile(o+'.composition-draw.rt01.bin', np.uint32).reshape(720, 1280); gloss_o = ((nrm >> 20) & 127)/127.
    galb_p = r28(f'{c}.{cd}.rt{s28[0]:02d}.bin')[..., :3]; par = r28(f'{c}.{cd}.rt{s28[1]:02d}.bin')
    D_o = 8*r24(o+'.composition-draw.rt03.bin')**2; D_p = r26(f'{c}.{cd}.rt{s26[0]:02d}.bin')
    S_o = 8*r24(o+'.composition-draw.rt04.bin')**2; S_p = r26(f'{c}.{cd}.rt{s26[1]:02d}.bin'); S_p0 = r26(c+'.after-hba.rt34.bin')
    lpv = r26(o+'.composition-draw.rt05.bin', 640, 360).repeat(2, 0).repeat(2, 1)
    dl = np.minimum(np.sqrt((D_o*D_o).sum(-1, keepdims=True)), 1)
    comp_o = r26(o+'.composition-output.rt00.bin'); comp_p = r26(c+'.composition-output.rt00.bin')
    spec_o = lum(comp_o-galb_o**2.2*(D_o+(1-dl)*lpv)); spec_p = lum(comp_p-srgb(galb_p)*D_p)
    idx = np.fromfile(o+'.probe-draw.rt05.bin', np.uint8).reshape(720, 1280)
    amb = 8*r24(o+'.sun-before.rt00.bin')**2; osun = lum(8*r24(o+'.sun-output.rt00.bin')**2-amb)
    shadow_p = np.fromfile(f'{c}.{cd}.rt06.bin', np.uint8).reshape(720, 1280, 2)[..., 0]/255.
    valid = (lum(galb_o) > .02) & (lum(galb_p) > .02) & (par[..., 2] <= .001)
    shade = valid & (osun < .01) & (shadow_p == 0)
    classes = {'all shade': shade,
               'shade, sky only (orig probe index 0)': shade & (idx == 0),
               'shade, box probes (orig index>0)': shade & (idx > 0),
               'metal (orig F0>.2)': shade & (f0_o > .2),
               'glossy dielectric (gloss>.9, F0<=.2) e.g. wet/polished': shade & (f0_o <= .2) & (gloss_o > .9),
               'rough dielectric (gloss<=.9, F0<=.2) e.g. concrete': shade & (f0_o <= .2) & (gloss_o <= .9)}
    rec = out[v] = {}
    print(f'## {v}: shade px {shade.sum()} ({shade.mean():.0%} of frame); orig index-0 share of shade {np.mean(idx[shade] == 0) if shade.any() else 0:.0%}')
    for k, m in classes.items():
        n = int(m.sum())
        if n < 1500: print(f'   {k}: {n} px'); continue
        r = rec[k] = {'pixels': n, 'orig_spec_part': float(spec_o[m].mean()), 'port_spec_part': float(spec_p[m].mean()),
                      'orig_diffuse_part': float(lum(galb_o**2.2*D_o)[m].mean()), 'port_diffuse_part': float(lum(srgb(galb_p)*D_p)[m].mean()),
                      'orig_spec_accum': float(lum(S_o)[m].mean()), 'port_spec_accum_probes': float(lum(S_p0)[m].mean()), 'port_spec_accum_total': float(lum(S_p)[m].mean()),
                      'port_spec_share_of_comp': float(spec_p[m].mean()/max(lum(comp_p)[m].mean(), 1e-9)),
                      'comp_p_over_o': float(lum(comp_p)[m].mean()/max(lum(comp_o)[m].mean(), 1e-9))}
        print(f'   {k:58s} n={n:6d}  spec part o {r["orig_spec_part"]:.5f} p {r["port_spec_part"]:.5f} | diffuse part o {r["orig_diffuse_part"]:.4f} p {r["port_diffuse_part"]:.4f} | '
              f'spec accum o {r["orig_spec_accum"]:.5f} p {r["port_spec_accum_total"]:.5f} | port spec share {r["port_spec_share_of_comp"]:.0%} | comp p/o {r["comp_p_over_o"]:.2f}')
    # Small highlights: original display luma >= .35. Specular share of each composition,
    # original probe vs local-light specular, and the effect of the original specular part.
    disp = np.asarray(Image.open(o).convert('RGB'), float)/255
    hl = (lum(disp) >= .35) & valid
    if hl.sum() >= 20:
        probe_o = lum(8*r24(o+'.sun-before.rt01.bin')**2)
        so, co = spec_o[hl].mean(), lum(comp_o)[hl].mean(); sp, cp = spec_p[hl].mean(), lum(comp_p)[hl].mean()
        rec['highlights'] = {'pixels': int(hl.sum()), 'orig_spec_share': float(so/co), 'port_spec_share': float(sp/cp),
                             'orig_probe_spec_accum': float(probe_o[hl].mean()), 'orig_local_spec_accum': float((lum(S_o)-probe_o)[hl].mean()),
                             'port_probe_spec_accum': float(lum(S_p0)[hl].mean()), 'port_local_spec_accum': float((lum(S_p)-lum(S_p0))[hl].mean()),
                             'comp_p_over_o': float(cp/co), 'port_with_orig_spec_over_o': float((cp-sp+so)/co)}
        h = rec['highlights']
        print(f"   highlights (orig display >= .35) n={h['pixels']}: spec share o {h['orig_spec_share']:.0%} p {h['port_spec_share']:.0%}; "
              f"spec accum o probe {h['orig_probe_spec_accum']:.4f} local {h['orig_local_spec_accum']:.4f} | p probe {h['port_probe_spec_accum']:.4f} local {h['port_local_spec_accum']:.4f}; "
              f"comp p/o {h['comp_p_over_o']:.2f}, with original specular {h['port_with_orig_spec_over_o']:.2f}")
if os.environ.get('OUT'): json.dump(out, open(os.environ['OUT'], 'w'), indent=1)
