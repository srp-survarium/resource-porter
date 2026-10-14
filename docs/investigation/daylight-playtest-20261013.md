# Daylight playtest, 2026-10-13

Scope is Factory 0.10b day and Chemical Plant 0.11a day in the combined
`/mnt/hdd/Projects/survarium-map-pack-20261013/resources.db`. Other weather
variants are deferred. No fixes from this playtest have been validated yet.

## Factory reports

User screenshots in `/home/sheep/screenshots/`:

| Screenshot | Report |
| --- | --- |
| `2026-10-13-20-38-50.png` | Excessively glossy metal tanks and machinery. |
| `2026-10-13-20-39-12.png` | Bright, dense water artifacts when looking down. |
| `2026-10-13-20-39-24.png` | Skeleton and cloth remains look metallic. |
| `2026-10-13-20-39-37.png` | Some hangar lights are misplaced. |

Factory day uses `ported/v010w`, not the rain namespace `ported/v010b`.
Previous Chemical Plant ambient-mip findings do not establish the cause of
these Factory material defects. Preserve the existing atmosphere while
checking source material values, texture channels and native shader bindings.

## Chemical Plant collision demonstration

The user reported invisible collision in some places, with most of the map
working, and demonstrated a location near the factory boiler. Client 2's
observed camera eye at 20:59:58 local time was
`[-63.0432358, 8.87118912, 27.9907207]`, facing
`[0.997191727, -0.0740554109, 0.0111269513]`. These are camera coordinates,
not measured character feet or a confirmed contact point. The user had
previously corrected an accidental free-camera demonstration; the camera
observer itself used mode 0 and did not override movement.

Evidence directory:
`/home/sheep/Projects/survarium/resource-porter/.work/collision-review-20261013/`.
It contains `collision-spot-01.json`, `client2-positions.jsonl` and the saved
window screenshot. Subsequent observations moved toward
`[-67.87, 7.98, 29.82]`. Do not treat the initial eye as a unique blocked point.

Source collision instances 459/460 place
`props/machines/factory_boiler.model/{collision,walker_collision}` at
`[-66.765, 5.611, 29.921]`, rotation `[0, 1.571, 0]`, unit scale.
The walker collision is a compound of 26 primitives. Origin proximity alone
does not identify the offending shape: large building meshes may have remote
origins. Prior audits verified retained instance fields and decoded payloads,
but did not test the 0.26 cooked shapes, controller dimensions or contacts.

The private two-client session later stopped. At 21:07 the only open window
was `survarium_skhid.exe`, loading the separate 6,285,533,906-byte database in
`/home/sheep/games/Survarium-0.26g-srp-files(2)/game/`. Its screenshot showed
overgrown brick ruins, not the Chemical Plant demonstration. The old observer
status was stale. Do not attribute that window to the port or restart/modify
it to continue port diagnostics without resolving the user's intended window.

Unresolved: precise blocked direction/contact, whether the obstacle belongs
to the boiler or surrounding building, and whether the target engine cooks
the copied primitive dimensions identically. No collider has been removed.

## Packed-resource and engine audit

`tools/chemical-plant/collision_evidence.rs` extracts a bounded collision
working set without rebuilding a database. `audit_collision_region.py`
compares it with the original 0.11a database and lists shapes whose bounds
intersect the demonstration region. The packed database, not just the old
staging bundle, was checked: 599 retained instances preserve all fields;
445 payloads match bytewise or as decoded primitive configs. There were no
collision/walker substitutions. The sole missing instance is the previously
documented Soviet computer at `[-6.003, 6.695, 8.802]`, outside this region.

Eight collision instances intersect the region `[-69,5,26]..[-61,10,32]`.
Five pass the source movement-controller filter: terrain2, factory_boiler,
moss4, old_factory_base and old_factory_indoor01. The other three are the
boiler/building bullet-collision instances. Every candidate has a render
instance with exactly the same model, position, rotation and scale. Native
0.21/0.26 project samples retain the same common group/mask conventions:
movement 2/4, hittable 8/16, shared 10/20.

**Confirmed engine difference, not yet a confirmed cause of the user's contact:**
the 0.11a cylinder factory forwards saved dimensions `[x,y,z]`; 0.21 and
0.26 construct Bullet half-extents `[x,y,x]`. Old boiler cylinders have `z=0`.
The Bullet cylinder support function uses x as radius, but `getAabb` uses all
three half-extents. Therefore this changes bounds used to select collision
candidates, not simply the radius or placement of the intended cylinder.
Restoring zero bounds globally would reintroduce false-negative collisions
and is not an acceptable untested fix.

Disassembly verification, preferred image base `0x10000`:

| Build | Primitive factory | Cylinder branch | Cylinder constructor |
| --- | --- | --- | --- |
| 0.11a | `0x72b1a0` | `0x72b23d` reads `dim+8` | `0x751ad0` |
| 0.21d | `0x5904e` | `0x590ca` duplicates x into z | `0x74815` |
| 0.26g | `0x7a1f0` | `0x7a2b9` duplicates x into z | `0xa9d40` |

`collision_symbols.py` reads the original PDB PUB32 records; the installed
llvm-pdbutil rejects these large MSF directories. The 0.26 branch was found
by its unique instruction sequence and verified against its type dispatch,
constructor call and constructor dimension storage. Evidence assembly and
symbol JSON are preserved beside the region report in the evidence folder.

For the demonstrated boiler (target project instance 459), reconstructed
vertical bounds differ substantially:

| Walker primitive | 0.11a Y bounds | 0.26 Y bounds |
| --- | --- | --- |
| Cylinder 12 | 6.6832–6.6845 | 4.9844–8.3833 |
| Cylinder 14 | 6.5834–6.6811 | 4.9106–8.3538 |

These are analytical bounds using the recovered rotation convention. A
sensitivity check with a 0.9 m wide, 1.8 m tall capsule and assumed eye-to-foot
offsets 1.4/1.6/1.8 m finds respectively 7/5/3 of 42 camera samples overlapping
new cylinder bounds but not old ones. This is only supporting evidence:
actual 0.26 controller dimensions, crouch state, contact manifolds and sweep
paths have not been measured. It does not justify deleting those cylinders.

The saved screenshot was taken at 21:00:35.599, not at the initial marker.
The nearest telemetry sample is 21:00:35.993 at
`[-67.78634,7.99394,29.99897]`. Camera direction changed quickly around that
time, so the screenshot and initial marker must not be treated as a matched
pose. A controlled test must record the actual capsule/contact and compare
the same attempted movement with only this collision behavior changed.
