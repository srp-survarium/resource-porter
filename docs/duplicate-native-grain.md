# Duplicate native grain in the source postprocess recipe

The original 0.11a/0.10b postprocess adds intentional image grain before final gamma. The adapted source recipe retained it, but native 0.26 subsequently ran its own grain-enabled aberration pass. A stationary Chemical Plant capture proves the active native permutation: 2,528-byte DXBC, digest `14918839a50dac2bf3dd66124a690f12`, `image_grain_parameters.z = 0.0533333346`. Its final `mad_sat` adds positive animated noise after the source postprocess. This is shared renderer behavior, not a Chemical Plant material defect.

`native_grain.py` extracts all eight shipped aberration permutations and verifies four counterparts that differ only in the grain feature bit. The opt-in `SRP_LEGACY_GRAINLESS_ABERRATION_DIR` selects the corresponding original native grain-free bytecode. It requires the paired source post tail, grain texture and gamma override, preserving the intentional source grain. No archive or authored scene settings change.

The native reflection layouts are **not interchangeable without adaptation**. Removing grain removes `s_nofilter` at slot 0 and shifts retained samplers down one slot. The generator asserts identical input signatures, unchanged retained SRV/CB bindings and constant offsets, and records the sampler mapping by name. The hook temporarily maps the original sampler bindings for the draw and restores them afterwards. All four pairs are checked; the actual runtime draw uses the expected 2,036-byte counterpart and linear-clamp sampler at slot 0. Observer kind 22 saves the active shader, constants, samplers, inputs and output. The control `.grain-fix.json` records successful replacements.

## Captures and limits

Durable evidence: `.work/chemical-plant-011a/comparison` in the main resource-porter repository. `026g/native-aberration-contract/poses` holds the active native contract and material audit. `026g/grain-baseline-sequence/sequence` and `026g/grainless-aberration/sequence` each hold 12 stationary sky and 12 stationary geometry frames with UI off and identical camera requests. `grain-temporal-comparison.json` records frame hashes and measurements from `audit_temporal_noise.py` at step 8.

| Region | Baseline temporal RMS after frame-mean removal | Corrected | Reduction |
| --- | ---: | ---: | ---: |
| Sky | 0.014495 | 0.006105 | 57.9% |
| Near moss | 0.034286 | 0.032103 | 6.4% |
| Distant building | 0.017848 | 0.012951 | 27.4% |

Frozen original sky measures 0.005923 by the same method. Raw temporal RMS remains affected by brightness/cloud changes: sky 0.019298 → 0.014282. Independent cloud, foliage, moss and lighting animation phases are not synchronized. The correction removes a proven extra noise source; it does not establish that all stationary shimmer is fixed. Missing cloud appearance, glossy materials, localized collision reports and remaining near-surface variation stay open.

The C hook builds with warnings treated as errors. The extraction audit checks all four binding contracts. Runtime verifies the active corrected bytecode and sampler mapping across the captured sequences; other feature combinations have structural checks but no separate runtime scenes yet.

One initial baseline attempt stalled after its material capture while a separate Wine compiler prefix initialized. The read-only activity trace showed `application_active=0`, `application_activated=0`, `activity_gate=1`, `activity_allows_tick=0`, with equal stationary logic/render counters. Its logs are preserved under `native-aberration-contract`; that failed sequence is excluded. A fresh baseline without concurrent compiler activity completed all 24 frames.
