// 011e: eight-tap mean, clamped final physical cube mip, cubic bounds and AO
// before encoded blending. Native specular remains a separate, unmodified pass.
#define CONFIG_SOURCE_011E_FALLOFF 1
#define LEGACY_SOURCE_011A 1
#define CONFIG_SOURCE_LAST_CUBE_MIP 1
#include "legacy_probe_ambient_aux.hlsl"
