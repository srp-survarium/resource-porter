// Authored Factory post/sky settings; original post constants confirmed by capture.
#if defined(LEGACY_SOURCE_011E_POST)
// Shipped 0.11e level_03_day / level_04_day materials. The post shader
// multiplies authored shoulder strength by .01 before its clamp/addback.
#define LEGACY_FILMIC_ANGLE .3
#define LEGACY_FILMIC_STRENGTH 6
#define LEGACY_FILMIC_NUMERATOR 2
#if defined(LEGACY_WEATHER_CHEMICAL_DAY)
#define LEGACY_FILMIC_TOE .0004
#define LEGACY_FILMIC_LINEAR 6.5
#define LEGACY_FILMIC_DENOMINATOR .02
#define LEGACY_DARK_DESATURATION 3.3
#else
#define LEGACY_FILMIC_TOE .0008
#define LEGACY_FILMIC_LINEAR 6.3
#define LEGACY_FILMIC_DENOMINATOR .04
#define LEGACY_DARK_DESATURATION 3.5
#endif
#elif defined(LEGACY_WEATHER_CHEMICAL_DAY)
#define LEGACY_CLOUD_U_OFFSET .25
#define LEGACY_CLOUD_V_SCALE (1.0/.96)
// v0.11a level_04_day.material, confirmed against shipped post shader source.
#define LEGACY_FILMIC_TOE .0004
#define LEGACY_FILMIC_LINEAR 6.8
#define LEGACY_FILMIC_ANGLE .5
#define LEGACY_FILMIC_STRENGTH 6.2
#define LEGACY_FILMIC_NUMERATOR 1.7
#define LEGACY_FILMIC_DENOMINATOR .06
#define LEGACY_CLOUD_BAND .15
#define LEGACY_CLOUD_POWER 10
#define LEGACY_CLOUD_TINT 1
#define LEGACY_CLOUD_COLOR pow(float3(.9860015,.9614621,.8960299),2.2)
#define LEGACY_CLOUD_ALPHA_BLEND
#elif defined(LEGACY_WEATHER_DAY)
#define LEGACY_FILMIC_TOE .003
#define LEGACY_FILMIC_LINEAR 6.2
#define LEGACY_FILMIC_STRENGTH 5.9
#define LEGACY_FILMIC_NUMERATOR 1.5
#define LEGACY_FILMIC_DENOMINATOR .1
#define LEGACY_CLOUD_BAND .15
#define LEGACY_CLOUD_POWER 10
#define LEGACY_CLOUD_TINT 1
#define LEGACY_CLOUD_COLOR pow(float3(.9860015,.9614621,.8960299),2.2)
#define LEGACY_CLOUD_ALPHA_BLEND
#elif defined(LEGACY_WEATHER_EVENING)
#define LEGACY_FILMIC_TOE .002
#define LEGACY_FILMIC_LINEAR 7
#define LEGACY_FILMIC_STRENGTH 6.2
#define LEGACY_FILMIC_NUMERATOR 2
#define LEGACY_FILMIC_DENOMINATOR .01
#define LEGACY_CLOUD_BAND .15
#define LEGACY_CLOUD_POWER 10
#define LEGACY_CLOUD_TINT 2.5
#define LEGACY_CLOUD_COLOR pow(float3(.9619594,.76427114,.5775735),2.2)
#define LEGACY_CLOUD_ALPHA_BLEND
#else
#define LEGACY_FILMIC_TOE .0015
#define LEGACY_FILMIC_LINEAR 6.2
#define LEGACY_FILMIC_STRENGTH 5.9
#define LEGACY_FILMIC_NUMERATOR 1.5
#define LEGACY_FILMIC_DENOMINATOR .04
#define LEGACY_CLOUD_BAND .5
#define LEGACY_CLOUD_POWER 2
#define LEGACY_CLOUD_TINT 10
#endif

#ifndef LEGACY_FILMIC_ANGLE
#define LEGACY_FILMIC_ANGLE .2
#endif

#if defined(LEGACY_SOURCE_011E_CLOUDS)
// 0.11e level_03/level_04 day: authored cloud rotation 0, uploaded as
// (cos, sin) by the source but (sin, cos) by 0.26, which is the quarter turn
// verified on Chemical sky views; the same upload applies to Radar. Cloud
// colour multiplier: Chemical 3, Radar 1 (post_process materials).
#undef LEGACY_CLOUD_U_OFFSET
#undef LEGACY_CLOUD_V_SCALE
#undef LEGACY_CLOUD_TINT
#define LEGACY_CLOUD_U_OFFSET .25
#define LEGACY_CLOUD_V_SCALE (1.0/.96)
#if defined(LEGACY_WEATHER_CHEMICAL_DAY)
#define LEGACY_CLOUD_TINT 3
#else
#define LEGACY_CLOUD_TINT 1
#endif
#endif
