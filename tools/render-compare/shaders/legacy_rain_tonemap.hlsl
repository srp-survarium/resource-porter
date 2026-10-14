// Isolate v0.10b rain exposure/filmic response within the active 0.26 post ABI.
// Bloom inputs and vignette stay at the captured target contract.
#include "weather_profile.hlsli"
cbuffer PostConstants : register(b0)
{
    float4 image_grain_parameters : packoffset(c11);
    float4 bloom_parameters : packoffset(c18);
};
Texture2D t_frame_color : register(t0);
Texture3D t_color_grading_lut : register(t1);
Texture2D t_frame_luminance : register(t2);
#ifndef CONFIG_ORIGINAL_POST_TAIL
Texture2D t_light_scattering_result : register(t3);
#endif
Texture2D t_bloom_combine : register(t4);
#ifdef CONFIG_ORIGINAL_POST_TAIL
Texture2D t_legacy_grain : register(t12);
#endif
SamplerState s_linear : register(s0);
SamplerState s_accumulator : register(s1);
SamplerState s_material1 : register(s2);

float4 main(float4 position : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET
{
    float3 frame=t_frame_color.SampleLevel(s_material1,uv,0).xyz;
    float3 bloom=t_bloom_combine.SampleLevel(s_material1,uv,0).xyz;
    float3 color=lerp(frame,bloom,bloom_parameters.x);
#ifndef CONFIG_ORIGINAL_POST_TAIL
    // All 68 frozen original post draws omit the god-ray texture binding.
    color+=t_light_scattering_result.Sample(s_linear,uv).xyz;
#endif
#if defined(CONFIG_ORIGINAL_POST_TAIL) && !defined(LEGACY_SOURCE_011E_POST)
    float level=dot(float3(.3,.7,.2),color);
    if (level<.03) color=lerp(color,dot(float3(.2125,.7154,.0721),color),pow(1-saturate(level/.03),2));
#endif
    float average=clamp(t_frame_luminance.Sample(s_accumulator,float2(.5,.5)).x,.09,.4);
#ifdef LEGACY_SOURCE_011E_POST
    // Original 011e complex_post_process_blend.ps, quality2/3 program
    // 3eefb58778b7: exposure, shoulder clamp/addback, then dark desaturation.
    float key=1.03-2/(20*(log2(average+1)*.30103+2));
    float3 x=max(color*pow(key/average,.2)-LEGACY_FILMIC_TOE,0)+LEGACY_FILMIC_TOE;
    color=(x*(LEGACY_FILMIC_LINEAR*x+LEGACY_FILMIC_ANGLE))/(x*(LEGACY_FILMIC_STRENGTH*x+LEGACY_FILMIC_NUMERATOR)+LEGACY_FILMIC_DENOMINATOR);
    float level=dot(float3(.3,.7,.2),color);
    float3 dark_tint=level*float3(.8,.8,1.4);
    color=pow(lerp(dark_tint,color,saturate(level*LEGACY_DARK_DESATURATION+.21)),2.2);
#else
    float3 x=max(color*pow(.25/average,.2)-LEGACY_FILMIC_TOE,0);
    color=pow((x*(LEGACY_FILMIC_LINEAR*x+LEGACY_FILMIC_ANGLE))/(x*(LEGACY_FILMIC_STRENGTH*x+LEGACY_FILMIC_NUMERATOR)+LEGACY_FILMIC_DENOMINATOR),2.2);
#endif
#ifdef CONFIG_ORIGINAL_POST_TAIL
    uint width,height; t_frame_color.GetDimensions(width,height);
    int2 noise_pixel=int2(floor((uv+image_grain_parameters.xy*2-1)*float2(width,height))) & 63;
    color=saturate(color+t_legacy_grain.Load(int3(noise_pixel,0)).rgb*.08*pow(color,.6));
    float3 encoded=pow(abs(color),1/2.2);
#else
    float3 encoded=1.055*pow(abs(color),1/2.4)-.055;
#endif
    float3 graded=t_color_grading_lut.SampleLevel(s_material1,encoded*(15./16)+(.5/16),0).xyz;
#ifdef CONFIG_ORIGINAL_POST_TAIL
    color=pow(abs(graded),2.2);
#else
    color=pow((abs(graded)+.055)/1.055,2.4);
#endif
    if (image_grain_parameters.w>.1)
#ifdef LEGACY_SOURCE_011E_POST
        color*=min(pow(abs(1-dot(uv-.5,uv-.5)),(image_grain_parameters.w+1)*.1),1);
#else
        color*=min(pow(abs(1-dot(uv-.5,uv-.5)),image_grain_parameters.w+1),1);
#endif
    return float4(color,0);
}
