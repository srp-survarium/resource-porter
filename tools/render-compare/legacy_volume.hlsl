// Original 0.10b cone opacity with the captured native 0.26 input layout.
// Experiment for Factory's cone_volume_material (authored attenuation_scale=13).
cbuffer Volume : register(b0) {
    float4 mode_direction_and_uv_tile : packoffset(c0);
    float solid_transparency : packoffset(c1.x);
    float attenuation_scale : packoffset(c1.y);
    float4 volume_color : packoffset(c3);
};
cbuffer Scene : register(b1) {
    row_major float3x4 m_V2W : packoffset(c11);
    float4 screen_res : packoffset(c14);
    float3 eye_position : packoffset(c20);
    float scene_time : packoffset(c25.x);
};
Texture2D t_diffuse : register(t0);
Texture2D t_position : register(t1);
Texture2D t_spot_falloff : register(t2);
Texture2D t_sphere_falloff : register(t14);
SamplerState s_base : register(s0);
SamplerState s_position : register(s1);
struct Input {
    float4 position : SV_Position;
    float4 tbn_x : TEXCOORD0;
    float4 tbn_y : TEXCOORD1;
    float4 tbn_z : TEXCOORD2;
    float4 world_position : TEXCOORD3;
    float4 view_position : TEXCOORD4;
    float4 normal : TEXCOORD5;
    float2 tc : TEXCOORD6;
};
float4 main(Input input) : SV_Target {
    float3x3 tangent_to_view = float3x3(input.tbn_x.xyz, input.tbn_y.xyz, input.tbn_z.xyz);
    float3x3 tangent_to_world = mul((float3x3)m_V2W, tangent_to_view);
    float3 world_position = mul(m_V2W, float4(input.view_position.xyz, 1));
    float3 direction = normalize(mul(normalize(eye_position - world_position), tangent_to_world));
    float3 reflection = reflect(direction, float3(0, 0, 1));
    reflection.z = sqrt((reflection.z + 8) * .8);
    float2 falloff_uv = reflection.xy / reflection.z + .5;
    float cone = t_spot_falloff.Sample(s_base, float2(falloff_uv.x, input.tc.y)).r;
    float sphere = t_sphere_falloff.Sample(s_base, falloff_uv).r;
    float depth = t_position.SampleLevel(s_position, input.position.xy / screen_res.xy, 0).r;
    float depth_fade = saturate(.75 * (depth - input.view_position.z));
    // Native attenuation_scale is optimized out and its captured bytes are undefined.
    float distance_fade = saturate(13 * input.view_position.z);
    float alpha = saturate(distance_fade * cone * depth_fade * volume_color.a * sphere);
    alpha *= solid_transparency;
    float2 uv = mode_direction_and_uv_tile.w * float2(2, 1)
              * (input.tc + mode_direction_and_uv_tile.xy * scene_time);
    return float4(t_diffuse.Sample(s_base, uv).rgb * 100 + volume_color.rgb, alpha);
}
