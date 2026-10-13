#include "legacy_sun_aux_common.hlsl"
Texture2D t_legacy_sun_projection : register(t13);
// Separate additive draw after sun. Caller must preserve original translucent
// material coverage (source stencil bit4), not only test a nonzero parameter.
float4 main(float4 pixel : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET {
    float3 position,normal; float4 parameters;
    read_surface(uv,position,normal,parameters);
    float translucency=pow(floor(pow(saturate(parameters.z),1/2.2)*31)/31,2.2);
    if (translucency<=0) return 0;
    float projected=t_legacy_sun_projection.SampleLevel(s_position,uv,0).x;
    // Original helper GREEN stores visibility; native helper RED is visibility.
    float visibility=t_sun_shadow_and_scattering.SampleLevel(s_accumulator,uv,0).x;
    float attenuation=pow(1-saturate((projected-visibility)/(translucency*.035)),10);
    float back=dot(normal,light_direction);
    float forward=saturate(dot(-normalize(position),light_direction));
    float3 color=source_translucency_color.xyz;
    float3 result=translucency*.1+color*back*attenuation*translucency*1.5
        +translucency*back*.1*color+color*forward*forward*forward*translucency*attenuation*2;
    return float4(sqrt(saturate(result*.125)),0);
}
