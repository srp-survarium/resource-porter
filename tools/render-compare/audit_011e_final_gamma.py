#!/usr/bin/env python3
"""Verify original011e final gamma from pinned executable and archived shaders."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

EXE_SHA = '83d6cd0728e3ab71acfa8507b9458c23373ac25ebace9ca4bf1539e53f9f2396'
SHADERS = {
    'copy_image.ps': '685c00b3451a1fb3291f40696e7b4ae34b0b18f6bad32ab62e634b5f8f9cb451',
    'post_process_post_sharpen.ps': 'a9d9bdb481194a037e10f8cbd9f1d1d8aece9118d736e5f7f03965333b82e173',
    'present.ps': '0a747779f1f1c05f61b8208fab09f784ab448c1d1d337bf22490cd9af85747dc',
}

def shader_words(code):
    n, = struct.unpack_from('<I', code, 28)
    for at in struct.unpack_from('<'+'I'*n, code, 32):
        if code[at:at+4] == b'SHDR':
            size, = struct.unpack_from('<I', code, at+4)
            return struct.unpack_from('<'+'I'*(size//4), code, at+8)
    raise ValueError('missing source SHDR')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--exe', type=Path, required=True)
    p.add_argument('--database', type=Path, required=True)
    p.add_argument('--listing', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args(); a.output.mkdir(parents=True, exist_ok=True)
    exe = a.exe.read_bytes()
    assert hashlib.sha256(exe).hexdigest() == EXE_SHA
    pe, = struct.unpack_from('<I', exe, 0x3c)
    count, = struct.unpack_from('<H', exe, pe+6)
    optional, = struct.unpack_from('<H', exe, pe+20)
    image_base, = struct.unpack_from('<I', exe, pe+24+28)
    sections = [struct.unpack_from('<4I', exe, pe+24+optional+i*40+8) for i in range(count)]
    def read(va, length):
        for _, rva, raw_size, raw_at in sections:
            at = va-image_base-rva
            if 0 <= at and at+length <= raw_size:
                return exe[raw_at+at:raw_at+at+length]
        raise ValueError(hex(va))
    def u32(va): return struct.unpack('<I', read(va, 4))[0]
    def target_name(index):
        branch = u32(0x630a70+index*4)
        assert read(branch,1) == b'\xb8' and read(branch+5,1) == b'\xc3'
        pointer = u32(branch+1)
        return read(pointer,80).split(b'\0',1)[0].decode()
    assert target_name(0x2e) == '$user$present'
    assert target_name(0x2f) == '$user$previous_present'
    checks = {
        0x55705b: ('swapchain format28 UNORM', 'c786a00000001c000000'),
        0x556acc: ('CreateRenderTargetView default descriptor', '6a0052508b4124ffd0'),
        0x6311b6: ('present resource format28', '6a1c'),
        0x6311bc: ('present resource index2e', 'b92e000000'),
        0x608e83: ('copy effect member+a0', '8d85a0000000e8328ba7ff'),
        0x608ec7: ('sharpen effect member+b0', '8d85b0000000e82e8fa7ff'),
        0x6086b0: ('apply copy effect', '8b83a000000033c9e833e2a5ff'),
        0x608752: ('copy output to present', 'b92e000000e8d4bda5ff53e83ec5ffff'),
        0x60869b: ('post-sharpen output to present', 'b92e000000e88bbea5ff53e875c2ffff'),
    }
    for va, (_, expected) in checks.items(): assert read(va,len(bytes.fromhex(expected))) == bytes.fromhex(expected),hex(va)
    found = {}
    with a.database.open('rb') as db:
        for line in a.listing.read_text().splitlines():
            fields = line.split(maxsplit=4)
            if len(fields)!=5 or not fields[4].startswith('shaders/sm_4_0/'): continue
            family = fields[4].split('/')[2]
            if family not in SHADERS: continue
            db.seek(int(fields[3])); entry=db.read(int(fields[1]))
            start=entry.index(b'DXBC'); size=struct.unpack_from('<I',entry,start+24)[0]
            code=entry[start:start+size]
            assert hashlib.sha256(code).hexdigest()==SHADERS[family]
            words=shader_words(code)
            if family=='copy_image.ps':
                assert words[0x15]&2047==56 and words[0x1d]&2047==14
                assert words[0x1c]==struct.unpack('<I',struct.pack('<f',2.2))[0]
                assert [words[x]&2047 for x in (0x30,0x36,0x3d)]==[47,56,25]
                equation='pow(abs(color),1/(gamma_correction_factor*2.2))'
            elif family=='post_process_post_sharpen.ps':
                assert [words[x]&2047 for x in (0x11e,0x124,0x12e)]==[47,56,25]
                assert words[0x12a]==struct.unpack('<I',struct.pack('<f',1/2.2))[0]
                equation='pow(abs(sharpened_color),1/2.2)'
            else:
                assert words[15]&2047==72 and words[26]&2047==62
                equation='passthrough sample of already encoded color'
            (a.output/(family+'.dxbc')).write_bytes(code)
            found[family]={'resource':fields[4],'sha256':SHADERS[family],'digest':code[4:20].hex(),'equation':equation}
    assert set(found)==set(SHADERS)
    report={'conclusion':'Retain one explicit final gamma conversion; original011e applies it before passthrough present, not via sRGB RTV.',
            'exe_sha256':EXE_SHA,'present_target_index':46,'present_target_name':target_name(46),
            'backbuffer_format':28,'srgb_rtv':False,
            'instruction_guards':[{'va':hex(va),'meaning':v[0],'bytes':v[1]} for va,v in checks.items()],
            'shaders':found,'limits':'Static executable/PDB and bytecode proof. Does not establish runtime AA branch, AA equivalence, gamma values other than captured1, or display color management.'}
    (a.output/'audit.json').write_text(json.dumps(report,indent=2)+'\n');print(report['conclusion'])

if __name__=='__main__': main()
