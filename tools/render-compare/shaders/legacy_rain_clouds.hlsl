// Authored Factory cloud equations in the 0.26 cloud ABI.
// Day/evening use their source color: native upload applies an additional
// linear-space conversion and sunlight factor. Rain keeps its saved recipe.
#include "weather_profile.hlsli"
cbuffer DrawConstants : register(b0) { float4 sky_clouds_parameters0; };
Texture2D sky_clouds_texture : register(t1);
SamplerState s_linear : register(s0);
float4 main(float4 position : SV_POSITION,float2 uv : TEXCOORD3) : SV_TARGET
{
    clip(uv.y>=.5?-1:1);
    float fade=min(pow(1-saturate((1-uv.y*2)/LEGACY_CLOUD_BAND),LEGACY_CLOUD_POWER),1);
    float2 tc=float2(uv.x,uv.y*2);
    float4 sum=sky_clouds_texture.SampleLevel(s_linear,tc,2)+sky_clouds_texture.SampleLevel(s_linear,tc,1);
    sum=sky_clouds_texture.SampleLevel(s_linear,tc,0)+sum;
    float alpha=sum.w/3;
#ifdef LEGACY_CLOUD_COLOR
    float3 tinted=sum.xyz*LEGACY_CLOUD_COLOR*LEGACY_CLOUD_TINT/3;
#else
    float3 tinted=sum.xyz*sky_clouds_parameters0.xyz*LEGACY_CLOUD_TINT/3;
#endif
    float faded_alpha=lerp(alpha,0,fade);
    return float4((1-faded_alpha)*(1-tinted)+tinted,
#ifdef LEGACY_CLOUD_ALPHA_BLEND
        faded_alpha
#else
        alpha
#endif
    );
}
