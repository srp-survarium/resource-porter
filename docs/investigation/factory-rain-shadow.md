# Factory 0.10b rain shadow camera

`SRP_LEGACY_FACTORY_RAIN_SHADOW=1` restores the original rain camera in the
pinned 0.26g executable. The adapter is off by default and is specific to the
Factory 0.10b recipe. It validates all instructions before patching them.

Original `stage_pre_rain.cpp` uses the complete view direction times 32 and a
64-unit orthographic span. Native 0.26 halves the forward offset, drops its
vertical component and uses a 128-unit span. The adapter corrects these three
operations, preserving the 128-square depth target and far-plane calculation.

Five matched views in `026g/pipeline-factory-rain-shadow-source` reproduce the
original world-to-shadow matrix within 5.97e-8. The comparison first converts
the original view-to-shadow matrix to world space using captured `m_V`.

| View | Previous depth MAE | Corrected depth MAE |
| --- | ---: | ---: |
| Interior | .017265 | .007982 |
| North yard | .191517 | .183656 |
| South yard | .106634 | .093031 |
| Upper factory | .270863 | .258802 |
| Streetlamp | .047376 | .001093 |

Depth values are normalized R16 values. This proves the camera correction,
not full rain-shadow parity: native draws additional casters in regions where
the original depth map is clear. Caster screen-size selection uses different
units in the two engines and needs a separate source-derived correction.
The original occlusion test is already option-gated; making the native test
unconditional would not restore source behavior.

`reproduction/factory-rain-shadow-20261013` preserves the exact loaded DLL,
source, launcher, poses, matrix/depth results, original source and native
assembly, plus hashes of all capture artifacts. These paths are relative to
`.work/render-compare` in the main resource-porter repository. The DLL built
with warnings treated as errors; all five captures completed and the private
client/server stopped. Rain geometry, motion and encoded-light accumulation
remain separate unfinished work.
