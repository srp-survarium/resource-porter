// Isolate source diffuse-cube sampling/decoding within the captured 0.26 ABI.
// Native specular, material response and probe falloff are retained.
#ifdef CONFIG_ENCODED_PROBE
#define color_parameters encoded_color_parameters
#endif
cbuffer ProbeConstants : register(b0) {
    float3 s_eye_ray_corner : packoffset(c7);
#ifdef CONFIG_PROBE_BOX
    row_major float4x4 world_to_outer_probe : packoffset(c8);
    float4 color_parameters : packoffset(c16);
    float3 outer_cube_side_sizes : packoffset(c17);
    float3 inner_cube_side_sizes : packoffset(c18);
    int num_mips : packoffset(c20);
#else
    float4 color_parameters : packoffset(c8);
    float3 probe_location : packoffset(c11);
    float inner_radius : packoffset(c11.w);
    int num_mips : packoffset(c12);
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
Texture2D t_ssao_accumulator : register(t3);
TextureCube t_probe_cubemap : register(t4);
TextureCube t_probe_cubemap_diffuse : register(t5);
Texture2D t_probe_brdf : register(t6);
#ifdef CONFIG_SOURCE_AMBIENT_AO
Texture2D t_legacy_ssao : register(t14);
#endif
SamplerState s_linear : register(s0);
SamplerState s_base : register(s1);
SamplerState s_position : register(s2);
SamplerState s_normal : register(s3);
SamplerState s_accumulator : register(s4);
SamplerState s_material1 : register(s5);
struct Result { float4 diffuse : SV_Target0;
#ifndef CONFIG_SOURCE_DIFFUSE_ONLY
    float4 specular : SV_Target1;
#endif
};
float3 native_sample(float3 direction,float mip) {
    float4 value=t_probe_cubemap.SampleLevel(s_base,direction,mip);
    return value.rgb*value.rgb/(value.a*value.a)*.25;
}
float3 source_diffuse_sample(float3 direction) {
    float4 encoded=t_probe_cubemap_diffuse.SampleLevel(s_material1,direction,1);
    return pow(abs(encoded.rgb/encoded.a),2.2);
}
#ifdef CONFIG_SOURCE_QUALITY3
static const float3 source_cones[8]={
    float3(.99725014,-.029667195,.06791174),
    float3(.948293,-.029853212,-.31598923),
    float3(.89564174,-.095602594,.43438),
    float3(.8242554,.55417657,.11615242),
    float3(.7863557,-.6177587,.004354838),
    float3(.7716971,.4068773,-.48880917),
    float3(.75259656,-.34534776,-.56065434),
    float3(.7230467,.28388765,.6297708)
};
#endif
Result main(float4 pixel : SV_POSITION) {
    float2 uv=pixel.xy*screen_res.zw;
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float r=dot(f,f);
    float3 normal=clamp(normalize(float3(f*sqrt(abs(1-r*.25)),r*.5-1)),-1,1);
    float3 world_normal=normalize(mul((float3x3)m_V2W,normal));
    float4 parameters=t_parameters.SampleLevel(s_accumulator,uv,0);
#ifdef CONFIG_SOURCE_AMBIENT_AO
    parameters.xy=floor(saturate(parameters.xy)*127)/127;
#endif
    // The converted diffuse cube begins at the old 16x16 mip; mip 1 is the
    // original 8x8 mip. Retail quality 1 maps to shader quality 3, whose
    // eight directional samples are deliberately divided by nine.
#ifdef CONFIG_SOURCE_QUALITY3
    float3 diffuse=0;
    [unroll] for (int tap=0;tap<8;++tap) {
        float3 cone=source_cones[tap];
        float3 direction=normalize(mul(float3x3(cone,cone.zxy,cone.yzx),world_normal));
        diffuse+=source_diffuse_sample(direction);
    }
    #ifdef LEGACY_SOURCE_011A
    diffuse*=1./8;
#else
    diffuse*=1./9;
#endif
#else
    float3 diffuse=source_diffuse_sample(world_normal);
#endif
    diffuse*=color_parameters.x;
#ifdef CONFIG_SOURCE_AMBIENT_AO
    // Source applies AO to each probe before min(diffuse,10), Fresnel and
    // encoding. Multiplying the already blended accumulator is not equivalent.
    diffuse*=t_legacy_ssao.SampleLevel(s_linear,uv,0).x*2.5;
#endif
    float3 eye_ray=float3((1-2*uv)*s_eye_ray_corner.xy/s_eye_ray_corner.z,1);
    float3 world_position=mul(m_V2W,float4(eye_ray*t_position.SampleLevel(s_position,uv,0).x,1));
    float3 view_direction=normalize(eye_position-world_position);
    float view_dot_normal=saturate(dot(world_normal,view_direction));
    float3 reflection=-reflect(view_direction,world_normal);
#ifndef CONFIG_SOURCE_DIFFUSE_ONLY
    float mip=(num_mips-1)*sqrt(view_dot_normal)*parameters.y;
    float2 brdf=t_probe_brdf.SampleLevel(s_material1,float2(view_dot_normal,parameters.y),0).xy;
    brdf*=brdf;
    float brdf_factor=brdf.x*(parameters.x-.04)+brdf.y;
    float3 specular=lerp(native_sample(reflection,floor(mip)),native_sample(reflection,floor(mip)+1),frac(mip))*brdf_factor;
    specular*=t_ssao_accumulator.SampleLevel(s_linear,uv,0).x*color_parameters.y;
#endif
#ifdef CONFIG_PROBE_BOX
    float3 probe_space=mul(world_to_outer_probe,float4(world_position,1)).xyz;
#ifdef CONFIG_SOURCE_FALLOFF
    float3 falloff=1-exp(-6*sqrt(saturate(1-abs(probe_space))));
    float attenuation=pow(falloff.x*falloff.y*falloff.z,2);
#else
    float3 falloff=1-smoothstep(inner_cube_side_sizes,outer_cube_side_sizes,abs(probe_space)*outer_cube_side_sizes);
    float attenuation=falloff.x*falloff.y*falloff.z;
#endif
#else
#ifdef CONFIG_SOURCE_FALLOFF
    // Undo the target's 2% outer-volume contraction to recover source radius.
    float source_radius=light_range/.98;
    float distance_ratio=length(probe_location-world_position)/(source_radius*.95);
    float power=distance_ratio*distance_ratio;
    power*=power;
    power*=power;
    float attenuation=exp(-8*power);
#else
    float attenuation=1-smoothstep(inner_radius,light_range,length(probe_location-world_position));
#endif
#endif
    Result result;
#ifdef CONFIG_SOURCE_FALLOFF
    // Both G-buffer producers store smoothness = (1 - roughness) * multiplier.
    // probe shader calls that stored value gloss, then derives diffuse_gloss
    // as 1 - gloss. Target t_parameters.y is smoothness, despite consumer names.
    float source_gloss=parameters.y;
    float source_diffuse_gloss=1-parameters.y;
#ifdef CONFIG_SOURCE_QUALITY3
    float3 half_direction=normalize(world_normal+reflection);
    float3 direction=lerp(half_direction,reflection,pow(saturate(source_diffuse_gloss),.8));
    float3 mirror_direction=normalize(direction);
    float3 light_direction=lerp(mirror_direction,normalize(world_normal+mirror_direction),source_gloss);
    float3 halfway_direction=normalize(light_direction+view_direction);
    float fresnel_dot=max(dot(view_direction,halfway_direction),0);
#else
    float fresnel_dot=view_dot_normal;
#endif
    float fresnel=parameters.x+(1-parameters.x)*pow(1-fresnel_dot,5);
    result.diffuse=float4(min(diffuse,10)*(1-fresnel*source_diffuse_gloss),attenuation);
#else
    result.diffuse=float4(diffuse*(1-parameters.x),attenuation);
#endif
#ifndef CONFIG_SOURCE_DIFFUSE_ONLY
    result.specular=float4(specular,attenuation);
#endif
#ifdef CONFIG_ENCODED_PROBE
    result.diffuse.rgb=sqrt(saturate(result.diffuse.rgb*.125));
#endif
    return result;
}
