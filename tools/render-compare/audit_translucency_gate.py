#!/usr/bin/env python3
"""Compare original translucency flags to native amounts on exact-depth pixels."""
import argparse
import json
from pathlib import Path
import struct


def audit(original, native, name):
    def read(base,suffix):
        return Path(str(base)+suffix).read_bytes()
    old,new=original/name,native/name
    old_depth=read(old,'.translucency-draw.rt00.bin')
    new_depth=read(new,'.composition-draw.rt03.bin')
    albedo=read(old,'.translucency-draw.rt02.bin')
    normal=read(old,'.translucency-draw.rt01.bin')
    parameters=read(new,'.composition-draw.rt02.bin')
    if len(old_depth)!=1280*720*2 or len(new_depth)!=len(old_depth) or any(len(data)!=1280*720*4 for data in (albedo,normal,parameters)):
        raise ValueError(f'{name}: incomplete image bytes')
    table=[[0,0],[0,0]]; matched=flag_off_positive=old_amount_flag_off=0; examples=[]
    for index in range(1280*720):
        if old_depth[index*2:index*2+2]!=new_depth[index*2:index*2+2]: continue
        depth=struct.unpack_from('<e',old_depth,index*2)[0]
        if not 0<depth<1000: continue
        matched+=1
        flag=albedo[index*4+3]>=128
        positive=parameters[index*4+2]>0
        amount=(struct.unpack_from('<I',normal,index*4)[0]>>27)&31
        table[int(flag and amount>0)][int(positive)]+=1
        old_amount_flag_off+=bool(amount and not flag)
        if positive and not flag:
            flag_off_positive+=1
            if len(examples)<8:
                examples.append([index%1280,index//1280,parameters[index*4+2],albedo[index*4+3]])
    return {'pose':name,'depth_identical_geometry_pixels':matched,
            'table_original_active_rows_native_positive_columns':table,
            'native_positive_source_flag_off':flag_off_positive,
            'source_packed_amount_nonzero_but_flag_off':old_amount_flag_off,'examples':examples}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original',type=Path); parser.add_argument('native',type=Path)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    paths=sorted(args.native.glob('*.bmp.composition-draw.targets.json'))
    if not paths: parser.error('no native composition captures found')
    rows=[]
    for path in paths:
        name=path.name.removesuffix('.composition-draw.targets.json')
        old=json.loads(Path(str(args.original/name)+'.translucency-draw.targets.json').read_text())
        new=json.loads(path.read_text())
        for items,expected in ((old,{0:54,1:24,2:28}),(new,{2:28,3:54})):
            slots={item['slot']:item for item in items}
            if any((slots[s]['format'],slots[s]['width'],slots[s]['height'])!=(fmt,1280,720) for s,fmt in expected.items()):
                raise ValueError(f'{name}: input format contract changed')
        rows.append(audit(args.original,args.native,name))
    report={'scope':'Same-weather exact-depth subset only; source class/stencil coverage not measured. Source active means albedo flag plus nonzero packed translucency. Equal quantized depth does not prove identical material or UV at foliage edges.','views':rows}
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


if __name__=='__main__':
    main()
