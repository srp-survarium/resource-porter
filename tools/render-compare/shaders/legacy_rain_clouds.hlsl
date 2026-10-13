// Factory rain diagnostic: legacy modulate equation in the 0.26 cloud ABI.
// The old rain material uses UV scale 1, horizon power 2, band 0.5 and tint
// multiplier 10. These fields are not all uploaded by the 0.26 cloud stage.
cbuffer DrawConstants : register(b0) { float4 sky_clouds_parameters0; };
Texture2D sky_clouds_texture : register(t1);
SamplerState s_linear : register(s0);
float4 main(float4 position : SV_POSITION,float2 uv : TEXCOORD3) : SV_TARGET
{
    clip(uv.y>=.5?-1:1);
    float fade=min(pow(1-saturate((1-uv.y*2)/.5),2),1);
    float2 tc=float2(uv.x,uv.y*2);
    float4 sum=sky_clouds_texture.SampleLevel(s_linear,tc,2)+sky_clouds_texture.SampleLevel(s_linear,tc,1);
    sum=sky_clouds_texture.SampleLevel(s_linear,tc,0)+sum;
    float alpha=sum.w/3;
    float3 tinted=sum.xyz*sky_clouds_parameters0.xyz*10/3;
    return float4((1-lerp(alpha,0,fade))*(1-tinted)+tinted,alpha);
}
