#!/usr/bin/env python3
"""Add the capture DLL import to a disposable copy of a pinned x86 client."""
import argparse
import hashlib
from pathlib import Path
import struct

PINNED = {
    '020e': 'ceee6e16b01e25bc8228c4335224eb78dd470f84672c94272907c68567347a87',
    '026g': '6a35b09ed3f9e48b18e19cdd4bf8ba8a5518eceeb75bba9049ad80150658e4cf',
    '010b': 'f7459d65611d20fcafc8b1db5e3fa3aae1013c22238c779787a79e70e4fd50f7',
    '011a': 'fd30e15c1a300df2bec6711074d98ad500e139f243c13e5924afd7c2ab994573',
    '011e': '83d6cd0728e3ab71acfa8507b9458c23373ac25ebace9ca4bf1539e53f9f2396',
}

def prepare(source, output, build):
    data = bytearray(source.read_bytes())
    if hashlib.sha256(data).hexdigest() != PINNED[build]:
        raise ValueError('client checksum does not match the verified build')
    def u16(at): return struct.unpack_from('<H', data, at)[0]
    def u32(at): return struct.unpack_from('<I', data, at)[0]
    def put(at, value): struct.pack_into('<I', data, at, value)
    def align(value, boundary): return (value + boundary - 1) // boundary * boundary
    pe = u32(0x3c)
    optional = pe + 24
    if u16(pe + 4) != 0x14c or u16(optional) != 0x10b:
        raise ValueError('expected PE32 x86')
    if u16(optional + 70) & 0x40:
        raise ValueError('ASLR client requires a relocatable bootstrap')
    count = u16(pe + 6)
    sections_at = optional + u16(pe + 20)
    sections = [struct.unpack_from('<8sIIIIIIHHI', data, sections_at + i*40) for i in range(count)]
    new_header = sections_at + count*40
    if new_header+40 > min(s[4] for s in sections if s[4]):
        raise ValueError('no room for another PE section header')
    def offset(rva):
        for s in sections:
            if s[2] <= rva < s[2] + s[3]: return s[4] + rva - s[2]
        raise ValueError(f'unmapped RVA {rva:x}')
    if build == '020e':
        # PDB get_local_video_memory_size: Wine can report below the minimum.
        # Only its verified failure branch returns the already-computed floor.
        # Avoid reference-only mip degradation when Wine reports no local VRAM.
        # Both source fallback constants use EDI; 1 GiB keeps full-quality mips.
        for rva in (0xb9172, 0xb9190):
            at = offset(rva)
            if data[at:at+5] != bytes.fromhex('bf00000010'):
                raise ValueError('020e video-memory minimum changed')
            data[at:at+5] = bytes.fromhex('bf00000040')
        at = offset(0xb91d3)
        expected = bytes.fromhex('680001000068106a8000e87a900000')
        if data[at:at+15] != expected:
            raise ValueError('020e video-memory failure branch changed')
        replacement = bytes.fromhex('8bc733d2eb0d') + bytes([0x90])*9
        assert len(replacement) == len(expected)
        data[at:at+len(expected)] = replacement
    if build == '020e':
        # The direct spectator path has no lobby/chat Flash movie. These PDB-
        # identified callbacks only update UI; the world/renderer stay original.
        callbacks = (
            (0x14e372, '558bec81ec8c040000', 8),  # chat_handler::set_mode
            (0x15578d, '558bec83e4f883ec1c', 8),  # switch_to_level_loading
            (0x1558cd, '558bec81ec38020000', 4),  # update_level_loading_progress
        )
        for rva, prologue, popped in callbacks:
            at = offset(rva)
            expected = bytes.fromhex(prologue)
            if data[at:at+len(expected)] != expected:
                raise ValueError(f'020e reference UI callback changed: {rva:x}')
            data[at:at+3] = b'\xc2' + struct.pack('<H', popped)
    imports = bytearray()
    at = offset(u32(optional + 104))
    while any(data[at:at+20]):
        imports.extend(data[at:at+20]); at += 20
    rva = align(max(s[2]+max(s[1],s[3]) for s in sections), u32(optional+32))
    raw = align(len(data), u32(optional+36))
    payload = imports + bytes(40)
    def append(blob, alignment=4):
        payload.extend(bytes((-len(payload)) % alignment))
        address = rva+len(payload)
        payload.extend(blob)
        return address
    dll = append(b'camera_hook.dll\0')
    symbol = append(b'\0\0SRPCaptureInitialize\0')
    lookup = append(struct.pack('<II', symbol, 0))
    iat = append(struct.pack('<II', symbol, 0))
    struct.pack_into('<IIIII', payload, len(imports), lookup, 0, 0, dll, iat)
    image_base = u32(optional+28)
    # The PE loader resolves our import before entering this bootstrap.
    # Run initialization outside DllMain, preserving the original entry state.
    stub_rva = rva+len(payload)
    stub = b'\x9c\x60\xff\x15'+struct.pack('<I', image_base+iat)+b'\x61\x9d\xe9'
    stub += struct.pack('<i', u32(optional+16)-(stub_rva+len(stub)+4))
    payload.extend(stub)
    raw_size = align(len(payload), u32(optional+36))
    struct.pack_into('<8sIIIIIIHHI', data, new_header, b'.capture', len(payload), rva, raw_size, raw, 0, 0, 0, 0, 0xe0000040)
    struct.pack_into('<H', data, pe+6, count+1)
    put(optional+16, stub_rva)
    put(optional+56, align(rva+len(payload), u32(optional+32)))
    put(optional+64, 0)  # checksum; local diagnostic image is unsigned
    put(optional+104, rva)
    put(optional+108, len(imports)+40)
    # Keep the original certificate blob and file offset: the client reads
    # its embedded certificate during startup even for local patched builds.
    put(optional+184, 0); put(optional+188, 0)  # bound imports
    data.extend(bytes(raw-len(data)))
    data.extend(payload); data.extend(bytes(raw_size-len(payload)))
    with output.open('xb') as stream: stream.write(data)
    return hashlib.sha256(data).hexdigest()

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build', choices=PINNED, required=True)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(prepare(args.source, args.output, args.build))
