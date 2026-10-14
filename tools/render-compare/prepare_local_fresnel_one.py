#!/usr/bin/env python3
"""Guarded native point/spot diffuse exception for source F0=1 materials.

Source 010b/011e diffuse uses (F > .99 ? 1 : 1-F). F0=1 implies F=1
at every angle. This deliberately implements only that exact subset; all
F0<1 pixels and the entire native specular/shadow code remain unchanged.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
from dxbc_checksum import checksum, repair

PINNED = {
    '64f2a63c0797cd222f2ff2c52b5a52da': 5320,
    '9b44f4080013ee93003784dda8692a9e': 5848,
    '648947ae1279ba54b2b6805ed4041caf': 6072,
    '7bc3bed3087dd426f55e0fca52dfb356': 6600,
    'db2973527d6471517c3cae26870cee6e': 6736,
    'e3270049b07044038374cf6f45f12839': 7216,
    '33204e907519b2ad0afa798e6154aebc': 7524,
    'feb8cf8a08456eb3fee725687cdd6682': 8004,
    'ca3ae943bbed9a18f9f2139f470bb866': 8032,
    'a496e301c856e3e9d45b66f41249b4cf': 8484,
    '033d5b535eece9a9df86835d2f9f31de': 8792,
    'd21df123dd4b6f22ec78ab0cbc54f4fe': 9300,
}

def chunks(code):
    n, = struct.unpack_from('<I', code, 28)
    for offset in struct.unpack_from('<'+'I'*n, code, 32):
        size, = struct.unpack_from('<I', code, offset+4)
        yield code[offset:offset+4], code[offset+8:offset+8+size]


def instructions(words):
    at = 2
    while at < len(words):
        length = words[at] >> 24 & 127
        assert length and at+length <= len(words)
        yield at, words[at:at+length]
        at += length
    assert at == len(words)


def patch(code):
    identity = code[4:20].hex()
    assert PINNED[identity] == len(code) and checksum(code) == code[4:20]
    entries = list(chunks(code))
    index = next(i for i, (tag, _) in enumerate(entries) if tag in (b'SHDR', b'SHEX'))
    tag, body = entries[index]
    words = list(struct.unpack('<'+'I'*(len(body)//4), body))
    seq = list(instructions(words))
    temps = [(at, ins) for at, ins in seq if ins[0]&2047 == 104]
    assert len(temps) == 1 and len(temps[0][1]) == 2
    count = temps[0][1][1]
    # All twelve verified shaders sample parameters from t1/s2; register varies.
    sample = [(at, ins) for at, ins in seq if ins[0]&2047 == 72 and
              len(ins) == 11 and ins[1] == 0x1000f2 and
              ins[5:9] == [0x107e46, 1, 0x106000, 2]]
    assert len(sample) == 1, (identity, sample)
    # The sole 1-F followed immediately by mul_sat is the diffuse gate.
    gates = []
    for j, (at, ins) in enumerate(seq[:-1]):
        nxt = seq[j+1][1]
        if (ins[0]&2047 == 0 and len(ins) == 8 and
                ins[3] == 0x8010000a | (((ins[1] >> 4 & 15).bit_length()-1) << 4) and
                ins[4:5] == [0x41] and ins[5] == ins[2] and
                ins[6:] == [0x4001, 0x3f800000] and
                nxt[0]&0x7ff == 56 and nxt[0]&0x2000):
            gates.append((at, ins))
    assert len(gates) == 1, (identity, gates)
    gate_at, gate = gates[0]
    component = (gate[1] >> 4 & 15).bit_length()-1
    dst = gate[1:3]
    read = [0x10000a | (component << 4), gate[2]]
    # eq rN.x, sampled_parameters.x, 1; movc gate, rN.x, 1, gate.
    compare = [24 | (7 << 24), 0x100012, count, 0x10000a, sample[0][1][2], 0x4001, 0x3f800000]
    select = [55 | (9 << 24)] + dst + [0x10000a, count, 0x4001, 0x3f800000] + read
    additions = {sample[0][0]+11: compare, gate_at+len(gate): select}
    updated = words[:2]
    for at, ins in seq:
        if at == temps[0][0]: ins = [ins[0], count+1]
        updated += ins
        updated += additions.get(at+len(ins), [])
    updated[1] = len(updated)
    # Structural inverse: removing only the two additions and temp declaration
    # change recovers every original instruction exactly.
    stripped = []
    for _, ins in instructions(updated):
        if ins in (compare, select): continue
        if ins[0]&2047 == 104: ins = [ins[0], count]
        stripped += ins
    assert stripped == words[2:]
    entries[index] = (tag, struct.pack('<'+'I'*len(updated), *updated))
    header = bytearray(code[:32+4*len(entries)])
    result = header
    for i, (chunk_tag, data) in enumerate(entries):
        struct.pack_into('<I', result, 32+4*i, len(result))
        result += chunk_tag + struct.pack('<I', len(data)) + data
    struct.pack_into('<I', result, 24, len(result))
    result = repair(bytes(result))
    assert checksum(result) == result[4:20]
    for (old_tag, old), (new_tag, new) in zip(chunks(code), chunks(result)):
        assert old_tag == new_tag
        if old_tag not in (b'SHDR', b'SHEX'): assert old == new
    return result, {'parameters_sample_word': sample[0][0], 'diffuse_gate_word': gate_at,
                    'new_temporary': count, 'added_instructions': 2,
                    'other_instructions_identical': True, 'other_chunks_identical': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-directory', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--header', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    rows = []
    for identity, size in PINNED.items():
        code = (args.native_directory/(identity+'.dxbc')).read_bytes()
        fixed, audit = patch(code)
        filename = identity+'.dxbc'
        (args.output/filename).write_bytes(fixed)
        rows.append(dict(original_digest=identity, original_size=size,
                         original_sha256=hashlib.sha256(code).hexdigest(),
                         fixed_digest=fixed[4:20].hex(), fixed_size=len(fixed),
                         fixed_sha256=hashlib.sha256(fixed).hexdigest(), file=filename, **audit))
    (args.output/'manifest.json').write_text(json.dumps(rows, indent=2)+'\n')
    def bytes_c(value): return '{'+','.join('0x'+value[i:i+2] for i in range(0,32,2))+'}'
    lines = ['// Generated by prepare_local_fresnel_one.py; default-off source F0=1 subset.',
             'static const struct { unsigned size, fixed_size; unsigned char digest[16], fixed_digest[16]; const char *file; } local_fresnel_one_shaders[] = {']
    for row in rows:
        lines.append('    {%d,%d,%s,%s,"%s"},' % (row['original_size'],row['fixed_size'],bytes_c(row['original_digest']),bytes_c(row['fixed_digest']),row['file']))
    args.header.write_text('\n'.join(lines+['};','']))
    print(json.dumps({'variants': len(rows), 'output': str(args.output)}))

if __name__ == '__main__': main()
