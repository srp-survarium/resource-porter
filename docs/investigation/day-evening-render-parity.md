# Day and evening renderer comparison

The user requested an independent continuation of the v0.10b/v0.26 lighting
comparison for authored day and evening variants, with broader sampling to
avoid tuning to the four rain views. This investigation is incomplete.

## Isolation and references

Branch `codex/compare-day-evening` started at `f719d78`. The rain worker's client,
archive, prefix, server and original captures are not modified. Independent
runtime files are under `.work/render-compare/day-evening/`; the original client
uses port 2251 and the target uses 2252. The first attempted original launch
found 2249 occupied by an unrelated server. That process was left untouched;
only this worker's failed launch was stopped. A bounded error-log excerpt is
retained instead of its repeated panic output.

The source projects are `projects/level_03/client_project` (day) and
`projects/level_03_evn/client_project` (evening). Target map IDs are 180 and 181,
with projects `v010b_level_03/day_2` and `v010b_level_03_evn/evening_2`. These are
actual authored variants, not rain with a different tint.

The private original server was rebuilt from `/tmp/srp-render-reference` source,
with port 2251 and the evening profile restored to `level_03_evn`. Its day profile
selects `level_03`. Original shader overrides remain disabled. Both Wine prefixes
have private Documents folders so client settings cannot modify the user's
profile or the rain worker's settings.

## Sampling

`tools/render-compare/poses-day-evening.json` preserves the initial 18-view plan:
ten distinct positions, with six views at three positions held out. Those
positions used farthest-spaced authored spawn points plus the existing upper,
interior and yard views. The user then requested coverage beyond the spawn
clusters. The original plan is retained; `poses-day-evening-coverage.json` adds
14 spatially distinct positions, and `poses-day-evening-expanded.json` combines
them into 32 views across 24 positions.

Added locations span the hospital paths, officer roof/terrace, central vehicle
route, chemical yard and east route, southeast rise, garage east route, upper
central courtyard, west terrace, northwest route, north gate and northeast
route. Their eye heights use an intersection with actual original collision
triangles plus 1.65 metres. Coordinates span x [-60,77.857], z [-106.277,40],
with eye heights [1.961,15.5]. Terrain itself extends farther; these coordinates
are evidence of spatial coverage, not a claim to sample every accessible place.

The initial day capture finished before the broader-coverage request arrived.
It remains frozen in `010b/day/`; only the 14 new positions are added in a
separate `010b/day-coverage/` session. Evening uses one complete 32-view session
in `010b/evening/`. The view selection uses map geometry, not measured renderer
mismatch. The coverage extension is marked separately as held out.

## Observer changes

Commit `68c3081` adds R10G10B10A2_UNORM readback and matching channel decoding.
Original normal, diffuse-light and specular-light buffers were previously
skipped. The observer now recognizes 65 archived shader identities across 16
families, including probe, sun, translucency, skylight, sun mask and volume cone.
Capture retains the first draw of each reference family, not a complete frame
trace or every probe instance. Outputs and bound inputs are kept unchanged.

The C tools build with warnings as errors. Packed-format decoder checks cover
zero, full-scale and independently populated channels. No game tests were added.
Frozen manifests validate each written resource's dimensions/byte count and
hash each pose's artifacts. The initial day batch contains 342 packed 10-bit
buffers; the full evening batch contains 608.

## Authored post-processing differences

Actual original post-draw constant buffers confirm these filmic parameters:

| Variant | Shoulder | Linear strength | Linear angle | Toe strength | Toe numerator | Toe denominator |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Day | .3 | 6.2 | .2 | 5.9 | 1.5 | .1 |
| Evening | .2 | 7 | .2 | 6.2 | 2 | .01 |
| Rain reference | .15 | 6.2 | .2 | 5.9 | 1.5 | .04 |

Day uses `color_grading/nt`; evening uses `color_grading/evn`. Both captured
original variants upload grain strength .08 and dark-desaturation limit .03.
The rain shader's fixed tone curve must not be applied unchanged to these maps.
Native target baselines must be retained before adapting the connected pipeline.

Shared rain experiment `a7d0612` was cherry-picked as `d33a2e7`: probe gloss
complement correction and an optional original translucency base term. That
experiment is not yet validated for day or evening. Further work must compare
same-weather normal/roughness inputs, lighting buffers and whole images across
the expanded locations. Regional brightness alone is insufficient evidence of
visual parity.
