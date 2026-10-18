# RLS Vostok and Mamayev weather parity

Work in progress: the baseline converts and loads, but is not visually matched.

`ports/survarium-021d-two-maps-weather-to-026g.json` preserves the first
evening/rain conversion separately from the released day manifest. Both use
the same 0.21d build 2010 donor and 0.26g build 2777 target, pinned by hash.
The weather baseline retains donor lighting (`native_lighting: false`) and
the released importer's native material finishes. IDs are 167/168 for RLS
Vostok and 176/177 for Mamayev; map numbers remain 10/11.

Build with `build-survarium021-maps`, apply against the pinned stock target,
and verify, as in [the conversion documentation](survarium-021-to-026.md).
The preserved baseline contains 6,957 resources. Its applied database hash is
`731b578a8f53066693102f62d2a5450b91a1bbb3fc10b54d525319eb2e585683`.
Keep this manifest/output when introducing parity improvements: the user
explicitly requested preserving the existing 0.21-to-0.26 transformation.

## Capture procedure

`tools/render-compare/build.sh` builds a screenshot/camera-only diagnostic DLL,
a viewport helper and the original 0.21 reference server. It does not replace
shaders. `prepare_reference.py` stages an isolated original client;
`prepare_client.py` pins the executable and records diagnostic compatibility
patches. The 0.21 direct-spectator patches skip unavailable lobby callbacks and
a null statistics reset, and retain full texture mips under Wine.

`run_reference.py` accepts a prepared runtime, a project and fixed camera JSON.
For a port capture add `--build 026g --map ID --server MATCH_SERVER`.
Explicit Wine, viewport and xdotool paths are required. Only the runner's own
prefix and process groups are stopped. Capture manifests hash the runtime and
images. Camera position, direction, FOV and 1280x720 dimensions are checked.
Loading/gameplay directors differ in 0.26; readiness observes all camera calls,
then the first request selects the active director. Win32 restoration handles
windows that remain suspended after an X map/focus operation.

`make_overviews.py` proposes elevated views from authored gameplay positions.
They require visual inspection: terrain and trees can obstruct a candidate.
Keep wide panoramas and closer elevated views with multiple visible gameplay
features. These are individual perspectives, not 360-degree panoramas.

Optimize one matching screenshot at a time. Freeze the original reference;
recapture the same pose after a conversion change, then recheck earlier views.
Evening and rain are in scope; night is not.

## First observation

Mamayev evening `mamayev-close-03` has a clear view over paths, rocks, foliage
and the horizon. The baseline 0.26 image has flatter grey lighting and a pale
band on distant hills compared with 0.21. Geometry and camera framing align.
Investigate environment/shader semantics before tuning material constants.

The user screenshot `2026-10-18-06-25-27.png` depicts a conversion result to
preserve, not an original reference. Original 0.21 captures remain the parity
target. Local evidence is retained under `private/map-weather-20261018` in the
companion skhid-srp workspace, including the baseline database, build/apply/
verification logs, camera poses and capture manifests.
