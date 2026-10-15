#!/usr/bin/env python3
"""Stage-by-stage shade/sun comparison: original 0.11e stage buffers vs port (0.26) buffers.
Regions come from agreement of the two sun masks: original = increase of its sun stage,
port = composition input t_sun_shadow_and_scattering.x. Translucent (port parameters.z>0)
pixels are reported separately.
usage: OUT=json shade_stages.py ORIG_POSES_DIR PORT_DIR VIEW [VIEW...]"""
import json, os, sys
import numpy as np
from PIL import Image
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from buffers import r26, r28, r24
L = np.array([.2126, .7152, .0722])
def lum(a): return a@L if a.ndim == 3 else a
def srgb(x): return ((np.abs(x)+.055)/1.055)**2.4
def exposure(a):
    a = min(max(a, .09), .4); key = 1.03-2/(20*(np.log2(a+1)*.30103+2)); return (key/a)**.2
def slots(base, stage, fmt, width=1280):
    return [e['slot'] for e in json.load(open(f'{base}.{stage}.targets.json')) if e.get('format') == fmt and e.get('width') == width and 'skipped' not in e]
orig_dir, port_dir, views = sys.argv[1], sys.argv[2], sys.argv[3:]
out = {}
for v in views:
    o = f'{orig_dir}/{v}.bmp'; c = f'{port_dir}/{v}.bmp'
    if not (os.path.exists(o+'.composition-draw.rt03.bin') and os.path.exists(c+'.after-hba.rt31.bin')):
        print('##', v, 'missing buffers'); continue
    cd = 'composition-draw'; s26 = slots(c, cd, 26); s28 = slots(c, cd, 28)
    galb_o = r28(o+'.composition-draw.rt02.bin')[..., :3]; galb_p = r28(f'{c}.{cd}.rt{s28[0]:02d}.bin')[..., :3]
    par = r28(f'{c}.{cd}.rt{s28[1]:02d}.bin')
    alb_o = galb_o**2.2; alb_p = srgb(galb_p)
    sky = 8*r24(o+'.skylight-output.rt00.bin')**2
    amb_o = 8*r24(o+'.sun-before.rt00.bin')**2; amb_p = r26(c+'.after-hba.rt31.bin')
    sun_after = 8*r24(o+'.sun-output.rt00.bin')**2
    tb = 8*r24(o+'.translucency-before.rt00.bin')**2 if os.path.exists(o+'.translucency-before.rt00.bin') else sun_after
    to = 8*r24(o+'.translucency-output.rt00.bin')**2 if os.path.exists(o+'.translucency-output.rt00.bin') else sun_after
    D_o = 8*r24(o+'.composition-draw.rt03.bin')**2; D_p = r26(f'{c}.{cd}.rt{s26[0]:02d}.bin')
    S_o = 8*r24(o+'.composition-draw.rt04.bin')**2; S_p = r26(f'{c}.{cd}.rt{s26[1]:02d}.bin'); S_p0 = r26(c+'.after-hba.rt34.bin')
    lpv = r26(o+'.composition-draw.rt05.bin', 640, 360).repeat(2, 0).repeat(2, 1)
    dl = np.minimum(np.sqrt((D_o*D_o).sum(-1, keepdims=True)), 1)
    comp_o = r26(o+'.composition-output.rt00.bin'); comp_p = r26(c+'.composition-output.rt00.bin')
    diff_o = alb_o*(D_o+(1-dl)*lpv); spec_o = comp_o-diff_o
    eff_p = comp_p/np.maximum(alb_p, 1e-3)
    post_o = r26(o+'.post-draw.rt03.bin'); ps = slots(c, 'post-draw', 26); post_p = r26(f'{c}.post-draw.rt{ps[0]:02d}.bin')
    oavg = float(np.fromfile(o+'.post-draw.rt04.bin', np.float32)[0]); pavg = None
    for e in json.load(open(c+'.post-draw.targets.json')):
        if e.get('width') == 1 and 'skipped' not in e:
            pavg = float(np.fromfile(f"{c}.post-draw.rt{e['slot']:02d}.bin", np.float32)[0])
    od = np.asarray(Image.open(o).convert('RGB'), float)/255; pd = np.asarray(Image.open(c).convert('RGB'), float)/255
    shadow_p = np.fromfile(f'{c}.{cd}.rt06.bin', np.uint8).reshape(720, 1280, 2)[..., 0]/255.
    osun = lum(sun_after-amb_o); oloc = lum(D_o-to)
    rows = {
        'stored albedo': (galb_o, galb_p),
        'decoded albedo (o ^2.2 / p sRGB)': (alb_o, alb_p),
        'sky ambient (o)': (sky, None),
        'ambient (o sky+probes / p after decode)': (amb_o, amb_p),
        'sun increase (o)': (sun_after-amb_o, None),
        'translucency stage increase (o)': (to-tb, None),
        'local lights increase': (D_o-to, D_p-amb_p),
        'diffuse accum at composition': (D_o, D_p),
        'effective lighting (comp/albedo)': (comp_o/np.maximum(alb_o, 1e-3), eff_p),
        'specular accum': (S_o, S_p),
        'composition diffuse part (p: albedo*accum, shade only)': (diff_o, alb_p*D_p),
        'composition specular part (p: shade only)': (spec_o, comp_p-alb_p*D_p),
        'composition HDR': (comp_o, comp_p),
        'fog etc. (post input - composition)': (post_o-comp_o, post_p-comp_p),
        'post input HDR': (post_o, post_p),
        'display (stage frame; o has mb lift)': (od, pd),
    }
    valid = (lum(galb_o) > .02) & (lum(galb_p) > .02) & (lum(D_o) > .003)
    opaque = par[..., 2] <= .001
    regions = {'shade both, opaque': valid & opaque & (osun < .01) & (shadow_p == 0) & (oloc < .01),
               'shade both, translucent': valid & ~opaque & (osun < .01) & (shadow_p == 0),
               'sun both, opaque': valid & opaque & (osun > .2) & (shadow_p > .5),
               'sun only in port': valid & (osun < .01) & (shadow_p > .5),
               'sun only in original': valid & (osun > .2) & (shadow_p == 0)}
    rec = out[v] = {'exposure': {'o_avg': oavg, 'o_exp': exposure(oavg), 'p_avg': pavg, 'p_exp': exposure(pavg) if pavg else None},
                    'valid_pixels': int(valid.sum()), 'translucent_fraction': float((valid & ~opaque).sum()/max(valid.sum(), 1))}
    print(f'## {v}  exposure: orig avg {oavg:.3f} (x{exposure(oavg):.3f})  port avg {pavg:.3f} (x{exposure(pavg):.3f})  '
          f'valid px {valid.sum()}  translucent {rec["translucent_fraction"]:.0%}')
    for rn, m in regions.items():
        n = int(m.sum()); rec[rn] = {'pixels': n, 'fraction': n/max(int(valid.sum()), 1)}
        if n < 1500: print(f'   {rn}: {n} px'); continue
        print(f'   {rn}: {n} px ({n/valid.sum():.0%})')
        for k, (a, b) in rows.items():
            va = float(np.mean(lum(a)[m])); vb = None if b is None else float(np.mean(lum(b)[m]))
            rec[rn][k] = [va, vb]
            print(f'     {k:42s} o {va:8.4f}' + ('' if vb is None else f'  p {vb:8.4f}  p/o {vb/va if abs(va)>1e-6 else float("nan"):6.3f}'))
        # counterfactuals on the port composition (means over the region)
        cp = float(np.mean(lum(comp_p)[m])); co = float(np.mean(lum(comp_o)[m]))
        dec = float(np.mean(lum(comp_p*(galb_p**2.2/np.maximum(srgb(galb_p), 1e-6)))[m]))
        rec[rn]['cf'] = {'port comp': cp, 'orig comp': co, 'port comp with 2.2 albedo decode': dec}
        line = f'     counterfactual p/o: as captured {cp/co:.3f}; 2.2 albedo decode {dec/co:.3f}'
        if rn.startswith('shade'):
            both = float(np.mean(lum(galb_p**2.2*D_p+spec_o)[m])); nospec = float(np.mean(lum(alb_p*D_p+spec_o)[m]))
            rec[rn]['cf'].update({'port with original specular part': nospec, 'port with 2.2 decode and original specular': both})
            line += f'; original specular part {nospec/co:.3f}; both {both/co:.3f}'
        print(line)
if os.environ.get('OUT'): json.dump(out, open(os.environ['OUT'], 'w'), indent=1)
