// Native 026 composition bindings; source weather constants are exported from
// matching original captures. These auxiliary passes do not replace composition.
cbuffer CompositionConstants : register(b0) {
    float3 light_direction : packoffset(c0);
    float3 s_eye_ray_corner : packoffset(c25);
};
cbuffer SourceSunConstants : register(b3) {
    float3 source_sun_diffuse;
    float source_shadow_transparency;
    float4 source_translucency_color;
};
Texture2D t_normal : register(t0);
Texture2D t_parameters : register(t2);
Texture2D t_position : register(t3);
Texture2D t_sun_shadow_and_scattering : register(t6);
SamplerState s_position : register(s0);
SamplerState s_normal : register(s1);
SamplerState s_accumulator : register(s2);
void read_surface(float2 uv,out float3 position,out float3 normal,out float4 parameters) {
    position=float3((1-2*uv)*s_eye_ray_corner.xy/s_eye_ray_corner.z,1)
        *t_position.SampleLevel(s_position,uv,0).x;
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float radius=dot(f,f);
    normal=clamp(normalize(float3(f*sqrt(abs(1-radius*.25)),radius*.5-1)),-1,1);
    parameters=t_parameters.SampleLevel(s_accumulator,uv,0);
}
