#include "legacy_sun_aux_common.hlsl"
// Separate ONE/ONE draw into R10G10B10A2. The source geometry/stencil mask
// must be supplied by the caller. Native visibility is a remaining input
// difference: this adapter does not restore original cascade filtering.
float4 main(float4 pixel : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET {
    float3 position,normal; float4 parameters;
    read_surface(uv,position,normal,parameters);
    float3 view=normalize(-position),light=-light_direction;
    float3 halfway=normalize(light+view);
    float f0=(uint)(saturate(parameters.x)*127)/127.;
    float fresnel=f0+(1-f0)*pow(1-max(dot(view,halfway),0),5);
    float diffuse=saturate(dot(light,normal))*(fresnel>.99?1:1-fresnel);
    float visibility=t_sun_shadow_and_scattering.SampleLevel(s_accumulator,uv,0).x;
    // Source sun multiplies visibility AFTER encoding; spot lights do not.
    return float4(sqrt(saturate(source_sun_diffuse*max(diffuse,0)*.125))
        *(visibility+source_shadow_transparency),0);
}
