// Unshadowed source 010b/011e point diffuse; exact native 64f2a63c... bindings.
// The four Chemical011e point lights all author diffuse influence == 1.
cbuffer SourcePointConstants : register(b0) {
    float3 light_color : packoffset(c0);
    float light_intensity : packoffset(c0.w);
    float3 eye_ray_corner : packoffset(c7);
};
cbuffer StaticGlobals : register(b1) { float4 screen_res : packoffset(c14); };
cbuffer PositionalLight : register(b2) {
    float3 light_position : packoffset(c0);
    float light_attenuation_power : packoffset(c0.w);
    float light_range : packoffset(c1.x);
};
Texture2D t_normal : register(t0);
Texture2D t_parameters : register(t1);
Texture2D t_position : register(t2);
SamplerState s_position : register(s0);
SamplerState s_normal : register(s1);
SamplerState s_accumulator : register(s2);
float4 main(float4 pixel : SV_POSITION) : SV_TARGET {
    float2 uv=pixel.xy*screen_res.zw;
    float3 position=float3((1-2*uv)*eye_ray_corner.xy/eye_ray_corner.z,1)
        *t_position.SampleLevel(s_position,uv,0).x;
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float r=dot(f,f);
    float3 normal=clamp(normalize(float3(f*sqrt(abs(1-r*.25)),r*.5-1)),-1,1);
    float f0=(uint)(saturate(t_parameters.SampleLevel(s_accumulator,uv,0).x)*127)/127.;
    float3 to_light=light_position-position;
    float3 direction=normalize(to_light),view=normalize(-position);
    float3 halfway=normalize(direction+view);
    float fresnel=f0+(1-f0)*pow(1-max(dot(view,halfway),0),5);
    float diffuse=saturate(dot(direction,normal))*(fresnel>.99?1:1-fresnel);
    diffuse*=pow(max(abs(1-saturate(dot(to_light,to_light)/(light_range*light_range))),.001),light_attenuation_power);
    return float4(sqrt(saturate(light_color*light_intensity*max(diffuse,0)*.125)),0);
}
