struct Output { float4 position : SV_POSITION; float2 uv : TEXCOORD0; };
Output main(uint vertex : SV_VertexID) {
    Output result;
    result.uv=float2((vertex<<1)&2,vertex&2);
    result.position=float4(result.uv*float2(2,-2)+float2(-1,1),0,1);
    return result;
}
