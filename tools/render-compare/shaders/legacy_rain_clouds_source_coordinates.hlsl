// Opt-in Factory010 rain coordinates. Source cloud VS is time-independent,
// uploads (cos(rotation),sin(rotation)), and leaves mesh V unchanged. Native
// uploads (sin,cos) and scales V by .96. This wrapper assumes the converted
// Factory rain profile's rotation_speed=0 and source U tiling=1.
#if defined(LEGACY_WEATHER_DAY) || defined(LEGACY_WEATHER_EVENING) || defined(LEGACY_WEATHER_CHEMICAL_DAY)
#error This candidate is verified against Factory010 rain only.
#endif
#define LEGACY_CLOUD_U_OFFSET .25
#define LEGACY_CLOUD_V_SCALE (1.0/.96)
#include "legacy_rain_clouds.hlsl"
