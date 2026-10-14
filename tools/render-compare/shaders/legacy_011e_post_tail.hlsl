// Optional original 0.11e post-tail equations in the existing target ABI.
// Compile with LEGACY_WEATHER_CHEMICAL_DAY for level_04; otherwise level_03.
// Bloom/DOF inputs and the target final output conversion remain unchanged.
#define LEGACY_SOURCE_011E_POST
#define CONFIG_ORIGINAL_POST_TAIL
#include "legacy_rain_tonemap.hlsl"
