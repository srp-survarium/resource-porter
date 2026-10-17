# SPDX-License-Identifier: GPL-3.0-or-later
"""composite.py layersdir out.png idx,idx,...  (normal blend, bottom-first order = file order)"""
import sys, numpy as np
from PIL import Image
from pathlib import Path
d=Path(sys.argv[1]); idx=[int(x) for x in sys.argv[3].split(',')]
acc=None
for i in idx:
    a=np.asarray(Image.open(d/f'{i:02d}.png')).astype(np.float32)/255
    if acc is None: acc=np.zeros_like(a)
    al=a[...,3:4]; acc[...,:3]=a[...,:3]*al+acc[...,:3]*(1-al); acc[...,3:4]=al+acc[...,3:4]*(1-al)
Image.fromarray((acc*255+0.5).astype(np.uint8)).save(sys.argv[2])
