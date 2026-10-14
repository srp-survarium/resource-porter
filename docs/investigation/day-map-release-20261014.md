# Day map release work

The user expanded the release to three maps during the 2026-10-14 review:

| Map | Donor | Candidate ID |
| --- | --- | --- |
| Factory day | 0.11e build 884 | 195 |
| Chemical Plant day | 0.11e build 884 | 194 |
| Factory day with anomalies | 0.20e | 196 |

The user called the first donor 0.11g; the available archive is 0.11e, which
was clarified in conversation. The current five-map package still contains
earlier donors. These candidates have not replaced it. Factory 0.10b remains
a useful reference, but is no longer one of the requested release maps.

The 0.11d note documents lighting fixes for Chemical Plant and Radar, plus
cylindrical collision fixes. The 0.11e note documents an installer hotfix.
Neither establishes a Factory beam fix. There is no Radar project in our
0.11e database. Match original views and inspect source contracts before
claiming that any remaining rendering defect is fixed.

## Verified progress

Chemical battery placement was accepted by the user. All 33 native bodies
settle stably after the body-origin correction; the two-view gallery and
checkpoints are preserved. Authored parent positions were not moved.

The 0.11e Chemical additive bundle passes resource closure and all 4,173 file
hashes. It retains source collectors, battery origins and LPV geometry. The
newer wall collision opens the reported corridor; character traversal still
needs a native test. Its source probe falloff is under binary/shader review.

Factory 0.11e lacks a server project in the archive. The build worker is
checking source scripts and earlier donor provenance before selecting any
server data. Do not substitute modern 0.26 battery positions.

The private Factory 0.20e test proves electrical damage and repeated verified
two-client checkpoints. The user reports that the anomaly works. The user is
playing that client; automated inputs and shutdown are suspended. The new
screenshots identify lighting calibration locations, using matched original
views as in earlier comparisons. Source ambient-light contracts differ from
both 0.21 and 0.26; wholesale reuse of another preset is unverified.

## Remaining rendering work

Factory 0.11e remains textures/materials match the earlier donor byte-for-byte.
Their BC2 format is excluded from native streaming pools. The prepared BC3
candidate preserves every RGB block and mip, with independently checked alpha
thresholds; actual bound texture resolution and cutout edges still need a
runtime comparison. Roughness/intensity-mask conversion and source direct-light
behavior must be checked together. The newer motor material already preserves
its source Fresnel switch, so the earlier motor patch must not be copied over.

The earlier Factory bulb emission fix is independently verified and its
converter repair is committed. Some shafts still need a source-opacity
comparison; authored transforms match the old source. The newer Factory's
lamp/cone assets are being compared before transferring that repair.

The current package has the individually verified old-Factory water foam,
motor Fresnel and bulb emission patches applied. Reversible plans and refreshed
hashes are under `validation/release-fixes-20261014`; the combined runtime
check remains pending. Those old-donor checks do not certify the new release.

Keep candidate resources separate until their source contracts, native loading,
gameplay/collision behavior and visual comparisons are checked. Preserve the
scripts, decoded evidence and compact screenshots; build the final shared
database once the three candidates are ready.
