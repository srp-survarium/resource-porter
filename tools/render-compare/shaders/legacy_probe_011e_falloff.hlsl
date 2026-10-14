// Isolate proven 011e attenuation from the inherited 011a probe experiment.
// Other probe contracts (index mask, AO and specular) remain unchanged.
#define CONFIG_SOURCE_011E_FALLOFF 1
#define LEGACY_SOURCE_011A 1
#define CONFIG_SOURCE_LAST_CUBE_MIP 1
#include "legacy_probe_quality3.hlsl"
