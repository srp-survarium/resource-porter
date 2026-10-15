// Optional 0.11e skylight underlay for the encoded diffuse accumulator.
//
// 0.11e lights every pixel with its skylight stage first: it overwrites the
// sqrt-encoded diffuse target with sqrt(sky/8). Box probes then lerp over it
// (SRC_ALPHA/INV_SRC_ALPHA) with the 0.11e cubic falloff
// (legacy_probe_011e_falloff*.hlsl), and the map-enclosing sphere lights only
// pixels no box claims, if its interval is not inverted. 0.26 has no such
// stage: it clears the accumulator, so the sphere probe lit everything. With SRP_LEGACY_SKYLIGHT_UNDERLAY naming a directory with
// skylight-underlay.dxbc and source-skylight.bin (the 32 floats 0.11e uploads
// as skylight_parameters0..7), the hook draws the skylight into RT0 of the
// first tagged probe draw of each frame, using that draw's G-buffer and
// constant bindings. Without the variable nothing changes.
static void *skylight_underlay_code;
static size_t skylight_underlay_size;
static float skylight_underlay_parameters[32];
static int skylight_underlay_enabled;
static ID3D11PixelShader *skylight_underlay_ps;
static ID3D11Buffer *skylight_underlay_constants;
static ID3D11BlendState *skylight_underlay_blend;
static ID3D11DepthStencilState *skylight_underlay_depth;
static ID3D11RasterizerState *skylight_underlay_raster;
static unsigned skylight_underlay_frame=~0u,skylight_underlay_draws,skylight_underlay_failures;
static const GUID skylight_probe_guid={0x7c1e9a52,0x3b84,0x4d6e,{0x8f,0x21,0x5a,0x0c,0x93,0x6b,0xd4,0x17}};
// Tag the encoded probe overrides at creation (1 sphere, 2 box).
static void tag_skylight_probe(ID3D11PixelShader *shader,unsigned marker) {
    if (!skylight_underlay_enabled || !shader) return;
    ID3D11PixelShader_SetPrivateData(shader,&skylight_probe_guid,sizeof(marker),&marker);
}

static void report_skylight_underlay(void) {
    char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.skylight-underlay.json",control);
    FILE *file=fopen(path,"w"); if (!file) return;
    fprintf(file,"{\"scope\":\"0.11e skylight written encoded before the first probe; enclosing sphere probe skipped in its shader\",\"present\":%u,\"draws\":%u,\"failures\":%u,\"parameters\":[",
            present_calls,skylight_underlay_draws,skylight_underlay_failures);
    for (unsigned i=0;i<32;++i) fprintf(file,"%s%.9g",i?",":"",skylight_underlay_parameters[i]);
    fputs("]}\n",file); fclose(file);
}

static int configure_skylight_underlay(void) {
    char directory[MAX_PATH],path[MAX_PATH+40];
    DWORD length=GetEnvironmentVariableA("SRP_LEGACY_SKYLIGHT_UNDERLAY",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || !legacy_probe_code || !legacy_probe_box_code || !legacy_ssao_enabled || !legacy_ambient_decode_code) {
        error("skylight underlay requires the encoded probe shaders, source SSAO and the ambient decode"); return 0;
    }
    snprintf(path,sizeof(path),"%s/skylight-underlay.dxbc",directory);
    FILE *file=fopen(path,"rb"); if (!file) { error("skylight underlay shader missing"); return 0; }
    int ok=!fseek(file,0,SEEK_END); long size=ftell(file); rewind(file);
    if (!ok || size<32 || size>1024*1024) { fclose(file); error("skylight underlay shader size"); return 0; }
    skylight_underlay_code=malloc((size_t)size); skylight_underlay_size=(size_t)size;
    ok=skylight_underlay_code && fread(skylight_underlay_code,1,(size_t)size,file)==(size_t)size;
    fclose(file);
    if (!ok || memcmp(skylight_underlay_code,"DXBC",4) || *(uint32_t *)((char *)skylight_underlay_code+24)!=(uint32_t)size) {
        error("skylight underlay shader is not DXBC"); return 0;
    }
    snprintf(path,sizeof(path),"%s/source-skylight.bin",directory);
    file=fopen(path,"rb"); if (!file) { error("skylight underlay constants missing"); return 0; }
    ok=fread(skylight_underlay_parameters,1,sizeof(skylight_underlay_parameters),file)==sizeof(skylight_underlay_parameters) && fgetc(file)==EOF;
    fclose(file);
    for (unsigned i=0;i<32;++i) if (!isfinite(skylight_underlay_parameters[i])) ok=0;
    // p1 = (lower, range, power): a positive range keeps the ramp finite.
    if (!ok || skylight_underlay_parameters[5]<=0 || skylight_underlay_parameters[6]<0) {
        error("skylight underlay constants contract failed"); return 0;
    }
    skylight_underlay_enabled=1; return 1;
}

static int init_skylight_underlay(ID3D11DeviceContext *context) {
    if (skylight_underlay_ps && skylight_underlay_constants && skylight_underlay_blend && skylight_underlay_depth && skylight_underlay_raster) return 1;
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    HRESULT hr=S_OK;
    if (!skylight_underlay_ps) hr=original_create_ps(device,skylight_underlay_code,skylight_underlay_size,NULL,&skylight_underlay_ps);
    if (SUCCEEDED(hr) && !skylight_underlay_constants) {
        D3D11_BUFFER_DESC desc={0}; desc.ByteWidth=sizeof(skylight_underlay_parameters); desc.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
        D3D11_SUBRESOURCE_DATA initial={skylight_underlay_parameters,0,0};
        hr=ID3D11Device_CreateBuffer(device,&desc,&initial,&skylight_underlay_constants);
    }
    if (SUCCEEDED(hr) && !skylight_underlay_blend) {
        D3D11_BLEND_DESC blend={0};
        blend.RenderTarget[0].RenderTargetWriteMask=D3D11_COLOR_WRITE_ENABLE_ALL;
        blend.RenderTarget[0].SrcBlend=blend.RenderTarget[0].SrcBlendAlpha=D3D11_BLEND_ONE;
        blend.RenderTarget[0].DestBlend=blend.RenderTarget[0].DestBlendAlpha=D3D11_BLEND_ZERO;
        blend.RenderTarget[0].BlendOp=blend.RenderTarget[0].BlendOpAlpha=D3D11_BLEND_OP_ADD;
        hr=ID3D11Device_CreateBlendState(device,&blend,&skylight_underlay_blend);
    }
    if (SUCCEEDED(hr) && !skylight_underlay_depth) {
        D3D11_DEPTH_STENCIL_DESC depth={0}; depth.DepthFunc=D3D11_COMPARISON_ALWAYS;
        hr=ID3D11Device_CreateDepthStencilState(device,&depth,&skylight_underlay_depth);
    }
    if (SUCCEEDED(hr) && !skylight_underlay_raster) {
        D3D11_RASTERIZER_DESC raster={0}; raster.FillMode=D3D11_FILL_SOLID; raster.CullMode=D3D11_CULL_NONE; raster.DepthClipEnable=TRUE;
        hr=ID3D11Device_CreateRasterizerState(device,&raster,&skylight_underlay_raster);
    }
    ID3D11Device_Release(device);
    return SUCCEEDED(hr);
}

// Call before a draw executes. Draws once per Present, ahead of the first
// tagged (encoded) probe draw, into that draw's RT0 (the diffuse accumulator).
static void skylight_underlay(ID3D11DeviceContext *context) {
    if (!skylight_underlay_enabled || skylight_underlay_frame==present_calls) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned marker=0; UINT size=sizeof(marker);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&skylight_probe_guid,&size,&marker);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || size!=sizeof(marker) || marker<1 || marker>2) return;
    // The source SSAO creates the fullscreen VS with the scene's first depth.
    ID3D11VertexShader *fullscreen=legacy_ssao_fullscreen_shader();
    if (!fullscreen) return;
    skylight_underlay_frame=present_calls;
    ID3D11RenderTargetView *targets[8]={0}; ID3D11DepthStencilView *dsv=NULL;
    ID3D11DeviceContext_OMGetRenderTargets(context,8,targets,&dsv);
    if (!targets[0] || !init_skylight_underlay(context)) {
        ++skylight_underlay_failures; error("skylight underlay contract failed");
    } else {
        ID3D11VertexShader *vs=NULL; ID3D11GeometryShader *gs=NULL; ID3D11InputLayout *layout=NULL; D3D11_PRIMITIVE_TOPOLOGY topology;
        ID3D11RasterizerState *raster=NULL; ID3D11DepthStencilState *depth=NULL; UINT reference=0;
        ID3D11BlendState *blend=NULL; float factors[4]; UINT mask=0; ID3D11Buffer *constants=NULL;
        ID3D11PixelShader *ps=NULL;
        ID3D11DeviceContext_VSGetShader(context,&vs,NULL,NULL); ID3D11DeviceContext_GSGetShader(context,&gs,NULL,NULL);
        ID3D11DeviceContext_IAGetInputLayout(context,&layout); ID3D11DeviceContext_IAGetPrimitiveTopology(context,&topology);
        ID3D11DeviceContext_RSGetState(context,&raster); ID3D11DeviceContext_OMGetDepthStencilState(context,&depth,&reference);
        ID3D11DeviceContext_OMGetBlendState(context,&blend,factors,&mask);
        ID3D11DeviceContext_PSGetConstantBuffers(context,3,1,&constants); ID3D11DeviceContext_PSGetShader(context,&ps,NULL,NULL);
        ID3D11DeviceContext_OMSetRenderTargets(context,1,targets,NULL);
        ID3D11DeviceContext_VSSetShader(context,fullscreen,NULL,0); ID3D11DeviceContext_GSSetShader(context,NULL,NULL,0);
        ID3D11DeviceContext_IASetInputLayout(context,NULL); ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ID3D11DeviceContext_RSSetState(context,skylight_underlay_raster); ID3D11DeviceContext_OMSetDepthStencilState(context,skylight_underlay_depth,0);
        ID3D11DeviceContext_OMSetBlendState(context,skylight_underlay_blend,NULL,0xffffffff);
        ID3D11DeviceContext_PSSetConstantBuffers(context,3,1,&skylight_underlay_constants);
        ID3D11DeviceContext_PSSetShader(context,skylight_underlay_ps,NULL,0);
        original_draw(context,3,0); ++skylight_underlay_draws;
        ID3D11DeviceContext_PSSetShader(context,ps,NULL,0); ID3D11DeviceContext_PSSetConstantBuffers(context,3,1,&constants);
        ID3D11DeviceContext_OMSetBlendState(context,blend,factors,mask);
        ID3D11DeviceContext_OMSetDepthStencilState(context,depth,reference); ID3D11DeviceContext_RSSetState(context,raster);
        ID3D11DeviceContext_IASetPrimitiveTopology(context,topology); ID3D11DeviceContext_IASetInputLayout(context,layout);
        ID3D11DeviceContext_GSSetShader(context,gs,NULL,0); ID3D11DeviceContext_VSSetShader(context,vs,NULL,0);
        ID3D11DeviceContext_OMSetRenderTargets(context,8,targets,dsv);
        if (vs) ID3D11VertexShader_Release(vs);
        if (gs) ID3D11GeometryShader_Release(gs);
        if (layout) ID3D11InputLayout_Release(layout);
        if (raster) ID3D11RasterizerState_Release(raster);
        if (depth) ID3D11DepthStencilState_Release(depth);
        if (blend) ID3D11BlendState_Release(blend);
        if (constants) ID3D11Buffer_Release(constants);
        if (ps) ID3D11PixelShader_Release(ps);
    }
    for (unsigned i=0;i<8;++i) if (targets[i]) ID3D11RenderTargetView_Release(targets[i]);
    if (dsv) ID3D11DepthStencilView_Release(dsv);
    if (skylight_underlay_draws==1 || skylight_underlay_draws%600==0 || skylight_underlay_failures==1) report_skylight_underlay();
}
