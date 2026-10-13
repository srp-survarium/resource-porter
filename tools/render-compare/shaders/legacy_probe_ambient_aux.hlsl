// Separate source ambient accumulator; does not replace native specular.
#define CONFIG_SOURCE_AMBIENT_AO 1
#define CONFIG_SOURCE_DIFFUSE_ONLY 1
#define CONFIG_ENCODED_PROBE 1
#define CONFIG_SOURCE_FALLOFF 1
#define CONFIG_SOURCE_QUALITY3 1
#include "legacy_probe_diffuse.hlsl"
