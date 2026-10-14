// Original 020e unlit cloud PS equations (DXBC 98013dc55c5f), with 026's
// texture slot and varying ABI. All weather values come from the authored
// environment constants; the paired CPU adapter restores rotation convention.
cbuffer CloudConstants : register(b0)
{
    float4 sky_clouds_parameters0;
    float4 sky_clouds_parameters1;
    float4 sky_clouds_parameters3;
    float4 sky_clouds_parameters4;
};
Texture2D sky_clouds_texture : register(t1);
SamplerState s_linear : register(s0);
struct CloudInput
{
    float4 position : SV_POSITION;
    float3 world_position : TEXCOORD0;
    float3 object_position : TEXCOORD1;
    float3 view_direction : TEXCOORD2;
    float2 uv : TEXCOORD3;
    float3 normal : TEXCOORD4;
    float3 tangent : TEXCOORD5;
    float3 binormal : TEXCOORD6;
};
float4 main(CloudInput input) : SV_TARGET
{
    if (input.uv.y>=.5208333134651184)
        return float4(sky_clouds_parameters3.xyz,1);
    float fade=min(pow(1-saturate((1-input.uv.y*2)/sky_clouds_parameters1.y),
                       sky_clouds_parameters1.x),1);
    float2 tc=float2(input.uv.x,input.uv.y*2);
    float4 sum=sky_clouds_texture.SampleLevel(s_linear,tc,2)
              +sky_clouds_texture.SampleLevel(s_linear,tc,1)
              +sky_clouds_texture.SampleLevel(s_linear,tc,0);
    float3 color=sum.xyz*sky_clouds_parameters0.xyz/3;
    return float4(lerp(color,sky_clouds_parameters3.xyz,fade),sum.w/3);
}
