// Source 010b shadowed-spot diffuse on the verified 026g 33204e90... bindings.
// Auxiliary replay only: native specular and native render targets stay live.
cbuffer SourceSpotConstants : register(b0) {
    float3 light_color : packoffset(c0); // Already pow(round(authored*255)/255,2.2).
    float light_intensity : packoffset(c0.w);
    row_major float4x4 view_to_light_matrix : packoffset(c2);
    float shadow_z_bias : packoffset(c6.x);
    float shadow_transparency : packoffset(c6.y);
    float shadow_map_size : packoffset(c6.z);
    float3 light_direction : packoffset(c7);
    float penumbra_cosine : packoffset(c7.w);
    float umbra_cosine : packoffset(c8.x);
    float inverse_angle_difference : packoffset(c8.y);
    float light_spot_falloff : packoffset(c8.z);
    float3 eye_ray_corner : packoffset(c9);
};
cbuffer StaticGlobals : register(b1) {
    row_major float3x4 m_V2W : packoffset(c11);
    float4 screen_res : packoffset(c14);
};
cbuffer PositionalLight : register(b2) {
    float3 light_position : packoffset(c0);
    float light_attenuation_power : packoffset(c0.w);
    float light_range : packoffset(c1.x);
};
Texture2D t_normal : register(t0);
Texture2D t_parameters : register(t1);
Texture2D t_position : register(t2);
Texture2D shadowmap_texture : register(t3);
SamplerState s_position : register(s0);
SamplerState s_normal : register(s1);
SamplerState s_accumulator : register(s2);
SamplerComparisonState s_shmap_local : register(s3);
static const float2 poisson_disk[16] = {
    float2(-.94201624,-.39906216),float2(.94558609,-.76890725),
    float2(-.094184101,-.92938870),float2(.34495938,.29387760),
    float2(-.91588581,.45771432),float2(-.81544232,-.87912464),
    float2(-.38277543,.27676845),float2(.97484398,.75648379),
    float2(.44323325,-.97511554),float2(.53742981,-.47373420),
    float2(-.26496911,-.41893023),float2(.79197514,.19090188),
    float2(-.24188840,.99706507),float2(-.81409955,.91437590),
    float2(.19984126,.78641367),float2(.14383161,-.14100790)
};
float4 main(float4 pixel : SV_POSITION) : SV_TARGET {
    float2 uv=pixel.xy*screen_res.zw;
    float3 position=float3((1-2*uv)*eye_ray_corner.xy/eye_ray_corner.z,1)
        *t_position.SampleLevel(s_position,uv,0).x;
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float r=dot(f,f);
    float3 normal=clamp(normalize(float3(f*sqrt(abs(1-r*.25)),r*.5-1)),-1,1);
    // Reproduce the source 7-bit field from the supplied target material value.
    // This does not recover source values already lost in target G-buffer packing.
    float f0=(uint)(saturate(t_parameters.SampleLevel(s_accumulator,uv,0).x)*127)/127.;
    float3 to_light=light_position-position;
    float distance2=dot(to_light,to_light);
    float3 direction=normalize(to_light),view=normalize(-position);
    float3 halfway=normalize(direction+view);
    float fresnel=f0+(1-f0)*pow(1-max(dot(view,halfway),0),5);
    float diffuse=saturate(dot(direction,normal))*(fresnel>.99?1:1-fresnel);
    diffuse*=pow(max(abs(1-saturate(distance2/(light_range*light_range))),.001),light_attenuation_power);
    float cosine=dot(-direction,light_direction);
    diffuse*=cosine>=umbra_cosine?1:pow(max(cosine-penumbra_cosine,0)*abs(inverse_angle_difference),light_spot_falloff);
    float3 offset=normal*12/shadow_map_size;
    float4 projected=mul(view_to_light_matrix,float4(mul(m_V2W,float4(position+offset,1)),1));
    float3 tc=projected.xyz/projected.w;
    tc.xy=tc.xy*float2(.5,-.5)+.5;
    tc.z-=shadow_z_bias;
    float shadow=0;
    [unroll] for (int i=0;i<16;++i)
        shadow+=shadowmap_texture.SampleCmpLevelZero(s_shmap_local,tc.xy+poisson_disk[i]/shadow_map_size,tc.z).x;
    diffuse*=saturate(shadow/16+shadow_transparency);
    // All matched authored Factory lights have diffuse influence == 1.
    return float4(sqrt(saturate(light_color*light_intensity*max(diffuse,0)*.125)),0);
}
