# Client-reported hit demo

This demo proves that the shipped Survarium v0.100b client can report its
existing bullet/player collision result to SRP and that SRP can kill one of the
server-spawned showcase characters. It intentionally does not merge the broader
SRP PR stack.

## Runtime chain

The launcher is:

```sh
./scripts/run-canonical-stk2-a01-hit-demo.sh
```

It keeps these verified inputs:

- the frozen masonry-v3 resource database, SHA-256
  `4a51b1a80540e93766851674c9c4e9041960a66ef15f06139a6183f2e0795174`;
- the canonical ten-character `stk2-mixed-roster` profile;
- the existing local-player suicide and ten-second roof respawn;
- the frozen login, browser, and lobby servers.

Only the match server and injected client patch are new. `--reuse` launches the
last built artifacts without rebuilding either one.

## Why a client patch is needed

The retail client already performs bullet collision and constructs
`survarium::hit_info`. That structure contains:

- attacking and target player IDs;
- body-part and damage-type names;
- calculated damage and armor piercing;
- a process-local bullet pointer.

Both recovered hit paths dispatch this structure through
`network_client::on_player_hit_received(hit_info const&)`. The corresponding
v0.100b virtual callback is empty, so the result never reaches the unfinished
server.

The version-locked 32-bit DLL replaces only that vtable slot. It does not patch
the shared empty function body because linker identical-code folding can make
unrelated empty functions share the same address. It verifies the executable's
PE timestamp, image size, and expected original vtable entry before installing
the hook. The existing crash-report recipient patch changes unrelated data and
is compatible.

The broader PR-stack DLL also bypassed two match-status guards for its
permanent-warmup design. This adaptation removes those patches: the canonical
showcase already sends `GameStatusChanged(inprocess)`, and its controls and
local respawn work without them.

## Synthetic client packet

The hook creates an ordinary ordered match packet with the SRP-only opcode
`client_player_hit = 0x4B`. Its packed payload is:

| Field | Wire type |
| --- | --- |
| version | `u8`, currently 1 |
| hit initiator | `u8` |
| target player | `u8` |
| body-part length and UTF-8 bytes | `u8` + at most 15 bytes |
| damage-type length and UTF-8 bytes | `u8` + at most 15 bytes |
| calculated damage | finite positive `f32 LE` |
| armor piercing | finite non-negative `f32 LE` |

The bullet pointer is process-local and is never serialized.

## Server rules

The checkpoint server has one real network client, player 0, and nine
server-spawned characters, players 1–9. For every syntactically valid report it:

1. replaces the untrusted attacker ID with player 0;
2. rejects player 0, absent IDs, and already-dead targets;
3. replaces client damage with a fixed 25 and armor piercing with zero;
4. subtracts that damage from a 100-point server-side health value;
5. sends stock `hit_player` (`0x89`) so the client receives the normal hit;
6. sends stock `kill_player` (`0x83`) when health reaches zero.

Four accepted hits therefore kill a full-health showcase character. Remote
characters are not scheduled for respawn. Local suicide still uses its
separate existing ten-second roof-respawn state.

Target disassembly confirms that although the PDB types the two `hit_player`
ID reads as `bool`, `packet_reader::r<bool>` copies the raw byte without
normalization. The server therefore serializes the complete IDs and can address
the entire fixed player array rather than only IDs zero and one.

## Trust boundary

This is a deliberately client-authoritative demo. The server validates packet
shape, target membership, liveness, and attacker identity, and it selects the
damage amount. It does not validate the shot ray, weapon state, rate of fire,
map geometry, visibility, distance, ammunition, or body-part intersection.
Those checks require an authoritative combat simulation or stronger
reconciliation and are future work.

Accepted hits and deaths are written to the match-server log:

```text
accepted demo hit: attacker=0 victim=1 body_part="..." health=75
demo character 1 killed by player 0
```
