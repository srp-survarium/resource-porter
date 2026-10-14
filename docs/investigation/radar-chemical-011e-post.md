# Original 0.11e post-processing audit

Matched original/target captures confirm broad shadow darkness on both maps.
The inherited presets use 0.10b Radar and 0.11a Chemical equations. The original
0.11e archive proves differences in both equations and authored parameters.

`audit_011e_post.py SOURCE_DB LISTING OUTPUT` extracts all nine distinct post
programs and both authored post materials without starting the original client.
The reviewed quality2/3 DXBC is
`3eefb58778b7932d90b8f4af638dcc077ab6b59a9076bfa45e3e44de55f976af`.
Its SHDR word offsets establish:

| Words | Source equation | Inherited preset |
| --- | --- | --- |
| 245–333 | Clamp average to [.09,.4]; key=1.03−2/(20×(log2(average+1)×.30103+2)); exposure=(key/average)^.2 | exposure=(.25/average)^.2 |
| 338–418 | x=max(exposed−shoulder×.01,0)+shoulder×.01; authored rational tone curve | No shoulder addback; older coefficients |
| 425–476 | After tone curve, blend luma×(.8,.8,1.4) toward color by saturate(luma×dark_limit+.21) | Different desaturation before tone mapping |
| 648–720 | Vignette exponent=(power+1)×.1 | exponent=power+1 |

The exposure, shoulder and desaturation operations also occur in all other
eight post variants. The vignette is quality-dependent; the adapter retains
the existing target vignette enable switch. Original grain and grading use
the existing adapter's 2.2 conversions. Original `present.ps` is a passthrough
sample, but the preceding copy/sharpen pass explicitly applies final gamma
before it. Pinned shader and executable evidence confirms a plain UNORM
backbuffer: [retain one final gamma conversion](radar-011e-final-gamma.md).

The post constant upload order in reconstructed `stage_postprocess.cpp`
agrees with reflection: shoulder/linear/angle/toe strength at c17,
numerator/denominator/white/dark desaturation at c18. Authored values are:

| Parameter | Radar 0.11e | Chemical 0.11e |
| --- | ---: | ---: |
| Shoulder | .08 | .04 |
| Linear strength | 6.3 | 6.5 |
| Linear angle | .3 | .3 |
| Toe strength | 6 | 6 |
| Toe numerator | 2 | 2 |
| Toe denominator | .04 | .02 |
| Dark desaturation limit | 3.5 | 3.3 |
| Grain scale | .08 | .08 |
| Vignette power | 1 | 1 |

`legacy_011e_post_tail.hlsl` prepares the optional correction using the current
target constant/texture layout. Define `LEGACY_WEATHER_CHEMICAL_DAY=1` for
Chemical; otherwise it uses Radar. It retains the existing bloom/DOF inputs,
grain texture, grading LUT and final output conversion. Thus it is a bounded
post-tail correction, not proof that every source producer has been ported.

## Native comparison

Both post variants compile with `/WX`. Three Chemical views and seven Radar
views were captured in post-only, probe-only and combined arms. Source
references stayed frozen. No exposure multiplier was fitted to screenshots;
the shared package is unchanged. Post-only improves all ten views.

| Sample | Baseline mean RGB error | Post-only | Probe-only | Combined |
| --- | ---: | ---: | ---: | ---: |
| Radar, seven views | .17477 | .09633 | .17417 | .09543 |
| Chemical hall | .11327 | .06731 | .11627 | .07012 |
| Chemical west interior | .11392 | .07435 | .11910 | .08090 |
| Chemical yard | .13640 | .08850 | .15840 | .10590 |

These display-RGB grid metrics include unsynchronized grain and vegetation.
Exact actual camera position, direction, FOV and dimensions match; contact
sheets were also visually checked. Chemical's rich hall captures confirm the
actual post/probe shader bytes. Radar uses screenshot-only captures, pinned
input hashes and successful shader-replacement receipts. All comparison
clients and owned Wine processes exited before the next worker's handoff.

Radar's probe arm also replaces the inherited 0.10b gather/mip assumptions;
it is not an attenuation-only experiment. Neither map's probe-only result
justifies promoting it independently. Post-only is the supported correction,
but broad illumination remains deficient. The original 0.11e skylight shader
is byte-identical to the known earlier shader, and the current preset omits
its source height/cube/wrap contribution. That contract is the next check.

Scripts, frozen inputs, captures, contact sheet, hashes and shutdown proof:
`/tmp/radar-011e-lighting-review/`; isolated comparison package:
`/tmp/radar-011e-lighting-candidate/`. The two compiled post shaders and build
receipts are in the Chemical worker's `.work/reference-011e/post-tail/`.
The durable checkpoint is
`resource-porter/.work/radar-011e-post-20261014/` (109 hash-verified files,
93 MB), including the exact candidate shader builds. All 29 original and
29 baseline frames are separately frozen at
`.work/render-compare/reproduction/reference-011e-20261014/` (222 verified
files). Neither checkpoint includes a duplicate resource database.

Independent review checked the source instruction ranges, both coefficient sets
and upload order without finding a shader mismatch. All eight existing
0.10b/0.11a profile/tail combinations preprocess byte-identically to the parent
of the correction. The review corrected the audit listing command to
`resources-db list SOURCE_DB`; the paths-only porter listing is insufficient.
Seven compact review files are preserved under the durable checkpoint
`independent-review/`, with verified hashes.
