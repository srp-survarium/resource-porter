#!/usr/bin/env python3
"""A minimal AVM2 (ABC) patcher: append constants, insert code into one method body.

Only what v011e_ui_swf.py needs. The constant pool is parsed and re-written
with new entries appended at the end of their tables, so every existing
index keeps its meaning; every other section is copied byte for byte except
the one method body that changes (its code, code length and max_stack;
exception offsets past the insertion point move with it). Branches that
cross an insertion point are refused rather than rewritten.
"""
import struct

# Operand kinds per opcode (AVM2 overview, section "Instructions").
U30, U8, S24, SWITCH = 'u30', 'u8', 's24', 'switch'
OPS = {}
for op in (0x01, 0x02, 0x03, 0x07, 0x09, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x23, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b,
           0x30, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x47, 0x48, 0x50, 0x51, 0x52, 0x57, 0x64,
           0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x81, 0x82, 0x83, 0x84, 0x85, 0x87, 0x88, 0x89, 0x8f, 0x90, 0x91, 0x93, 0x95, 0x96, 0x97,
           0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb0, 0xb1, 0xb3, 0xb4,
           0xc0, 0xc1, 0xc4, 0xc5, 0xc6, 0xc7, 0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xf3):
    OPS[op] = ()
for op in (0x08, 0x2c, 0x2d, 0x2e, 0x2f, 0x31, 0x40, 0x41, 0x42, 0x49, 0x53, 0x55, 0x56, 0x58, 0x59, 0x5a, 0x5d, 0x5e, 0x5f, 0x60,
           0x61, 0x62, 0x63, 0x65, 0x66, 0x68, 0x6a, 0x6c, 0x6d, 0x6e, 0x6f, 0x80, 0x86, 0x92, 0x94, 0xb2, 0xc2, 0xc3, 0xf0, 0xf1, 0x06, 0x04, 0x05, 0x25):
    OPS[op] = (U30,)
for op in (0x43, 0x44, 0x45, 0x46, 0x4a, 0x4c, 0x4e, 0x4f):
    OPS[op] = (U30, U30)
OPS[0x24] = (U8,)                 # pushbyte
OPS[0x65] = (U30,)                # getscopeobject
OPS[0x32] = (U30, U30)            # hasnext2
OPS[0xef] = (U8, U30, U8, U30)    # debug
for op in (0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a):
    OPS[op] = (S24,)
OPS[0x1b] = (SWITCH,)             # lookupswitch


def u30(n):
    out = bytearray()
    while True:
        b = n & 0x7f; n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def read_u30(b, p):
    n = shift = 0
    while True:
        x = b[p]; p += 1
        n |= (x & 0x7f) << shift; shift += 7
        if not x & 0x80:
            return n, p


def instructions(code):
    """(offset, opcode, operands, length) of each instruction."""
    out, p = [], 0
    while p < len(code):
        start, op = p, code[p]; p += 1
        assert op in OPS, f'unknown opcode 0x{op:02x} at {start}'
        args = []
        for kind in OPS[op]:
            if kind == U30:
                v, p = read_u30(code, p); args.append(v)
            elif kind == U8:
                args.append(code[p]); p += 1
            elif kind == S24:
                args.append(int.from_bytes(code[p:p + 3], 'little', signed=True)); p += 3
            else:
                default = int.from_bytes(code[p:p + 3], 'little', signed=True); p += 3
                count, p = read_u30(code, p)
                cases = [int.from_bytes(code[p + 3 * i:p + 3 * i + 3], 'little', signed=True) for i in range(count + 1)]
                p += 3 * (count + 1); args.append((default, cases))
        out.append((start, op, args, p - start))
    return out


class Abc:
    def __init__(self, data):
        self.data = data
        p = 4
        self.ints, p = self._table(p, lambda q: self._skip_u30(q), 1)
        self.uints, p = self._table(p, lambda q: self._skip_u30(q), 1)
        self.doubles, p = self._table(p, lambda q: q + 8, 1)
        self.strings, p = self._table(p, self._skip_string, 1)
        self.namespaces, p = self._table(p, lambda q: self._skip_u30(q + 1), 1)
        self.nssets, p = self._table(p, self._skip_nsset, 1)
        self.multinames, p = self._table(p, self._skip_multiname, 1)
        self.after_pool = p
        # Walk to the method bodies to know each one's span.
        n, p = read_u30(data, p)
        for _ in range(n):
            p = self._skip_method(p)
        n, p = read_u30(data, p)
        for _ in range(n):
            p = self._skip_u30(p)
            m, p = read_u30(data, p)
            for _ in range(2 * m):
                p = self._skip_u30(p)
        classes, p = read_u30(data, p)
        for _ in range(classes):
            p = self._skip_instance(p)
        for _ in range(classes):
            p = self._skip_u30(p); p = self._skip_traits(p)
        n, p = read_u30(data, p)
        for _ in range(n):
            p = self._skip_u30(p); p = self._skip_traits(p)
        self.bodies_at = p
        n, p = read_u30(data, p)
        self.bodies = []
        for _ in range(n):
            start = p
            fields = []
            for _ in range(5):
                v, p = read_u30(data, p); fields.append(v)
            length, p = read_u30(data, p)
            code_at = p; p += length
            exc, p = read_u30(data, p)
            exceptions = []
            for _ in range(exc):
                e = []
                for _ in range(5):
                    v, p = read_u30(data, p); e.append(v)
                exceptions.append(e)
            traits_at = p
            p = self._skip_traits(p)
            self.bodies.append(dict(start=start, end=p, fields=fields, code=bytearray(data[code_at:code_at + length]),
                                    exceptions=exceptions, traits=data[traits_at:p]))
        assert p == len(data), 'trailing ABC bytes'
        self.new = {k: [] for k in ('doubles', 'strings', 'multinames')}

    def _skip_u30(self, p):
        return read_u30(self.data, p)[1]

    def _table(self, p, skip, base):
        n, p = read_u30(self.data, p)
        entries = []
        for _ in range(max(n - base, 0)):
            q = skip(p); entries.append(self.data[p:q]); p = q
        return entries, p

    def _skip_string(self, p):
        n, p = read_u30(self.data, p); return p + n

    def _skip_nsset(self, p):
        n, p = read_u30(self.data, p)
        for _ in range(n):
            p = self._skip_u30(p)
        return p

    def _skip_multiname(self, p):
        kind = self.data[p]; p += 1
        if kind in (0x07, 0x0d):
            return self._skip_u30(self._skip_u30(p))
        if kind in (0x0f, 0x10):
            return self._skip_u30(p)
        if kind in (0x11, 0x12):
            return p
        if kind in (0x09, 0x0e):
            return self._skip_u30(self._skip_u30(p))
        if kind in (0x1b, 0x1c):
            return self._skip_u30(p)
        if kind == 0x1d:
            p = self._skip_u30(p); n, p = read_u30(self.data, p)
            for _ in range(n):
                p = self._skip_u30(p)
            return p
        raise ValueError(f'multiname kind 0x{kind:02x}')

    def _skip_method(self, p):
        n, p = read_u30(self.data, p)
        p = self._skip_u30(p)
        for _ in range(n):
            p = self._skip_u30(p)
        p = self._skip_u30(p)
        flags = self.data[p]; p += 1
        if flags & 0x08:
            k, p = read_u30(self.data, p)
            for _ in range(k):
                p = self._skip_u30(p); p += 1
        if flags & 0x80:
            for _ in range(n):
                p = self._skip_u30(p)
        return p

    def _skip_instance(self, p):
        p = self._skip_u30(self._skip_u30(p))
        flags = self.data[p]; p += 1
        if flags & 0x08:
            p = self._skip_u30(p)
        n, p = read_u30(self.data, p)
        for _ in range(n):
            p = self._skip_u30(p)
        p = self._skip_u30(p)
        return self._skip_traits(p)

    def _skip_traits(self, p):
        n, p = read_u30(self.data, p)
        for _ in range(n):
            p = self._skip_u30(p)
            kind = self.data[p]; p += 1
            if kind & 0x0f in (0, 6):
                p = self._skip_u30(self._skip_u30(p))
                vindex, p = read_u30(self.data, p)
                if vindex:
                    p += 1
            else:
                p = self._skip_u30(self._skip_u30(p))
            if kind & 0x40:
                m, p = read_u30(self.data, p)
                for _ in range(m):
                    p = self._skip_u30(p)
        return p

    # Lookups and appends -------------------------------------------------
    def string(self, text):
        raw = u30(len(text.encode())) + text.encode()
        for i, s in enumerate(self.strings + self.new['strings']):
            if s == raw:
                return i + 1
        self.new['strings'].append(raw)
        return len(self.strings) + len(self.new['strings'])

    def find_string(self, text):
        raw = u30(len(text.encode())) + text.encode()
        return self.strings.index(raw) + 1

    def double(self, value):
        raw = struct.pack('<d', value)
        for i, s in enumerate(self.doubles + self.new['doubles']):
            if s == raw:
                return i + 1
        self.new['doubles'].append(raw)
        return len(self.doubles) + len(self.new['doubles'])

    def multiname(self, kind, *args):
        raw = bytes([kind]) + b''.join(u30(a) for a in args)
        for i, s in enumerate(self.multinames + self.new['multinames']):
            if s == raw:
                return i + 1
        self.new['multinames'].append(raw)
        return len(self.multinames) + len(self.new['multinames'])

    def multiname_raw(self, index):
        return self.multinames[index - 1]

    def multiname_name(self, index):
        raw = self.multinames[index - 1]
        if raw[0] in (0x07, 0x0d):
            ns, q = read_u30(raw, 1); name, _ = read_u30(raw, q)
        elif raw[0] in (0x09, 0x0e):
            name, _ = read_u30(raw, 1)
        else:
            return None
        s = self.strings[name - 1]
        n, q = read_u30(s, 0)
        return s[q:].decode()

    def package_namespace(self):
        empty = self.find_string('') if (u30(0)) in self.strings else 0
        for i, ns in enumerate(self.namespaces):
            if ns[0] == 0x16 and read_u30(ns, 1)[0] == empty:
                return i + 1
        raise ValueError('no public package namespace')

    def body_with(self, predicate):
        found = [b for b in self.bodies if predicate(b)]
        assert len(found) == 1, f'{len(found)} method bodies match'
        return found[0]

    def insert(self, body, offset, code, stack_extra):
        """Inserts `code` at instruction offset `offset` of `body`."""
        ins = instructions(bytes(body['code']))
        starts = {i[0] for i in ins} | {len(body['code'])}
        assert offset in starts, 'not an instruction boundary'
        for start, op, args, length in ins:
            targets = []
            if op in range(0x0c, 0x1b):
                targets = [start + length + args[0]]
            elif op == 0x1b:
                default, cases = args[0]
                targets = [start + default] + [start + c for c in cases]
            for t in targets:
                assert (start < offset) == (t < offset) or t == offset and start < offset, 'a branch crosses the insertion point'
        body['code'][offset:offset] = code
        for e in body['exceptions']:
            for k in (0, 1, 2):
                if e[k] >= offset:
                    e[k] += len(code)
        body['fields'][1] += stack_extra

    def bytes(self):
        out = bytearray(self.data[:4])
        def table(entries, new):
            all_ = entries + new
            out.extend(u30(len(all_) + 1 if all_ else 0)); [out.extend(e) for e in all_]
        table(self.ints, []); table(self.uints, []); table(self.doubles, self.new['doubles'])
        table(self.strings, self.new['strings']); table(self.namespaces, []); table(self.nssets, [])
        table(self.multinames, self.new['multinames'])
        out += self.data[self.after_pool:self.bodies_at]
        out += u30(len(self.bodies))
        for b in self.bodies:
            for v in b['fields']:
                out += u30(v)
            out += u30(len(b['code'])) + b['code'] + u30(len(b['exceptions']))
            for e in b['exceptions']:
                for v in e:
                    out += u30(v)
            out += b['traits']
        return bytes(out)
