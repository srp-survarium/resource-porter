# Chemical Plant collision observer

Offline ABI verified against native 0.26 executable SHA256
`ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153`.
Addresses below are preferred virtual addresses, image base `0x10000`.
No collision behavior or renderer recipe is changed. Runtime validation is pending.

## Verified native ABI

| Item | Address / layout | Evidence |
|---|---|---|
| Movement callback vtable | `0x9e3fd0` | RTTI `character_move_sweep_callback`; destructor `0x73ef0`, filter `0x73d70`, result `0x895f0` |
| Result callback | ECX=this, result pointer and bool on stack; float in ST0, `ret 8` | Disassembly `0x895f0..0x898b5` |
| Sweep filter | group 4, mask 2 at callback +8/+10 | Constructor writes `0x895bf..0x895dd` |
| Controller vtable | `0x9e3b9c` | RTTI `bullet_character_controller`; destructor `0x7b600`, update `0x7b730` |
| Controller shape | stand float2 +`0x60`, crouch float2 +`0x68`, ghost +`0x70`, capsule +`0x1b0` | Constructor `0x7b150` |
| Ghost filter | group `0x824`, mask 3 at controller +`0x2c700/+0x2c702` | Constructor `0x7b4b7..0x7b4cf`; distinct from movement sweep filter |
| Actual player dimensions | supplied through actor data +`0x1cc/+0x1d4` | Call chain `0x2d6eec → 0x3569d0 → 0x356a40 → 0x7b150`; do not assume old 0.9×1.8 defaults |
| Ghost position | controller +`0xb0`; full Bullet transform at ghost +`0x10` | Native updateAction and Bullet layout |
| Static collider wrapper | object +`0xf8` → wrapper, vtable `0xa1f258` | Constructor `0x356c20`, pointer assignment `0x356c67` |
| Resource mapping | wrapper +`0x10` → collision resource, wrapper +`0x14` → object; resource +`0x108` → root shape | Constructor/destructor and factory `0x356d60`; resource vtable `0x9e31ec` |
| Live object shapes | object +`0xcc` current shape, +`0xd4` root shape | Bullet ABI; snapshot inside callback before temporary compound state is restored |

Callback fields: closest fraction +4, sweep endpoints +`0x10/+0x20`,
accepted normal/point/object +`0x30/+0x40/+0x50`, move vector +`0x60`,
self ghost +`0x90`. Input `LocalConvexResult`: object +0, shape info +4,
normal +`0x10`, point +`0x20`, fraction +`0x30`.
Native acceptance writes begin at `0x89881`. A rejected candidate can return the
previous closest fraction; returned fraction alone does not prove acceptance.
The decoder reports state changes and candidate/state equality separately.

Original Bullet compound sweep sets shape part −1 and triangle index to child
index (`btCollisionWorld.cpp`, `LocalInfoAdder`). The observer preserves both
raw fields; the decoder labels this interpretation as a candidate until the
native hit and resource geometry agree. Static wrapper/resource pointers plus
world transform identify the authored collider without calling resource APIs.
Resource names are not yet resolved.

## Build and controlled use

```sh
CC=/path/to/i686-w64-mingw32-gcc \
LDFLAGS='-L/path/to/mcfgthread/lib' \
bash tools/chemical-plant/build_collision_observer.sh \
  /path/to/frozen/camera_hook.dll /tmp/collision-observer-build
```

The build produces a proxy `camera_hook.dll` and byte-identical frozen
`camera_hook_render.dll`. The proxy delegates `SRPCaptureInitialize` to the
frozen DLL, then starts collision observation. It does not rebuild rendering
code or alter shader assets. During an owned, stopped-client lease, save the
existing DLL and copy these two siblings into that private client. Set
`SRP_CAPTURE_COLLISION=1`; keep the existing `SRP_CAPTURE_CONTROL` and rendering
environment. Restore the original DLL after stopping the owned client.

Outputs beside the control file:

- `.collision.bin`: versioned 1,268-byte records, maximum 8,192 attempts (~10 MiB).
- `.collision-status.json`: installation state, seen/dropped/written counters.
- `.collision-stop`: creating this file stops observation, restores the vtable
  slot and flushes the ring while the game remains running. Remove a stale stop
  file before another run.

Decode after flushing:

```sh
python3 tools/chemical-plant/decode_collision_observer.py \
  /path/to/control.collision.bin --output /tmp/collision-contacts.jsonl
```

The hook changes only the guarded callback vtable slot. It always calls the
original callback with unchanged arguments and returns its float. Snapshot
reads use `ReadProcessMemory`; the callback writes a bounded nonblocking ring,
and a separate 10-ms worker performs file I/O. Overflow is counted. Diagnostic
read overhead is real; this is not a release feature. Do not unload the proxy
DLL while physics runs. Slot restoration alone cannot cancel an in-flight call.

The capture contains raw Bullet coordinates. The decoder additionally labels
engine ghost origin by reversing Bullet Z; ghost origin is capsule center, not
camera eye or feet. It logs both capsule parameters and live transforms so the
standing/crouching offset can be verified from the actual test.

## Validation and boundaries

Cross-compiled proxy passed `-std=c11 -O2 -Wall -Wextra -Werror -static -shared`.
Generated assembly passes ECX and both stack arguments to `0x895f0`, restores the
float through ST0 and returns with `ret 8`. A synthetic record checked binary
layout, child index 12, accepted-state changes, coordinate conversion and bad
header rejection. No client has been launched for this observer yet.

This observes movement sweeps only. Step-up/down callbacks, resting manifold
contacts and rejected broadphase objects are not captured. If the reported
blockage has no movement callback hit, inspect those paths next rather than
claiming collision is clear. The old/new boiler cylinder bounds difference
remains a hypothesis until an accepted native contact identifies its child.
