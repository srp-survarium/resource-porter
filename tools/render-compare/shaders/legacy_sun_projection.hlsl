// Projection bindings verified across the pinned native sun-shadow variants.
cbuffer ShadowConstants : register(b0) {
    row_major float4x4 view_to_shadow : packoffset(c16);
    float3 eye_ray_corner : packoffset(c22);
};
Texture2D t_position : register(t1);
float main(float4 pixel : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float3 position=float3((1-2*uv)*eye_ray_corner.xy/eye_ray_corner.z,1)
        *t_position.Load(int3(int2(pixel.xy),0)).x;
    float4 projected=mul(view_to_shadow,float4(position,1));
    return projected.z/projected.w;
}
