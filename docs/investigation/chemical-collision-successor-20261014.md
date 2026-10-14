# Chemical Plant 0.11e collision changes

The later donor contains a localized walking-collision correction directly in
front of the user's first recorded position. It does **not** change the nearby
boiler's collision geometry. Preserve the later donor's authored geometry;
there is no evidence for a general cylinder-dimension converter patch.

This is an offline comparison, not a replay of the player's contact. In
particular, the later screenshot position near X−67.79 is distinct from the
initial eye position X−63.04 used below.

## Local wall correction

Source `models/level/04/old_factory_base.model/walker_collision/exported_primitives`
changes from 168 to 173 primitives. Its instance transform is unchanged. Three
removed/added primitives intersect the inspected region X[−69,−61], Y[5,10],
Z[26,32]; all three belong to this walking-collision resource (group2/mask4).

| Shape | X bounds | Y bounds | Z bounds |
|---|---|---|---|
| Removed wall box | −62.638..−62.405 | 6.570..11.144 | 26.428..30.828 |
| Replacement main box | −62.643..−62.401 | 6.605..11.171 | 29.115..30.843 |
| Replacement upper box | −62.643..−62.401 | 8.868..10.033 | 26.402..27.072 |

The first recorded eye was [−63.0432,8.8712,27.9907], facing approximately +X.
The removed wall crosses that direction about 0.405 units ahead. Both replacement
boxes leave the eye's Z coordinate clear. This is a strong candidate for the
reported invisible blocker, but eye position is not controller shape and no
accepted native contact has yet linked the report to this box.

The 0.11e donor also removes five walker cylinders around X−72..−81,Z≈44 from
`old_factory_indoor01`; those are outside the reported boiler region.

## Cylinder changes are authored, not a universal new dimension convention

The boiler appears at precisely the same transforms in both donors. Its bullet
collision has 179 primitives and its walker collision has 26; both decoded
payloads are identical. All six boiler walker cylinders retain stored Z=0.
Changing only the donor for this resource cannot change its native 0.26 result.

Other cylinder assets do change. For example, `barrel_chemical_01/02` correct a
roughly sideways cylinder to identity rotation, with radius 0.5061→0.3128 and
half-height 0.3128→0.4523. Grouped barrels change too. Those barrel colliders are
bullet-only group8/mask16 in this map; they do not establish the cause of a
walking blocker.

PDB-backed addresses for 0.11e:

- `create_bt_primitive` 0x724930, cylinder branch 0x7249cd, constructor0x74bfd0.
- The branch still passes saved [x,y,z], including the Z load at0x7249e2.
- Constructor retains supplied Z; it does not replace it with X.
- Cylinder `getAabb` is0x74c0a0, versus0x751ba0 in0.11a. Their111 instructions
  match after normalizing direct-call relocation to the absolute-value helper.

Native0.26's independently verified constructor path instead passes [x,y,x].
This remains a cross-version bounds difference, but the successor audit does
not justify restoring zero-width bounds, deleting boiler cylinders or
uniformly rescaling imported primitives. The changelog alone does not identify
which engine/server code or authored assets constituted its cylinder fix.

## Complete comparison

The script matches resource path, position, rotation and scale; list indices
are not identity. It also compares every instance field and reads all payloads
under the union of referenced shape roots.

| Measure | Result |
|---|---|
| Collision instances | 600→654 |
| Fully identical instances | 573 |
| Common resource+transform identities | 576 |
| Referenced collision roots | 224→241 |
| Payloads | 367 identical,58 changed,72 added,20 removed |
| Client/server collision arrays | Identical within each donor |
| Extra server collision geometry | 48 each,47 geometric identities shared |

Added/removed payload counts describe presence in the archive union; the report
separately records whether each donor project references them. No placements,
including accepted battery locations, were changed during this audit.

## Reproduction and evidence

Donors:
`/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/{011a,011e}/app/resources.db`.
Project JSON inputs are root's decoded `/tmp/chemical-011e-audit/{011a,011e}-client_project.json`.
Create listing inputs with the existing `resources-db list DONOR` tool, then run:

```sh
python3 tools/chemical-plant/audit_collision_successor.py \
  --old-project /tmp/chemical-011e-audit/011a-client_project.json \
  --new-project /tmp/chemical-011e-audit/011e-client_project.json \
  --old-db '/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011a/app/resources.db' \
  --new-db '/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011e/app/resources.db' \
  --old-entries /tmp/chemical-011e-audit/011a-list.txt \
  --new-entries /tmp/chemical-011e-audit/011e-list.txt \
  --region -69 5 26 -61 10 32 \
  --output /tmp/chemical-011e-audit/collision-successor-report.json
```

Small evidence bundle:
`/tmp/resource-porter-chemical-011/.work/collision-successor-011e`.
It contains the complete payload/instance diff with SHA256s, local bounds,
client/server agreement, PDB symbols, disassembly and normalized AABB comparison.
No GPU session, shared archive mutation or converter change was made.
