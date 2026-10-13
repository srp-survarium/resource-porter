// v0.10b geometry scattering expressed as one 0.26 alpha-blended draw.
// The old stage sends (0,1,0) as its geometry-pass sun direction; therefore
// its Rayleigh term is 0.75 * scattering regardless of the angular phase.
cbuffer DrawConstants : register(b0) { float3 s_eye_ray_corner; };
cbuffer StaticConstants : register(b1) { float4 globals[26]; };
Texture2D t_position : register(t0);
Texture2D t_rayleigh_scattering : register(t1);
Texture2D t_mie_scattering : register(t2);
SamplerState s_linear : register(s0);
SamplerState s_position : register(s1);

float4 main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET
{
    float depth=t_position.SampleLevel(s_position,uv,0).x;
    float3 ray=float3(s_eye_ray_corner.xy*(1-2*uv)/s_eye_ray_corner.z,1);
    float4 view_position=float4(ray*depth,1);
    float3 world_position=float3(dot(globals[11],view_position),dot(globals[12],view_position),dot(globals[13],view_position));
    float3 direction=world_position-globals[20].xyz;
    float distance=length(direction);
    float elevation=.5-.5*direction.y/max(distance,.00001);
    float far_distance=globals[16].y;
    float amount=saturate(min(depth,far_distance)/far_distance);
    float3 rayleigh=t_rayleigh_scattering.SampleLevel(s_linear,float2(atan(min(distance,350)*.002),elevation),0).xyz;
    float3 mie=t_mie_scattering.SampleLevel(s_linear,float2(saturate(atan(distance*.00001)),elevation),0).xyz;
    float3 additive=.75*amount*rayleigh+saturate(saturate(min(depth,far_distance)/(far_distance*.5))*mie);
    float alpha=.1*amount;
    // SRC_ALPHA / INV_SRC_ALPHA reproduces old multiply then additive passes.
    return float4(additive/max(alpha,.000001),alpha);
}
