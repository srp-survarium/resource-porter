// Opt-in source spot diffuse, accumulated separately from the native targets.
// Replay uses a pre-draw depth/stencil copy: native light passes invert stencil.
static const GUID local_diffuse_guid={0x3d2a487b,0xf672,0x40f8,{0x8d,0xf2,0x18,0x6b,0xb6,0x5e,0x28,0x3a}};
static ID3D11PixelShader *legacy_local_diffuse_shader;
static ID3D11SamplerState *legacy_local_shadow_sampler;
static ID3D11Texture2D *legacy_local_diffuse_texture,*legacy_local_depth_texture;
static ID3D11RenderTargetView *legacy_local_diffuse_target;
static ID3D11ShaderResourceView *legacy_local_diffuse_view;
static ID3D11DepthStencilView *legacy_local_depth_view;
static D3D11_TEXTURE2D_DESC legacy_local_depth_desc;
static D3D11_DEPTH_STENCIL_VIEW_DESC legacy_local_depth_view_desc;
static unsigned legacy_local_frame,legacy_local_draws,legacy_local_unhandled,legacy_local_failures;
static unsigned legacy_local_capture_id;
static void tag_source_local(ID3D11PixelShader *shader,const void *code,SIZE_T size) {
    int index=local_light_index(code,size);
    if (!legacy_local_diffuse_code || index<0) return;
    unsigned marker=(unsigned)index+1;
    ID3D11PixelShader_SetPrivateData(shader,&local_diffuse_guid,sizeof(marker),&marker);
}
typedef struct {
    int ready;
    ID3D11PixelShader *shader;
    ID3D11RenderTargetView *targets[8];
    ID3D11DepthStencilView *depth;
} LocalDiffuseDraw;
static void release_local_draw(LocalDiffuseDraw *state) {
    if (state->shader) ID3D11PixelShader_Release(state->shader);
    if (state->depth) ID3D11DepthStencilView_Release(state->depth);
    for (unsigned i=0;i<8;++i) if (state->targets[i]) ID3D11RenderTargetView_Release(state->targets[i]);
    memset(state,0,sizeof(*state));
}
static void begin_local_diffuse_frame(ID3D11DeviceContext *context) {
    if (legacy_local_frame==present_calls) return;
    legacy_local_frame=present_calls; legacy_local_draws=legacy_local_unhandled=legacy_local_failures=0;
    if (legacy_local_diffuse_target) {
        const float zero[4]={0}; ID3D11DeviceContext_ClearRenderTargetView(context,legacy_local_diffuse_target,zero);
    }
}
static int prepare_local_targets(ID3D11DeviceContext *context,LocalDiffuseDraw *state) {
    ID3D11Device *device=NULL; ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL;
    ID3D11DeviceContext_GetDevice(context,&device);
    ID3D11RenderTargetView_GetResource(state->targets[0],&resource);
    HRESULT hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource); resource=NULL;
    if (FAILED(hr)) { ID3D11Device_Release(device); return 0; }
    D3D11_TEXTURE2D_DESC target; ID3D11Texture2D_GetDesc(texture,&target); ID3D11Texture2D_Release(texture); texture=NULL;
    D3D11_RENDER_TARGET_VIEW_DESC native_view; ID3D11RenderTargetView_GetDesc(state->targets[0],&native_view);
    if (target.Format!=DXGI_FORMAT_R11G11B10_FLOAT || target.SampleDesc.Count!=1 || target.ArraySize!=1 ||
        native_view.ViewDimension!=D3D11_RTV_DIMENSION_TEXTURE2D || native_view.Texture2D.MipSlice) {
        ID3D11Device_Release(device); return 0;
    }
    D3D11_TEXTURE2D_DESC current={0};
    if (legacy_local_diffuse_texture) ID3D11Texture2D_GetDesc(legacy_local_diffuse_texture,&current);
    if (!legacy_local_diffuse_target || !legacy_local_diffuse_view || current.Width!=target.Width || current.Height!=target.Height) {
        if (legacy_local_diffuse_view) ID3D11ShaderResourceView_Release(legacy_local_diffuse_view);
        if (legacy_local_diffuse_target) ID3D11RenderTargetView_Release(legacy_local_diffuse_target);
        if (legacy_local_diffuse_texture) ID3D11Texture2D_Release(legacy_local_diffuse_texture);
        legacy_local_diffuse_view=NULL; legacy_local_diffuse_target=NULL; legacy_local_diffuse_texture=NULL;
        target.Format=DXGI_FORMAT_R10G10B10A2_UNORM; target.MipLevels=1;
        target.BindFlags=D3D11_BIND_RENDER_TARGET|D3D11_BIND_SHADER_RESOURCE; target.Usage=D3D11_USAGE_DEFAULT;
        target.CPUAccessFlags=0; target.MiscFlags=0;
        hr=ID3D11Device_CreateTexture2D(device,&target,NULL,&legacy_local_diffuse_texture);
        if (SUCCEEDED(hr)) hr=ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)legacy_local_diffuse_texture,NULL,&legacy_local_diffuse_target);
        if (SUCCEEDED(hr)) hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)legacy_local_diffuse_texture,NULL,&legacy_local_diffuse_view);
        if (FAILED(hr)) { ID3D11Device_Release(device); return 0; }
        const float zero[4]={0}; ID3D11DeviceContext_ClearRenderTargetView(context,legacy_local_diffuse_target,zero);
    }
    ID3D11DepthStencilView_GetResource(state->depth,&resource);
    hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource);
    if (FAILED(hr)) { ID3D11Device_Release(device); return 0; }
    D3D11_TEXTURE2D_DESC depth; ID3D11Texture2D_GetDesc(texture,&depth);
    D3D11_DEPTH_STENCIL_VIEW_DESC view; ID3D11DepthStencilView_GetDesc(state->depth,&view);
    if (depth.Width!=target.Width || depth.Height!=target.Height || depth.SampleDesc.Count!=1 || depth.ArraySize!=1 ||
        view.ViewDimension!=D3D11_DSV_DIMENSION_TEXTURE2D || view.Texture2D.MipSlice ||
        (view.Format!=DXGI_FORMAT_D24_UNORM_S8_UINT && view.Format!=DXGI_FORMAT_D32_FLOAT_S8X24_UINT)) {
        ID3D11Texture2D_Release(texture); ID3D11Device_Release(device); return 0;
    }
    if (!legacy_local_depth_texture || memcmp(&depth,&legacy_local_depth_desc,sizeof(depth)) || memcmp(&view,&legacy_local_depth_view_desc,sizeof(view))) {
        if (legacy_local_depth_view) ID3D11DepthStencilView_Release(legacy_local_depth_view);
        if (legacy_local_depth_texture) ID3D11Texture2D_Release(legacy_local_depth_texture);
        legacy_local_depth_view=NULL; legacy_local_depth_texture=NULL;
        D3D11_TEXTURE2D_DESC copy=depth; copy.Usage=D3D11_USAGE_DEFAULT; copy.BindFlags=D3D11_BIND_DEPTH_STENCIL; copy.CPUAccessFlags=0; copy.MiscFlags=0;
        hr=ID3D11Device_CreateTexture2D(device,&copy,NULL,&legacy_local_depth_texture);
        if (SUCCEEDED(hr)) hr=ID3D11Device_CreateDepthStencilView(device,(ID3D11Resource *)legacy_local_depth_texture,&view,&legacy_local_depth_view);
        if (FAILED(hr)) { ID3D11Texture2D_Release(texture); ID3D11Device_Release(device); return 0; }
        legacy_local_depth_desc=depth; legacy_local_depth_view_desc=view;
    }
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)legacy_local_depth_texture,(ID3D11Resource *)texture);
    ID3D11Texture2D_Release(texture);
    if (!legacy_local_diffuse_shader)
        hr=original_create_ps(device,legacy_local_diffuse_code,legacy_local_diffuse_size,NULL,&legacy_local_diffuse_shader);
    if (SUCCEEDED(hr) && !legacy_local_shadow_sampler) {
        D3D11_SAMPLER_DESC sampler={0}; sampler.Filter=D3D11_FILTER_COMPARISON_MIN_MAG_MIP_LINEAR;
        sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.ComparisonFunc=D3D11_COMPARISON_LESS_EQUAL; sampler.MaxAnisotropy=1; sampler.MaxLOD=FLT_MAX;
        for (unsigned i=0;i<4;++i) sampler.BorderColor[i]=1;
        hr=ID3D11Device_CreateSamplerState(device,&sampler,&legacy_local_shadow_sampler);
    }
    ID3D11Device_Release(device);
    return SUCCEEDED(hr) && legacy_local_diffuse_shader && legacy_local_shadow_sampler;
}
static void prepare_local_diffuse(ID3D11DeviceContext *context,LocalDiffuseDraw *state) {
    memset(state,0,sizeof(*state)); if (!legacy_local_diffuse_code) return;
    begin_local_diffuse_frame(context);
    ID3D11DeviceContext_PSGetShader(context,&state->shader,NULL,NULL);
    if (!state->shader) return;
    UINT size=sizeof(unsigned); unsigned marker=0;
    HRESULT hr=ID3D11PixelShader_GetPrivateData(state->shader,&local_diffuse_guid,&size,&marker);
    if (FAILED(hr) || !marker) { release_local_draw(state); return; }
    static const unsigned char spot[]={0x33,0x20,0x4e,0x90,0x75,0x19,0xb2,0xad,0x0a,0xfa,0x79,0x8e,0x61,0x54,0xae,0xbc};
    if (marker>sizeof(local_light_shaders)/sizeof(local_light_shaders[0]) || memcmp(local_light_shaders[marker-1].digest,spot,16)) {
        ++legacy_local_unhandled; release_local_draw(state); return;
    }
    ID3D11DeviceContext_OMGetRenderTargets(context,8,state->targets,&state->depth);
    ID3D11BlendState *blend=NULL; ID3D11DeviceContext_OMGetBlendState(context,&blend,NULL,NULL);
    D3D11_BLEND_DESC description={0}; if (blend) { ID3D11BlendState_GetDesc(blend,&description); ID3D11BlendState_Release(blend); }
    const D3D11_RENDER_TARGET_BLEND_DESC *b=&description.RenderTarget[0];
    if (!state->targets[0] || !state->depth || !b->BlendEnable || b->SrcBlend!=D3D11_BLEND_ONE || b->DestBlend!=D3D11_BLEND_ONE ||
        b->BlendOp!=D3D11_BLEND_OP_ADD || (b->RenderTargetWriteMask&7)!=7 || !prepare_local_targets(context,state)) {
        ++legacy_local_failures; error("source local diffuse replay contract failed"); release_local_draw(state); return;
    }
    state->ready=1;
}
static void finish_local_diffuse(ID3D11DeviceContext *context,LocalDiffuseDraw *state,UINT count,UINT start,INT base_vertex,int indexed) {
    if (state->ready) {
        // Native depth/stencil state, VS, constants, SRVs, samplers, viewport and
        // scissor remain bound. Only PS and OM targets change for the replay.
        ID3D11DeviceContext_OMSetRenderTargets(context,1,&legacy_local_diffuse_target,legacy_local_depth_view);
        ID3D11DeviceContext_PSSetShader(context,legacy_local_diffuse_shader,NULL,0);
        ID3D11SamplerState *sampler=NULL; ID3D11DeviceContext_PSGetSamplers(context,3,1,&sampler);
        ID3D11DeviceContext_PSSetSamplers(context,3,1,&legacy_local_shadow_sampler);
        if (indexed) original_draw_indexed(context,count,start,base_vertex); else original_draw(context,count,start);
        ID3D11DeviceContext_PSSetSamplers(context,3,1,&sampler);
        if (sampler) ID3D11SamplerState_Release(sampler);
        ID3D11DeviceContext_PSSetShader(context,state->shader,NULL,0);
        ID3D11DeviceContext_OMSetRenderTargets(context,8,state->targets,state->depth);
        ++legacy_local_draws;
    }
    release_local_draw(state);
}
static void capture_local_diffuse(ID3D11DeviceContext *context,const Camera *camera) {
    if (!legacy_local_diffuse_code || legacy_local_capture_id==camera->id) return;
    begin_local_diffuse_frame(context);
    char base[MAX_PATH+48],path[MAX_PATH+80]; snprintf(base,sizeof(base),"%s.local-encoded",camera->output);
    snprintf(path,sizeof(path),"%s.json",base); FILE *file=fopen(path,"w");
    if (file) { fprintf(file,"{\"present\":%u,\"replayed_draws\":%u,\"unhandled_draws\":%u,\"failed_draws\":%u,\"scope\":\"local-light-only; zero ambient\"}\n",present_calls,legacy_local_draws,legacy_local_unhandled,legacy_local_failures); fclose(file); }
    if (legacy_local_diffuse_texture) {
        snprintf(path,sizeof(path),"%s.targets.json",base); file=fopen(path,"w");
        if (file) { fputs("[",file); dump_texture(legacy_local_diffuse_texture,base,file,"source_encoded_local_diffuse",0,NULL); fputs("]\n",file); fclose(file); }
    }
    legacy_local_capture_id=camera->id;
}
