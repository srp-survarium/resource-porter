# SPDX-License-Identifier: GPL-3.0-or-later
import numpy as np
from pathlib import Path
def read(path):
    b=Path(path).read_bytes(); idl=b[0]; assert b[2]==2,'uncompressed truecolour only'
    w=int.from_bytes(b[12:14],'little'); h=int.from_bytes(b[14:16],'little'); bpp=b[16]; desc=b[17]
    c=bpp//8; off=18+idl; n=w*h*c
    px=np.frombuffer(b[off:off+n],np.uint8).reshape(h,w,c)
    if not desc&0x20: px=px[::-1]
    px=px[...,[2,1,0,3][:c]] if c==4 else px[...,[2,1,0]]
    return np.ascontiguousarray(px), dict(raw=b,off=off,n=n,desc=desc,c=c)
def write(path, px, meta):
    """Same header, footer and pixel order as the source file; only pixel bytes change."""
    c=meta['c']; assert px.shape[2]==c and px.dtype==np.uint8
    p=px[...,[2,1,0,3][:c]] if c==4 else px[...,[2,1,0]]
    if not meta['desc']&0x20: p=p[::-1]
    raw=meta['raw']; out=raw[:meta['off']]+np.ascontiguousarray(p).tobytes()+raw[meta['off']+meta['n']:]
    assert len(out)==len(raw); Path(path).write_bytes(out)
