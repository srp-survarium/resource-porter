// Isolate v0.10b rain exposure/filmic response within the active 0.26 post ABI.
// Bloom inputs, target LUT conversion and vignette stay at the captured contract.
cbuffer PostConstants : register(b0)
{
    float4 image_grain_parameters : packoffset(c11);
    float4 bloom_parameters : packoffset(c18);
};
Texture2D t_frame_color : register(t0);
Texture3D t_color_grading_lut : register(t1);
Texture2D t_frame_luminance : register(t2);
Texture2D t_light_scattering_result : register(t3);
Texture2D t_bloom_combine : register(t4);
SamplerState s_linear : register(s0);
SamplerState s_accumulator : register(s1);
SamplerState s_material1 : register(s2);

float4 main(float4 position : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET
{
    float3 frame=t_frame_color.SampleLevel(s_material1,uv,0).xyz;
    float3 bloom=t_bloom_combine.SampleLevel(s_material1,uv,0).xyz;
    float3 color=lerp(frame,bloom,bloom_parameters.x)+t_light_scattering_result.Sample(s_linear,uv).xyz;
    float average=clamp(t_frame_luminance.Sample(s_accumulator,float2(.5,.5)).x,.09,.4);
    float3 x=max(color*pow(.25/average,.2)-.0015,0);
    color=pow((x*(6.2*x+.2))/(x*(5.9*x+1.5)+.04),2.2);
    float3 encoded=1.055*pow(abs(color),1/2.4)-.055;
    float3 graded=t_color_grading_lut.SampleLevel(s_material1,encoded*(15./16)+(.5/16),0).xyz;
    color=pow((abs(graded)+.055)/1.055,2.4);
    if (image_grain_parameters.w>.1)
        color*=min(pow(abs(1-dot(uv-.5,uv-.5)),image_grain_parameters.w+1),1);
    return float4(color,0);
}
