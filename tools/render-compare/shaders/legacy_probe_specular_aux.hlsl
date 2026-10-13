// Source 010b high-quality probe specular, using the captured 026 probe ABI.
// Private encoded accumulator only. Clip-by-normal and shadowed authored
// probes require their source feature constants before replay is supported.
cbuffer ProbeConstants : register(b0) {
    float3 s_eye_ray_corner : packoffset(c7);
#ifdef CONFIG_PROBE_BOX
    row_major float4x4 world_to_outer_probe : packoffset(c8);
    float4 color_parameters : packoffset(c16);
#else
    float4 color_parameters : packoffset(c8);
    float3 probe_location : packoffset(c11);
#endif
};
cbuffer StaticGlobals : register(b1) {
    row_major float3x4 m_V2W : packoffset(c11);
    float4 screen_res : packoffset(c14);
    float3 eye_position : packoffset(c20);
};
cbuffer LightConstants : register(b2) { float light_range : packoffset(c1); };
Texture2D t_normal : register(t0);
Texture2D t_parameters : register(t1);
Texture2D t_position : register(t2);
TextureCube t_probe_cubemap : register(t4);
Texture2D t_legacy_ssao : register(t14);
SamplerState s_linear : register(s0);
SamplerState s_base : register(s1);
SamplerState s_position : register(s2);
SamplerState s_normal : register(s3);
SamplerState s_accumulator : register(s4);

float3 sample_source_probe(float3 direction,float mip) {
    float4 value=t_probe_cubemap.SampleLevel(s_base,direction,mip);
    return pow(abs(value.rgb/value.a),2.2);
}

float4 main(float4 pixel : SV_POSITION) : SV_TARGET {
    float2 uv=pixel.xy*screen_res.zw;
    float3 position=float3((1-2*uv)*s_eye_ray_corner.xy/s_eye_ray_corner.z,1)
        *t_position.SampleLevel(s_position,uv,0).x;
    float3 world_position=mul(m_V2W,float4(position,1));
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float radius=dot(f,f);
    float3 normal=normalize(float3(f*sqrt(abs(1-radius*.25)),radius*.5-1));
    float3 world_normal=normalize(mul((float3x3)m_V2W,normal));
    float3 view=normalize(eye_position-world_position);
    float nv=dot(view,world_normal);
    float3 reflection=reflect(view,world_normal);
    float2 parameters=floor(saturate(t_parameters.SampleLevel(s_accumulator,uv,0).xy)*127)/127;
    float gloss=parameters.y,roughness_blend=1-gloss;
    float3 half_direction=normalize(world_normal-reflection);
    float3 direction=lerp(half_direction,-reflection,pow(roughness_blend,.8));
    float3 mirror=normalize(direction);
    float3 light=lerp(mirror,normalize(world_normal+mirror),gloss);
    float nl=saturate(dot(light,world_normal));
    float3 halfway=normalize(light+view);
    float vh=dot(view,halfway),nh=dot(world_normal,halfway);
    float fresnel=parameters.x+(1-parameters.x)*pow(1-max(vh,0),5);
    float geometry=min(min(2*nh*nv/vh,2*nh*nl/vh),1);
    float roughness=pow(2/(255*roughness_blend+2)-.00778,.48);
    float distribution=1/(roughness*roughness*3.14159265);
    float specular=nl>0?(nl*fresnel)*geometry*distribution/(4*max(vh,.00001)*abs(nl*nv)):0;
    float factor=min(fresnel*saturate(specular),100);
    if (dot(direction,world_normal)<0) factor=1;
    float mip=(1-sqrt(2/(gloss+2)))*4*pow(saturate(nv),.2);
    float3 upper=sample_source_probe(direction,trunc(mip*8+1));
    float3 lower=sample_source_probe(direction,trunc(mip*8));
    float3 lighting=lerp(factor*lower,factor*upper,frac(mip*8));
    lighting=lerp(lighting,0,gloss*(gloss*gloss));
    float ao=saturate(t_legacy_ssao.SampleLevel(s_linear,uv,0).x*2.5);
    lighting=min(lighting*(ao*ao*ao*ao*ao*color_parameters.y),4);
#ifdef CONFIG_PROBE_BOX
    float3 probe_space=mul(world_to_outer_probe,float4(world_position,1)).xyz;
    float3 falloff=1-exp(-6*sqrt(saturate(1-abs(probe_space))));
    float attenuation=pow(falloff.x*falloff.y*falloff.z,2);
#else
    float distance_ratio=length(probe_location-world_position)/(light_range/.98*.95);
    float power=distance_ratio*distance_ratio;
    power*=power;power*=power;
    float attenuation=exp(-8*power);
#endif
    return float4(sqrt(saturate(lighting*.125)),attenuation);
}
