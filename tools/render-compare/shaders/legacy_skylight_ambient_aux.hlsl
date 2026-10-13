// Source 010b sky initialization with the native probe's G-buffer bindings.
// Source constants must come from the matching authored weather/reference;
// native skylight's unused legacy fields cannot supply them.
cbuffer ProbeConstants : register(b0) { float3 eye_ray_corner : packoffset(c7); };
cbuffer StaticGlobals : register(b1) {
    row_major float3x4 m_V2W : packoffset(c11);
    float4 screen_res : packoffset(c14);
};
cbuffer SourceSkylight : register(b3) {
    float4 source_sky0,source_sky1,source_sky2,source_sky3;
    float4 source_sky4,source_sky5,source_sky6,source_sky7;
};
Texture2D t_normal : register(t0);
Texture2D t_position : register(t2);
Texture2D t_legacy_ssao : register(t14);
SamplerState s_linear : register(s0);
SamplerState s_position : register(s2);
SamplerState s_normal : register(s3);
float4 main(float4 pixel : SV_POSITION) : SV_TARGET {
    float2 uv=pixel.xy*screen_res.zw;
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float r=dot(f,f);
    float3 normal=normalize(float3(f*sqrt(abs(1-r*.25)),r*.5-1));
    float3 world_normal=mul((float3x3)m_V2W,normal);
    // The source vertex shader builds this basis from the wrap direction.
    float angle=-source_sky6.z>0?acos(source_sky6.x)+3.14159265:acos(-source_sky6.x);
    float sine,cosine; sincos(angle,sine,cosine);
    float3 basis_x=normalize(float3(cosine,0,sine));
    float3 basis_z=normalize(float3(sin(-angle),0,cosine));
    float3 basis_normal=world_normal.x*basis_x+world_normal.y*float3(0,1,0)+world_normal.z*basis_z;
    float3 weights=basis_normal*basis_normal;
    float3 color=weights.x*(basis_normal.x<0?source_sky2.xyz:source_sky3.xyz)
        +weights.y*(basis_normal.y<0?float3(source_sky5.w,source_sky6.w,source_sky7.w):source_sky4.xyz)
        +weights.z*(basis_normal.z<0?float3(source_sky2.w,source_sky3.w,source_sky4.w):source_sky5.xyz);
    color+=saturate(dot(world_normal,-source_sky6.xyz))*source_sky7.xyz;
    float3 position=float3((1-2*uv)*eye_ray_corner.xy/eye_ray_corner.z,1)*t_position.SampleLevel(s_position,uv,0).x;
    float height=mul(m_V2W,float4(position,1)).y;
    float ramp=pow(saturate((height-source_sky1.x)/source_sky1.y),source_sky1.z);
    color*=ramp*(t_legacy_ssao.SampleLevel(s_linear,uv,0).x*2.5)*source_sky0.xyz;
    return sqrt(saturate(float4(color,1)*.125));
}
