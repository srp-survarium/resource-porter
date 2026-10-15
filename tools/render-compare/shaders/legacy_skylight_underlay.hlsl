// 0.11e skylight stage (skylight.vs/.ps, DXBC checked against the 0.11e
// archive): six per-axis colours in the sun-wrapped basis, sun term, height
// ramp and skylight colour, written as E = sqrt(L/8) before the probes lerp
// over it. Drawn by the hook into the diffuse accumulator before the first
// probe of a frame; AO is applied later by the SSAO-stage decode.
#define CONFIG_SKYLIGHT_NO_AO 1
#include "legacy_skylight_ambient_aux.hlsl"
