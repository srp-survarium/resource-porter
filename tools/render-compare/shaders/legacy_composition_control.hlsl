// Reconstruct the captured 0.26 composition shader (DXBC de3e7a63...), so the
// encoded-addition experiment can hold its other inputs and formulas fixed.
cbuffer CompositionConstants : register(b0) {
    float3 light_direction : packoffset(c0);
    float3 light_color : packoffset(c1);
    float4 debug_multipliers : packoffset(c24);
    float3 s_eye_ray_corner : packoffset(c25);
    float roughness_multiplier : packoffset(c25.w);
    row_major float4x4 view_to_shadow : packoffset(c26);
    float use_rain : packoffset(c30);
};
cbuffer StaticGlobals : register(b1) {
    row_major float3x4 m_V2W : packoffset(c11);
    float3 eye_direction : packoffset(c21);
};
Texture2D t_normal : register(t0);
Texture2D t_diffuse : register(t1);
Texture2D t_parameters : register(t2);
Texture2D t_position : register(t3);
Texture2D t_diffuse_accumulation : register(t4);
Texture2D t_specular_accumulation : register(t5);
Texture2D t_sun_shadow_and_scattering : register(t6);
Texture2D t_rain_shadow_map : register(t7);
#ifdef CONFIG_FULL_ENCODED_DIFFUSE
Texture2D t_full_encoded_diffuse : register(t15);
#endif
SamplerState s_position : register(s0);
SamplerState s_normal : register(s1);
SamplerState s_accumulator : register(s2);
SamplerComparisonState s_shmap : register(s3);

#ifdef CONFIG_SOURCE_TRANSLUCENCY
cbuffer TranslucencyConstants : register(b2) { float4 source_sun_color; };
Texture2D t_legacy_sun_projection : register(t13);
#endif

float4 main(float4 pixel : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET {
    float3 position=float3((1-2*uv)*s_eye_ray_corner.xy/s_eye_ray_corner.z,1)
        *t_position.SampleLevel(s_position,uv,0).x;
    float2 f=t_normal.SampleLevel(s_normal,uv,0).xy*4-2;
    float r=dot(f,f);
    float3 normal=clamp(normalize(float3(f*sqrt(abs(1-r*.25)),r*.5-1)),-1,1);
    float3 albedo=pow((abs(t_diffuse.SampleLevel(s_accumulator,uv,0).xyz)+.055)/1.055,2.4);
    float4 parameters=t_parameters.SampleLevel(s_accumulator,uv,0);
    float2 shadow=t_sun_shadow_and_scattering.SampleLevel(s_accumulator,uv,0).xy;
    float3 sunlight=0, sun_specular=0;
    if (shadow.x>0) {
        float3 view=-position/max(length(position),.00001);
        float roughness=saturate(parameters.y*roughness_multiplier);
        float3 halfway=normalize(view-light_direction);
        float vh=dot(view,halfway),nv=saturate(dot(normal,view));
        float nl=saturate(dot(-light_direction,normal)),nh=dot(normal,halfway);
        float fv=parameters.x+(1-parameters.x)*pow(1-vh,5);
        float fl=parameters.x+(1-parameters.x)*pow(1-nl,5);
        float geometry=min(1/(4*max(nv*nl,.000001)),1);
        float alpha=pow(pow(roughness,6)*.5+roughness*.1,2);
        float distribution=alpha/(3.14159265*pow(nh*nh*(alpha-1)+1,2));
        float k=roughness*.5;
        geometry=min(geometry*(nv/(nv*(1-k)+k))*(nl/(nl*(1-k)+k)),1);
        sunlight=light_color*saturate((1-fl)*nl);
#ifdef CONFIG_SOURCE_SUN_DIFFUSE_SENTINEL
        // Original 010b sun.ps: F > .99 retains full Lambert diffuse. In
        // particular authored F0=1 is a sentinel, not zero diffuse. Preserve
        // every other native term while isolating that branch. Original F0
        // is stored in seven bits and its Schlick angle is view/halfway.
        float source_f0=floor(saturate(parameters.x)*127)/127;
        float source_fresnel=source_f0+(1-source_f0)*pow(1-max(vh,0),5);
        if (source_fresnel>.99) sunlight=light_color*nl;
#endif
        sun_specular=light_color*shadow.x*(nl*geometry*fv*min(distribution,1000));
    }
    float3 scattering=0;
#ifdef CONFIG_SOURCE_TRANSLUCENCY
    // Original material packing stores five gamma-space bits, then decodes 2.2.
    float translucency=pow(floor(pow(saturate(parameters.z),1/2.2)*31)/31,2.2);
    if (translucency>0) {
        float projected_depth=t_legacy_sun_projection.SampleLevel(s_position,uv,0).x;
        // Original sun helper GREEN is visibility, not its RED raw shadow depth.
        float attenuation=pow(1-saturate((projected_depth-shadow.x)/(translucency*.035)),10);
        float back_light=dot(normal,light_direction);
        float forward=saturate(dot(-normalize(position),light_direction));
        scattering=translucency*.1
            +source_sun_color.xyz*back_light*attenuation*translucency*1.5
            +translucency*back_light*.1*source_sun_color.xyz
            +source_sun_color.xyz*forward*forward*forward*translucency*attenuation*2;
    }
#else
    if (parameters.z>.001) {
        scattering=light_color*(max(dot(normal,light_direction),0)*shadow.y*parameters.z+parameters.z*.02);
#ifdef CONFIG_SOURCE_TRANSLUCENCY_BASE
        // Isolate the material-only base term from original translucency.ps.
        // Native backlight, shadow attenuation and view-dependent terms remain.
        scattering+=parameters.z*.1-light_color*parameters.z*.02;
#endif
        float world_y=normalize(mul((float3x3)m_V2W,normal)).y;
        scattering+=saturate(-world_y)*parameters.z*(-world_y*eye_direction.y);
    }
#endif
    float3 square_color=albedo*albedo;
    float3 tint=sqrt(square_color/max(length(square_color),.0001))*parameters.w;
    float fresnel=pow(1-saturate(dot(normal,-normalize(position))),5);
    float3 specular_color=lerp(tint*1.3,parameters.www,saturate(1.04-4*parameters.x*parameters.x+fresnel));
    float3 ambient=max(t_diffuse_accumulation.SampleLevel(s_accumulator,uv,0).xyz,0);
    float3 specular=max(t_specular_accumulation.SampleLevel(s_accumulator,uv,0).xyz,0)+sun_specular;
#ifdef CONFIG_FULL_ENCODED_DIFFUSE
    float3 encoded=saturate(t_full_encoded_diffuse.SampleLevel(s_accumulator,uv,0).rgb);
    float3 lighting=8*encoded*encoded;
#elif defined(CONFIG_ENCODED_ADDITION)
    // Isolate the old sun/resolve contract. Re-encoding the already combined
    // ambient buffer does NOT reproduce the old blending of individual probes
    // or local lights; those require their own matched producer changes.
    float3 encoded=sqrt(saturate(ambient*.125))
        +sqrt(saturate(sunlight*.125))*shadow.x
        +sqrt(saturate(scattering*.125));
    float3 lighting=8*pow(saturate(encoded),2);
#else
    float3 lighting=ambient+sunlight*shadow.x+scattering;
#endif
    float3 diffuse_result=albedo*debug_multipliers.x*lighting;
#ifndef CONFIG_NO_RAIN_DARKENING
    if (use_rain>.5) {
        float4 projected=mul(view_to_shadow,float4(position+normal*1.3,1));
        float3 tc=projected.xyz/projected.w;
        float rain=1;
        if (all(tc.xy>=0) && all(tc.xy<=1)) {
            float d=.00390625;
            rain=(t_rain_shadow_map.SampleCmpLevelZero(s_shmap,tc.xy+float2(d,d),tc.z)
                +t_rain_shadow_map.SampleCmpLevelZero(s_shmap,tc.xy+float2(-d,-d),tc.z)
                +t_rain_shadow_map.SampleCmpLevelZero(s_shmap,tc.xy+float2(d,-d),tc.z)
                +t_rain_shadow_map.SampleCmpLevelZero(s_shmap,tc.xy+float2(-d,d),tc.z))*.25;
        }
        float wet=saturate(parameters.x*330-500)*saturate((parameters.y-.5)*10);
        diffuse_result*=.85+rain*(5*(.9*wet+.1)-.85);
    }
#endif
    return float4(diffuse_result+specular_color*debug_multipliers.y*specular,0);
}
