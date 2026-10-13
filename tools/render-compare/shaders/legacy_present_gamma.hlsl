// Original final output conversion, paired with the original grading tail.
cbuffer CopyConstants : register(b0) { float gamma_correction_factor; };
Texture2D t_base : register(t0);
SamplerState s_accumulator : register(s0);
float4 main(float4 pixel : SV_POSITION,float2 uv : TEXCOORD0) : SV_TARGET {
    return float4(pow(abs(t_base.Sample(s_accumulator,uv).rgb),1/(2.2*gamma_correction_factor)),1);
}
