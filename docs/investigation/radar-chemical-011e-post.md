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
sample; its render-target/sRGB conversion has not been captured, so this work
does not change the target's paired final gamma shader.

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

Native post-only, probe-only and combined comparisons are pending. The source
references stay frozen; no exposure multiplier is fitted to screenshots and
the shared package remains unchanged during preparation.
