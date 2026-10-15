// Source (0.10b/0.11e) local-light accumulation: every light adds
// sqrt(saturate(L/8)) to a 10-bit encoded target, decoded as 8*E*E. The
// accumulator so far (t0, linear) and one light's diffuse (t1, linear) are
// re-encoded, summed with the UNORM clamp and decoded.
Texture2D t_accumulated : register(t0);
Texture2D t_light : register(t1);
float4 main(float4 pixel : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    int3 p=int3(int2(pixel.xy),0);
    float3 a=t_accumulated.Load(p).rgb, l=t_light.Load(p).rgb;
    float3 e=min(sqrt(saturate(a/8))+sqrt(saturate(l/8)),1);
    return float4(8*e*e,0);
}
