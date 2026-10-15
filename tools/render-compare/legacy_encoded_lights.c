// Optional source accumulation of local-light diffuse in sqrt-encoded space.
//
// The 0.10b/0.11e renderer accumulates every light into a 10-bit diffuse
// target as E = sqrt(saturate(L/8)) with additive blending and decodes
// 8*E*E at composition, so light sums are super-additive. 0.26 adds linear
// light. With SRP_LEGACY_ENCODED_LIGHTS (the combine shader) set, each native
// local-light draw (any point/spot identity, including the F0=1 overrides)
// renders its diffuse into a cleared private target; the diffuse accumulator
// then becomes 8*min(sqrt(sat(A/8))+sqrt(sat(L/8)),1)^2. Everything already in
// the accumulator acts as one encoded term, as in the original. Specular and
// all other targets stay native. Without the variable nothing changes.
static void *legacy_encoded_lights_code;
static size_t legacy_encoded_lights_size;
static ID3D11PixelShader *encoded_lights_ps;
static ID3D11Texture2D *encoded_light_texture,*encoded_accum_copy;
static ID3D11RenderTargetView *encoded_light_target;
static ID3D11ShaderResourceView *encoded_light_view,*encoded_accum_view;
static unsigned encoded_light_draws,encoded_light_failures,encoded_light_frame;
static const GUID encoded_light_guid={0x51b5c7f2,0x0e2d,0x4c47,{0x9a,0x63,0x2b,0x0d,0x7e,0x58,0x91,0x4c}};

static int native_local_light_identity(const void *code,SIZE_T size) {
    if (size<32) return -1;
    for (unsigned i=0;i<sizeof(local_light_shaders)/sizeof(local_light_shaders[0]);++i)
        if (size==local_light_shaders[i].size && !memcmp((const char *)code+4,local_light_shaders[i].digest,16)) return (int)i;
    return -1;
}
// Tag at creation with the native identity, before any override replaces the code.
static void tag_encoded_light(ID3D11PixelShader *shader,int identity) {
    if (!legacy_encoded_lights_code || identity<0 || !shader) return;
    unsigned marker=(unsigned)identity+1;
    ID3D11PixelShader_SetPrivateData(shader,&encoded_light_guid,sizeof(marker),&marker);
}
static void report_encoded_lights(void) {
    char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.encoded-lights.json",control);
    FILE *file=fopen(path,"w"); if (!file) return;
    fprintf(file,"{\"scope\":\"local-light diffuse added in sqrt-encoded space (0.10b/0.11e contract); specular native\",\"present\":%u,\"draws\":%u,\"failures\":%u}\n",
            present_calls,encoded_light_draws,encoded_light_failures);
    fclose(file);
}
static int prepare_encoded_targets(ID3D11Device *device,ID3D11Texture2D *accum) {
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(accum,&desc);
    if (desc.SampleDesc.Count!=1 || desc.ArraySize!=1 || desc.Format!=DXGI_FORMAT_R11G11B10_FLOAT) return 0;
    D3D11_TEXTURE2D_DESC current={0};
    if (encoded_light_texture) ID3D11Texture2D_GetDesc(encoded_light_texture,&current);
    if (encoded_light_texture && current.Width==desc.Width && current.Height==desc.Height) return 1;
    if (encoded_light_view) ID3D11ShaderResourceView_Release(encoded_light_view);
    if (encoded_light_target) ID3D11RenderTargetView_Release(encoded_light_target);
    if (encoded_light_texture) ID3D11Texture2D_Release(encoded_light_texture);
    if (encoded_accum_view) ID3D11ShaderResourceView_Release(encoded_accum_view);
    if (encoded_accum_copy) ID3D11Texture2D_Release(encoded_accum_copy);
    encoded_light_view=NULL; encoded_light_target=NULL; encoded_light_texture=NULL; encoded_accum_view=NULL; encoded_accum_copy=NULL;
    D3D11_TEXTURE2D_DESC light=desc; light.MipLevels=1; light.Usage=D3D11_USAGE_DEFAULT; light.CPUAccessFlags=0; light.MiscFlags=0;
    light.BindFlags=D3D11_BIND_RENDER_TARGET|D3D11_BIND_SHADER_RESOURCE; light.Format=DXGI_FORMAT_R16G16B16A16_FLOAT;
    D3D11_TEXTURE2D_DESC copy=desc; copy.MipLevels=desc.MipLevels; copy.Usage=D3D11_USAGE_DEFAULT; copy.CPUAccessFlags=0;
    copy.MiscFlags=0; copy.BindFlags=D3D11_BIND_SHADER_RESOURCE;
    HRESULT hr=ID3D11Device_CreateTexture2D(device,&light,NULL,&encoded_light_texture);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)encoded_light_texture,NULL,&encoded_light_target);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)encoded_light_texture,NULL,&encoded_light_view);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateTexture2D(device,&copy,NULL,&encoded_accum_copy);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)encoded_accum_copy,NULL,&encoded_accum_view);
    if (SUCCEEDED(hr) && !encoded_lights_ps) hr=original_create_ps(device,legacy_encoded_lights_code,legacy_encoded_lights_size,NULL,&encoded_lights_ps);
    return SUCCEEDED(hr);
}
// Returns 1 when the draw was performed here.
static int encoded_light_draw(ID3D11DeviceContext *context,UINT count,UINT start,INT base,int indexed) {
    if (!legacy_encoded_lights_code) return 0;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return 0;
    unsigned marker=0; UINT size=sizeof(marker);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&encoded_light_guid,&size,&marker);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || !marker) return 0;
    if (encoded_light_frame!=present_calls) {
        if (encoded_light_frame) report_encoded_lights();
        encoded_light_frame=present_calls; encoded_light_draws=encoded_light_failures=0;
    }
    ID3D11RenderTargetView *targets[8]={0}; ID3D11DepthStencilView *depth=NULL;
    ID3D11DeviceContext_OMGetRenderTargets(context,8,targets,&depth);
    ID3D11BlendState *blend=NULL; FLOAT factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    D3D11_BLEND_DESC bd={0}; if (blend) ID3D11BlendState_GetDesc(blend,&bd);
    const D3D11_RENDER_TARGET_BLEND_DESC *b=&bd.RenderTarget[0];
    ID3D11Resource *resource=NULL; ID3D11Texture2D *accum=NULL; ID3D11Device *device=NULL;
    int ok=targets[0] && b->BlendEnable && b->SrcBlend==D3D11_BLEND_ONE && b->DestBlend==D3D11_BLEND_ONE &&
           b->BlendOp==D3D11_BLEND_OP_ADD && ssao_context1 && ssao_state && legacy_ssao_fullscreen_shader();
    if (ok) {
        ID3D11RenderTargetView_GetResource(targets[0],&resource);
        ok=SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&accum));
    }
    if (ok) { ID3D11DeviceContext_GetDevice(context,&device); ok=prepare_encoded_targets(device,accum); }
    if (ok) {
        // 1. The native light, its diffuse redirected to a cleared private target.
        const float zero[4]={0};
        ID3D11DeviceContext_ClearRenderTargetView(context,encoded_light_target,zero);
        ID3D11RenderTargetView *redirected[8]; memcpy(redirected,targets,sizeof(redirected));
        redirected[0]=encoded_light_target;
        ID3D11DeviceContext_OMSetRenderTargets(context,8,redirected,depth);
        if (indexed) original_draw_indexed(context,count,start,base); else original_draw(context,count,start);
        ID3D11DeviceContext_OMSetRenderTargets(context,8,targets,depth);
        // 2. A := 8*min(sqrt(sat(A/8))+sqrt(sat(L/8)),1)^2 in a private pipeline state.
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)encoded_accum_copy,(ID3D11Resource *)accum);
        D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(accum,&desc);
        ID3DDeviceContextState *previous=NULL;
        ID3D11DeviceContext1_SwapDeviceContextState(ssao_context1,ssao_state,&previous);
        ID3D11DeviceContext_ClearState(context);
        ID3D11DeviceContext_VSSetShader(context,legacy_ssao_fullscreen_shader(),NULL,0);
        ID3D11DeviceContext_PSSetShader(context,encoded_lights_ps,NULL,0);
        ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ID3D11DeviceContext_RSSetState(context,ssao_raster);
        ID3D11ShaderResourceView *inputs[2]={encoded_accum_view,encoded_light_view};
        ID3D11DeviceContext_PSSetShaderResources(context,0,2,inputs);
        D3D11_VIEWPORT vp={0,0,(float)desc.Width,(float)desc.Height,0,1};
        ID3D11DeviceContext_RSSetViewports(context,1,&vp);
        ID3D11DeviceContext_OMSetRenderTargets(context,1,&targets[0],NULL);
        original_draw(context,3,0);
        ID3D11DeviceContext_ClearState(context);
        ID3D11DeviceContext1_SwapDeviceContextState(ssao_context1,previous,NULL);
        if (previous) ID3DDeviceContextState_Release(previous);
        ++encoded_light_draws;
    } else {
        ++encoded_light_failures;
        if (indexed) original_draw_indexed(context,count,start,base); else original_draw(context,count,start);
    }
    if (device) ID3D11Device_Release(device);
    if (accum) ID3D11Texture2D_Release(accum);
    if (resource) ID3D11Resource_Release(resource);
    if (blend) ID3D11BlendState_Release(blend);
    if (depth) ID3D11DepthStencilView_Release(depth);
    for (unsigned i=0;i<8;++i) if (targets[i]) ID3D11RenderTargetView_Release(targets[i]);
    return 1;
}
