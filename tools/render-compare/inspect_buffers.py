#!/usr/bin/env python3
"""Summarize captured HDR buffers and make fixed-exposure PNG previews."""
import argparse
import json
import math
from pathlib import Path
import struct
import zlib


def pixels(data, fmt):
    formats = {2: '<ffff', 10: '<eeee', 16: '<ff', 34: '<ee', 41: '<f', 54: '<e', 28: 'BBBB', 29: 'BBBB', 87: 'BBBB', 49: 'BB', 61: 'B'}
    if fmt in formats:
        for channels in struct.iter_unpack(formats[fmt], data):
            if fmt in (28, 29, 87, 49, 61): channels = tuple(v/255 for v in channels)
            if fmt == 87: channels = (channels[2], channels[1], channels[0], channels[3])
            yield channels
    elif fmt == 26:
        def decode(v, bits):
            exponent, mantissa = v >> bits, v & ((1 << bits)-1)
            if exponent == 31: return math.inf if not mantissa else math.nan
            return math.ldexp(mantissa/(1 << bits), -14) if not exponent else math.ldexp(1+mantissa/(1 << bits), exponent-15)
        for (v,) in struct.iter_unpack('<I', data):
            yield decode(v & 2047, 6), decode((v >> 11) & 2047, 6), decode(v >> 22, 5)
    else: raise ValueError(f'unsupported DXGI format {fmt}')


def png(path, width, height, data):
    def chunk(kind, payload):
        return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload))
    rows=b''.join(b'\0'+data[y*width*3:(y+1)*width*3] for y in range(height))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(rows))+chunk(b'IEND',b''))


def inspect(index):
    base=Path(str(index).removesuffix('.targets.json'))
    summary=[]
    for item in json.loads(index.read_text()):
        if not item.get('written'): continue
        file=Path(str(base)+f".{'vol' if 'depth' in item else 'rt'}{item['slot']:02d}.bin")
        output=bytearray(); totals=[0.,0.,0.]; minima=[math.inf]*3; maxima=[-math.inf]*3; count=0; nonfinite=0; examples=[]
        for pixel in pixels(file.read_bytes(),item['format']):
            if len(examples)<4:examples.append(pixel)
            rgb=(pixel[:3] if len(pixel)>=3 else (pixel[0],)*3)
            if not all(math.isfinite(v) for v in rgb):
                nonfinite+=1; rgb=(0.,0.,0.)
            for i,v in enumerate(rgb):
                totals[i]+=v; minima[i]=min(minima[i],v); maxima[i]=max(maxima[i],v)
                # Fixed display mapping only; raw values are retained unchanged.
                linear=max(0,min(1,v)); srgb=12.92*linear if linear<=.0031308 else 1.055*linear**(1/2.4)-.055
                output.append(round(255*srgb))
            count+=1
        item=dict(item,mean=[v/count for v in totals],minimum=minima,maximum=maxima,nonfinite=nonfinite,first_pixels=examples)
        summary.append(item)
        png(file.with_suffix('.png'),item['width'],item['height']*item.get('depth',1),output)
    result=Path(str(base)+'.summary.json');result.write_text(json.dumps(summary,indent=2)+'\n')
    for item in summary:print(item['slot'],item['name'],item['format'],item['width'],item['height'],'mean',item['mean'])

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('index',type=Path);args=parser.parse_args();inspect(args.index)
