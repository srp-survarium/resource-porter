#!/usr/bin/env python3
"""Audit the isolated sky equation, same-frame readiness and encoded additions."""
import argparse
import json
from pathlib import Path
import struct

from audit_original_skylight import evaluate_skylight


def audit(base, stride, allow_incomplete_locals=False):
    def read(suffix):
        return Path(str(base)+suffix).read_bytes()
    def metadata(suffix):
        return json.loads(read(suffix))
    sky=metadata('.sky-encoded.json')
    ambient=metadata('.ambient-encoded.json')
    local=metadata('.local-encoded.json')
    ao_state=metadata('.ssao-live.state.json')
    serials=[sky['present'],sky['ao_present'],ambient['present'],ambient['ao_present'],local['present'],ao_state['rendered_present'],ao_state['capture_present']]
    if len(set(serials))!=1 or not ambient['valid'] or not local['ambient_valid'] or not local['ambient_enabled']:
        raise ValueError(f'{base}: missing or stale ambient initialization')
    if (local['unhandled_draws'] and not allow_incomplete_locals) or local['failed_draws'] or ambient['failed_draws']:
        raise ValueError(f'{base}: unsupported or failed auxiliary draws')
    if ambient['sky_draws']!=1 or ambient['probe_draws']<1:
        raise ValueError(f'{base}: invalid sky/probe counts')
    state=sky['native_probe_state']
    expected_face=[1,6,6 if state['depth'][0] else 1,3]
    if state['reference']!=255 or state['depth'][1:]!=[0,4] or state['stencil']!=[1,64,255] or state['front']!=expected_face or state['back']!=expected_face:
        raise ValueError(f'{base}: first probe stencil differs from source effect')
    inputs={entry['slot']:entry for entry in metadata('.probe-draw.targets.json')}
    for slot,fmt in ((0,34),(2,54)):
        item=inputs[slot]
        if (item['format'],item['width'],item['height'])!=(fmt,1280,720):
            raise ValueError(f'{base}: first probe input contract changed')
    cb0,cb1=read('.probe-draw.cb0.bin'),read('.probe-draw.cb1.bin')
    eye=struct.unpack_from('<3f',cb0,112)
    matrix=[struct.unpack_from('<4f',cb1,176+i*16) for i in range(3)]
    params=[ambient['source_sky'][i*4:i*4+4] for i in range(8)]
    normal=read('.probe-draw.rt00.bin'); depth=read('.probe-draw.rt02.bin')
    ao=read('.ssao-live.rt02.bin'); result=read('.sky-encoded.rt00.bin')
    if len(normal)!=1280*720*4 or len(depth)!=1280*720*2 or len(ao)!=1280*720*2 or len(result)!=1280*720*4:
        raise ValueError(f'{base}: incomplete image bytes')
    nonzero=zero=failures=0; total=maximum=0.
    examples=[]
    for y in range(stride//2,720,stride):
        for x in range(stride//2,1280,stride):
            index=y*1280+x
            packed=struct.unpack_from('<I',result,index*4)[0]&0x3fffffff
            if not packed:
                zero+=1; continue
            nonzero+=1
            predicted=evaluate_skylight(params,eye,matrix,struct.unpack_from('<2e',normal,index*4),struct.unpack_from('<e',depth,index*2)[0],ao[index*2]/255,x,y)
            errors=[abs(predicted[c]-((packed>>(c*10))&1023)/1023) for c in range(3)]
            total+=sum(errors); maximum=max(maximum,max(errors))
            if max(errors)>1/1023+1e-6:
                failures+=1
                if len(examples)<8: examples.append({'pixel':[x,y],'predicted':predicted,'observed':[((packed>>(c*10))&1023)/1023 for c in range(3)]})
    ambient_bytes=read('.ambient-encoded.rt00.bin'); local_bytes=read('.local-encoded.rt00.bin')
    if len(ambient_bytes)!=len(result) or len(local_bytes)!=len(result):
        raise ValueError(f'{base}: incomplete accumulator bytes')
    decreased=increased=saturated=unchanged=0; maximum_increment=0; totals=[0,0,0]
    for (before,),(after,) in zip(struct.iter_unpack('<I',ambient_bytes),struct.iter_unpack('<I',local_bytes)):
        delta=[((after>>(10*c))&1023)-((before>>(10*c))&1023) for c in range(3)]
        decreased+=any(v<0 for v in delta); increased+=any(v>0 for v in delta)
        unchanged+=all(v==0 for v in delta); saturated+=any(((after>>(10*c))&1023)==1023 for c in range(3))
        maximum_increment=max(maximum_increment,max(delta))
        for c in range(3): totals[c]+=delta[c]
    return {'pose':str(base),'present':serials[0],'probe_draws':ambient['probe_draws'],'local_draws':local['replayed_draws'],
            'unhandled_local_draws':local['unhandled_draws'],'local_coverage_complete':not local['unhandled_draws'],
            'sky_nonzero_samples':nonzero,'sky_zero_samples_excluded':zero,'sky_outside_one_10bit_step':failures,
            'sky_maximum_channel_error':maximum,'sky_mean_absolute_rgb_error':total/(3*nonzero) if nonzero else None,
            'sky_examples':examples,'local_addition':{'decreased_pixels':decreased,'increased_pixels':increased,
                'unchanged_pixels':unchanged,'saturated_pixels':saturated,'maximum_increment_steps':maximum_increment,
                'mean_encoded_rgb_increment':[v/(1023*1280*720) for v in totals]},
            'limitations':['Sky zero samples do not establish stencil coverage.','Monotone local addition does not prove each source-light equation or raster coverage.','This does not compare target G-buffer values to original materials.']}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory',type=Path)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--stride',type=int,default=8)
    parser.add_argument('--allow-incomplete-locals',action='store_true',
                        help='Audit private sky/probe math while reporting unsupported local draws; does not certify full lighting')
    args=parser.parse_args()
    if args.stride<1: parser.error('stride must be positive')
    files=sorted(args.directory.glob('*.bmp.sky-encoded.json'))
    if not files: parser.error('no auxiliary skylight captures found')
    rows=[audit(Path(str(path).removesuffix('.sky-encoded.json')),args.stride,args.allow_incomplete_locals) for path in files]
    args.output.write_text(json.dumps(rows,indent=2)+'\n')
    print(json.dumps(rows,indent=2))
    if any(row['sky_outside_one_10bit_step'] or row['local_addition']['decreased_pixels'] for row in rows):
        raise SystemExit(1)


if __name__=='__main__':
    main()
