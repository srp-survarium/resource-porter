#!/usr/bin/env python3
"""Verify the captured original final-gamma draw before later AA and HUD passes."""
import argparse
import json
import math
from pathlib import Path
import struct

from inspect_buffers import pixels


def verify(base):
    def read(suffix):return Path(str(base)+suffix).read_bytes()
    meta=json.loads(read('.present-output.targets.json'))[0]
    assert meta['format'] in (28,87) and meta['written']
    assert json.loads(read('.present-output.view.json'))['format']==meta['format'], 'sRGB/typeless render-target view is not modeled'
    assert json.loads(read('.present-draw.targets.json'))[0]['format']==26
    assert read('.post-output.rt00.bin')==read('.present-draw.rt00.bin'), 'unmodeled pass before gamma'
    assert json.loads(read('.present-draw.blend.json'))=={'enabled':0,'source':2,'destination':1,'operation':1}
    gamma=struct.unpack_from('<f',read('.present-draw.cb0.bin'))[0]
    assert gamma>0 and math.isfinite(gamma)
    source=read('.present-draw.rt00.bin');actual=read('.present-output.rt00.bin')
    count=meta['width']*meta['height'];assert len(source)==len(actual)==count*4
    errors=0;nonfinite=0;maximum=0.
    for before,after in zip(pixels(source,26),pixels(actual,meta['format'])):
        for a,b in zip(before[:3],after[:3]):
            expected=max(0,min(1,abs(a)**(1/(2.2*gamma))))
            if not math.isfinite(a) or not math.isfinite(b):nonfinite+=1;continue
            delta=abs(expected-b);maximum=max(maximum,delta);errors+=delta>1/255+1e-7
    return {'pixels':count,'gamma':gamma,'post_to_copy_input_identical':True,'channels_outside_one_unorm8_step':errors,'nonfinite_channels':nonfinite,'maximum_absolute_error':maximum}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('capture',type=Path);args=parser.parse_args()
    bases=sorted(args.capture.glob('*.bmp'));assert bases,'no captured views'
    results={p.name:verify(p) for p in bases};print(json.dumps(results,indent=2))
    if any(r['channels_outside_one_unorm8_step'] or r['nonfinite_channels'] for r in results.values()):raise SystemExit('final gamma verification failed')
