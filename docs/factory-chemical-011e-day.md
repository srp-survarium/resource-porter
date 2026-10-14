# Factory and Chemical Plant 0.11e day bundle

`ports/survarium-011e-day-to-026g.json` builds both maps in one additive bundle:
Chemical Plant **194** and Factory **195**, sharing `ported/v011e` resources.
The 0.11e build 884 archive is the geometry, lighting, material and texture donor.
No 0.10b weather recipe or blanket 0.11a visual overlay is applied.

```sh
python3 tools/chemical-plant/rebuild.py \
  --porter /path/to/vostok-port \
  --manifest ports/survarium-011e-day-to-026g.json \
  --donor-db /path/to/011e/resources.db \
  --server-donor-db /path/to/011a/resources.db \
  --target-db /path/to/original-026g/resources.db \
  --output /new/011e-day-candidate --bundle-only
```

## Explicit Factory gameplay fallback

The 0.11e archive contains no Factory server project. Its binary and Lua client
projects contain no battery spawners, and no alternate server project exists in
the archive. The manifest therefore explicitly pins the 0.11a build 816 server
project: payload SHA-256
`a8c6ca4f156b11dad06697d52d6768d0f4351a60545abf9db16dfa0ed98f7d11`,
from database SHA-256
`f59650ee69e834f7757461c54affd93b6fbb767ab841d70468e85019b059c4f6`.

The converter verifies the database and payload hashes and refuses this fallback
unless all **43 player respawns, two collectors and 47 interaction zones** exactly
match the primary 0.11e client. Those checks pass. Factory's **21 battery
candidates are authored 0.11a data**, not claimed as recovered 0.11e server data.
All candidate coordinates remain unchanged; nested battery origins receive the
native collision-centre offset. Factory's collision and render geometry still
come from 0.11e, not the fallback server's obsolete collision array.

## CPU result and remaining runtime checks

The combined bundle contains **8,585 files / 1,209,865,537 bytes**, 333 converted
materials and 162 shared native dependencies. Dependency closure is complete,
every output hash passed, and Chemical's 33 battery transforms and distinct
collector zones pass the existing audit. No donor shader is imported.

`audit_factory_011e_spawns.py` reproduces the Factory fallback/transform checks and
compares initial native battery boxes with the actual 0.11e collision resources.
No collision payload required by the converted scene is missing. Nineteen
candidates have exact triangle or box support within 0.2 metres of the body
centre. Candidates **3 and 19** are among barrels: cylinder contact is deliberately
not inferred from conservative bounding boxes, and their nearest proven ground
surface is about one metre below. They need native settling checks.

Initial shallow contact at candidates **14, 17 and 18** reaches approximately
2.6–4.5 cm against a table or sofa. Candidates **0, 2 and 8** have nearby
non-box primitives that this CPU audit does not fully resolve. No body centre is
inside an exact box or below an upward collision triangle. These are initial
support/overlap checks, not proof of final settling or pickup accessibility.
No position was relocated.

Factory source `antenna_tree.model/walker_collision` is absent, but its original
ordinary collision payload exists and the converter's documented fallback keeps
it. `soviet_comp_02.model/collision` is absent without a walker counterpart;
this explicitly listed source omission is recorded for both maps.

The original 0.11e sphere shaders and native CPU upload agree on radial falloff;
explicit donor widths are retained, including authored reversed intervals.
Full probe appearance and selection remain separate parity questions. See
`chemical-plant-011e.md` and
`investigation/chemical-011e-probe-falloff-20261014.md`.

Artifacts: `/tmp/011e-day-candidate/bundle`, `chemical-cpu-audit.json` and
`factory-cpu-audit.json` beside it. This supersedes the Chemical-only bundle.
No full database or shared runtime resource was changed. Existing regression
functions cover the pinned fallback rejection and large config serialization;
no additional test functions were introduced.
