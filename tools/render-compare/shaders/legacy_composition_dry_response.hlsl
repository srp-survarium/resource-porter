// The old resolve has no rain multiplier. Isolate that difference while the
// weather, native material/specular response and encoded diffuse sum stay fixed.
#define CONFIG_ENCODED_ADDITION 1
#define CONFIG_NO_RAIN_DARKENING 1
#include "legacy_composition_control.hlsl"
