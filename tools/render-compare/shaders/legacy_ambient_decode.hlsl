// Replace the native SSAO-only multiply draw with an overwrite draw, restoring
// its blend state afterward. Decode probes before later linear-light passes.
#ifdef ORIGINAL_SSAO
Texture2D t_legacy_ssao : register(t0);
#define t_ssao_accumulator t_legacy_ssao
#else
Texture2D t_ssao_accumulator : register(t0);
#endif
Texture2D t_encoded_ambient : register(t2);
SamplerState s_linear : register(s0);
float4 main(float4 pixel : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET {
    float3 encoded=t_encoded_ambient.Load(int3(int2(pixel.xy),0)).rgb;
    float ao=t_ssao_accumulator.SampleLevel(s_linear,uv,0).x;
#ifdef ORIGINAL_SSAO
    ao*=2.5; // Original raw RG8 AO is normalized only when lighting consumes it.
#endif
    return float4(8*encoded*encoded*ao,1);
}
