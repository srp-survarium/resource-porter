# Radar 0.11e selective cone-opacity comparison

The existing source-opacity adapter improves the sampled Radar lamp shafts
without moving any geometry. It removes the broad native triangular halo in
the east hangar, keeps the office beam below its bulb, and reduces the outdoor
beam's opacity. The original 0.10b crops support this narrower profile; actual
0.11e reference captures are queued with the donor-reference worker. Do not
claim completed 0.11e lighting parity from this experiment.

Two target runs loaded map 195 from `/tmp/three-day-map-candidate/resources.db`
(SHA-256 `241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d`).
Both used the same compiled hook and inherited diagnostic weather recipe,
UI off, 1280×720, and three mode-2 screenshots. Only the cone pixel shader and
its paired original falloff textures changed between arms. The replacement
report records one successful shader creation; all camera acknowledgments were
error-free. There were no raw buffer dumps or database writes. Clients and
server stopped; the candidate's original hook/preset were restored byte-exact.

Evidence is in
`/tmp/resource-porter-factory-day-lights/.work/radar-011e-cone-ab`:

- `baseline/` and `source/`: six BMPs, PNGs, camera metadata, effective presets,
  original preset/hook copies, and override reports.
- `comparison/`: three source/native/replacement crop strips, unscaled PNG
  differences, and `metrics.json` with exact sample rectangles.
- `poses.json`, `run_ab.py`, `compare_captures.py`: bounded experiment replay.
- `activation/`: prepared preset overlay, baseline preset and guarded plan.

Threeway PNG columns are original 0.10b, native 0.26, and 0.26 with source cone
opacity. Original crosshairs remain visible. They constrain beam shape, not
0.11e scene brightness: 0.11e revises probes and post-processing, while the
diagnostic target recipe still inherits older weather constants.

| Same-recipe target region | Native mean luma | Source-opacity mean luma |
|---|---:|---:|
| East hangar beam shoulder | 0.17994 | 0.05862 |
| East hangar below bulb | 0.19973 | 0.18300 |
| East hangar static ceiling | 0.02690 | 0.02658 |
| West office beam shoulder | 0.16598 | 0.05927 |
| West office below bulb | 0.20936 | 0.13602 |
| West office static wall | 0.04068 | 0.04100 |
| South exterior beam | 0.32129 | 0.25786 |
| South exterior static brick | 0.03501 | 0.03498 |

These are display-encoded A/B measurements, not source-parity scores. Frames
have different animation times. The unchanged exterior placement and retained
below-bulb office shaft are controls against a global positional correction.
The material/atmosphere problems visible in the dark target backgrounds remain
separate; this change does not brighten them.

`tools/render-compare/prepare_radar_011e_cones.py` prepares an ID195-only overlay
without touching a package. It requires the exact tested hook, shader and both
falloff DDS hashes. Inputs are baseline preset, tested hook, shader, falloff
directory and a new output directory. The plan declares expected parent hashes
and requires all four new asset paths to be absent before installation.
Apply assets first and preset last; reverse only while all resulting hashes
still match. No activation was performed after the experiment.

The tested parent preset hash is
`ad2d649b74a83b0117cc15a7bfce2c776aabdcc5d4a5fe16ab7afc5efbbe63b7`.
The old frozen hook lacks the cone adapter, so the overlay references the tested
new hook at a separate asset path and leaves the old hook file intact. If another
worker updates the preset or hook, the prepared plan must not overwrite it;
merge the cone support into that newer baseline and retain the same-arm controls.
Chemical Plant and Radar 0.20e presets are outside this activation scope.

The obsolete unvalidated 0.10b `build_local.py` cone proposal was removed from
the worktree and retained as `obsolete-010b-recipe-candidate.patch` in the
experiment directory. It was never a released fix.
