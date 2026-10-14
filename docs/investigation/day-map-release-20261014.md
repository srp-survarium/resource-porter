# Day map release work

The user expanded the release to three maps during the 2026-10-14 review.
The earlier “Factory” name was wrong: `level_03` is Vostok Radar Station.
Original project landmarks, cross-version physical continuity, and native
localization establish its identity; see [the donor audit](radar-donor-identity-20261014.md).
Internal script names and namespaces retain their historical spelling so saved
recipes remain reproducible. The three prepared variants are:

| Map | Donor | Candidate ID |
| --- | --- | --- |
| Vostok Radar Station day | 0.11e build 884 | 195 |
| Chemical Plant day | 0.11e build 884 | 194 |
| Vostok Radar Station day with anomalies | 0.20e | 196 |

The user called the first donor 0.11g; the available archive is 0.11e, which
was clarified in conversation. The current five-map package still contains
earlier donors. These candidates have not replaced it. Factory 0.10b remains
a useful reference, but is no longer one of the requested release maps.

The 0.11d note documents lighting fixes for Chemical Plant and Radar, plus
cylindrical collision fixes. The 0.11e note documents an installer hotfix.
Radar is present in the 0.11e database as `level_03`; our earlier claim that it
was absent was a naming mistake. The release note does not establish a specific
cone-placement fix. Match original views and inspect source contracts before
claiming that any remaining rendering defect is fixed.

## Verified progress

Chemical battery placement was accepted by the user. All 33 native bodies
settle stably after the body-origin correction; the two-view gallery and
checkpoints are preserved. Authored parent positions were not moved.

The 0.11e Chemical additive bundle passes resource closure and all 4,173 file
hashes. It retains source collectors, battery origins and LPV geometry. The
newer wall collision opens the reported corridor; character traversal still
needs a native test. Binary/shader tracing supports preserving its authored
sphere width: the original also contracts the outer radius and produces the
same inverted falloff interval. This does not prove complete lighting parity.

Factory 0.11e lacks a server project in the archive. The build worker is
checking source scripts and earlier donor provenance before selecting any
server data. The selected fallback is the pinned 0.11a Factory server project:
its 43 client respawns, two collectors and 47 gameplay collision zones match
0.11e exactly. Its 21 battery candidates need a support check against the newer
map. Record the secondary donor explicitly; do not substitute modern 0.26
battery positions.

The private Factory 0.20e test proves electrical damage and repeated verified
two-client checkpoints. The user reports that the anomaly works. That session
has ended; the worker verified its processes and windows exited before
restoring diagnostic spawns. The new
screenshots identify lighting calibration locations, using matched original
views as in earlier comparisons. Source ambient-light contracts differ from
both 0.21 and 0.26; wholesale reuse of another preset is unverified.

## Remaining rendering work

Factory 0.11e remains textures/materials match the earlier donor byte-for-byte.
Their BC2 format is excluded from native streaming pools. The prepared BC3
candidate preserves every RGB block and mip, with independently checked alpha
thresholds. Native descriptor captures now prove that the baseline remains
textures bind only 64-square mips, while BC3 restores the authored 512-square
skeleton and 2048-square cloth textures. Visual review confirms restored holes
and detail. The BC3 correction can be promoted independently. Material-mask
restoration plus the source sun diffuse exception improves outdoor remains,
but still leaves cellar remains too dark: the source point/spot exception
needs a separate correction before that combination is promoted. The newer motor material already preserves
its source Fresnel switch, so the earlier motor patch must not be copied over.

The earlier Factory bulb emission fix is independently verified and its
converter repair is committed. Some shafts still need a source-opacity
comparison; authored transforms match the old source. The newer Factory's
lamp/cone audit finds unchanged transforms, meshes and falloff textures; all
30 cone shader variants preserve executable bytecode multiplicities. The newer
donor changes Factory lighting and bulb tint, but contains no demonstrated
beam-placement or cone-shader fix. Four existing material conversion tests pass
after combining the water and bulb repairs in the root worktree.

The current package has the individually verified old-Factory water foam,
motor Fresnel and bulb emission patches applied. Reversible plans and refreshed
hashes are under `validation/release-fixes-20261014`; the combined runtime
check remains pending. Those old-donor checks do not certify the new release.

Keep candidate resources separate until their source contracts, native loading,
gameplay/collision behavior and visual comparisons are checked. Preserve the
scripts, decoded evidence and compact screenshots; build the final shared
database once the three candidates are ready.

## Combined candidate

`/tmp/three-day-map-candidate/resources.db` contains IDs 194, 195 and 196;
SHA256 `241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d`.
All 13,797 composed resources were checked before packing and re-read with
matching hashes afterwards. This proves resource assembly, not native gameplay
or visual parity. The launcher resolves all three two-client plans; those
checks do not launch games. ID195 subsequently loaded in both three-view lamp
arms and both two-view remains arms. The workers verified shutdown and restored
the candidate's hook, preset and temporary material changes after each experiment.

The candidate includes only the independently verified BC3 remains repair;
material/sun-sentinel experiments are excluded. The 0.20e converter now uses
an identity local light offset when the donor has none, avoiding an unrelated
template lamp's `[0.01, -0.265, 0.002]` displacement. Its ambient-light shader
experiment remains disabled. The integrated converter passes 22 map tests and
four material tests. The full existing porter library suite subsequently passed
all 283 tests (zero failures or ignored tests). The merged capture hook,
including the original 0.11e reference path and optional local diffuse adapter,
compiles with `-Werror`.

Composition rejects duplicate map ownership (verified with the actual 0.11e
bundle twice), changed native registrations, undeclared dictionary edits and
conflicting shared resources. An explicit `--patch` requires the target DB hash,
the selected resource's original hash, and the replacement hash/size. A deliberately
wrong original hash was rejected before output creation. The BC3
preparer emits those guards. No precedence-based silent resource replacement
is used by this composition tool.

Reproduction after building both donor bundles and the BC3 repair:

```sh
compose-day-map-bundles --patch "$REPAIRS/texture-bundle" \
  "$BASE_DB" "$COMPOSED" "$BUNDLE_011E" "$BUNDLE_020E"
vostok-port apply --base-db "$BASE_DB" --bundle "$COMPOSED" \
  --work-dir "$CANDIDATE/packing" --output-db "$CANDIDATE/resources.db"
python tools/playable-map-pack/prepare_day_candidate.py \
  --base-package "$PREVIOUS_PACKAGE" --anomaly-package "$ANOMALY_PACKAGE" \
  --bundle "$COMPOSED" --output "$CANDIDATE"
```

The inherited presets are explicitly diagnostic until calibrated to each donor.
The candidate keeps its own clients, runtime and port 2266. The old package is
unchanged. Its 1.8 GiB intermediate packing database was deleted only after the
candidate passed resource readback; source bundles and reproducible repair
recipes remain available.

## Follow-up texture and material audit

The composed overlay contains 1,481 DDS resources. After the remains repair,
only `textures/ported/v011e/door/door_metal.dds` is still BC2: 1024-square,
nine mips, source SHA256
`4009f1b744d8f7a693178950cf5698ab980db746b1307d6727bc292a7be682d6`.
It is bound by Radar's antenna model through `tiled/door/door_metal.material`.
That material disables alpha testing. The native BC2 streaming exclusion makes
this another candidate for the same defect; its actual bound resolution has
not yet been observed.

`prepare_radar_door_bc3.py` prepares a guarded, unapplied replacement at
`/tmp/radar-door-bc3-candidate`. It preserves every RGB block, all mip sizes and
sRGB metadata; an independent Pillow decode confirmed exact RGB and identical
quarter/half alpha classifications in all nine mips. The existing remains
outputs remain byte-identical after parameterizing the transcoder's dimension
guard. Do not claim native door validation from these CPU checks.

Direct reads of the composed materials also confirm both bulb emission flags
are true in the 0.11e lamp and foam intensity is zero in all three 0.11e water
materials and the 0.20e water material. This checks that verified conversion
fixes survived composition; new-donor runtime views still govern visual parity.
