#!/usr/bin/env python3
"""Decode bounded native 0.26 movement-callback observations; no process access."""
import argparse
import json
from pathlib import Path
import struct


def decode(data):
    if len(data) < 16 or struct.unpack_from('<4I', data) != (0x434f4c31, 1, 1268, 0x10000):
        raise ValueError('unknown collision observer format')
    if (len(data) - 16) % 1268:
        raise ValueError('partial final record; preserve it, then copy a flushed capture')
    for at in range(16, len(data), 1268):
        r = data[at:at+1268]
        def u(offset): return struct.unpack_from('<I', r, offset)[0]
        def f(offset, count=1): return list(struct.unpack_from('<'+'f'*count, r, offset))
        flags = u(20)
        row = dict(tick=u(0), thread=u(4), callback=hex(u(8)), valid=flags,
                   ghost=hex(u(24)), controller=hex(u(28)), object=hex(u(32)),
                   wrapper=hex(u(36)), resource=hex(u(40)), resource_root=hex(u(44)),
                   normal_in_world_space=bool(u(16)), returned_fraction=f(56)[0])
        if flags & 1:
            row.update(sweep_from_bullet=f(60+0x10, 3), sweep_to_bullet=f(60+0x20, 3),
                       move_direction_bullet=f(60+0x60, 3), candidate_fraction=f(380+0x30)[0],
                       candidate_normal=f(380+0x10, 3), candidate_point_bullet=f(380+0x20, 3),
                       callback_filter=struct.unpack_from('<2h', r, 60+8),
                       previous_closest_fraction=f(60+4)[0], previous_hit=hex(u(60+0x50)))
        if flags & 2:
            part, index = struct.unpack_from('<2i', r, 48)
            row.update(shape_part=part, triangle_index=index)
            if part == -1:
                row['compound_child_index_candidate'] = index
        if flags & 4:
            origin = f(1064+0x30, 3)
            row.update(stand_dimensions=f(700, 2), crouch_dimensions=f(708, 2),
                       capsule_type=u(716+4), capsule_scaling=f(716+0x10, 3),
                       capsule_implicit_dimensions=f(716+0x20, 3), capsule_margin=f(716+0x30)[0],
                       capsule_up_axis=u(716+0x40), ghost_transform_bullet=f(1064, 16),
                       ghost_origin_bullet=origin, ghost_origin_engine=[origin[0], origin[1], -origin[2]],
                       ghost_filter=struct.unpack_from('<2h', r, 1128))
        if flags & 8:
            row.update(hit_transform_bullet=f(444+0x10, 16), hit_shape=hex(u(444+0xcc)),
                       hit_root_shape=hex(u(444+0xd4)), hit_flags=u(444+0xd8))
        for flag, offset, name in [(128, 1132, 'hit'), (256, 1200, 'root')]:
            if flags & flag:
                row[name+'_shape_type'] = u(offset+4)
                row[name+'_shape_raw_hex'] = r[offset:offset+0x44].hex()
                # Convex subclasses share these fields; leave compound/mesh payload uninterpreted.
                if u(offset+4) in (0, 8, 10, 13):
                    row[name+'_shape_scaling'] = f(offset+0x10, 3)
                    row[name+'_shape_implicit_dimensions'] = f(offset+0x20, 3)
                    row[name+'_shape_margin'] = f(offset+0x30)[0]
        if flags & 64:
            row.update(closest_fraction=f(220+4)[0], accepted_hit=hex(u(220+0x50)),
                       accepted_normal_bullet=f(220+0x30, 3), accepted_point_bullet=f(220+0x40, 3))
            row['accepted_state_changed'] = r[60+4:60+8] != r[220+4:220+8] or r[60+0x30:60+0x54] != r[220+0x30:220+0x54]
            # Equality alone can also mean the same object/fraction was already accepted.
            row['candidate_matches_accepted_state'] = u(220+0x50) == u(32) and r[220+4:220+8] == r[380+0x30:380+0x34]
        yield row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    rows = list(decode(args.capture.read_bytes()))
    args.output.write_text(''.join(json.dumps(row, allow_nan=False)+'\n' for row in rows))
    print(json.dumps(dict(records=len(rows), objects=len({r['object'] for r in rows}),
                         changed=sum(r.get('accepted_state_changed', False) for r in rows))))

if __name__ == '__main__':
    main()
