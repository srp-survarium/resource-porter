// Optional source 010b ambient initialization of the private encoded target.
// Native MRT probe draws remain intact; each replay has a pre-draw DSV copy.
static void *ambient_code[3];
static size_t ambient_code_size[3];
static float ambient_sky_parameters[32];
static ID3D11PixelShader *ambient_shaders[3];
static ID3D11Buffer *ambient_sky_constants;
static ID3D11BlendState *ambient_probe_blend,*ambient_sky_blend;
static ID3D11DepthStencilState *ambient_sky_depth;
static ID3D11RasterizerState *ambient_sky_raster;
static unsigned ambient_frame,ambient_probe_draws,ambient_failures,ambient_sky_draws;
static unsigned ambient_capture_id;
static unsigned ambient_sky_capture_id;
static int ambient_sealed,ambient_last_mip;
typedef struct { LocalDiffuseDraw draw; unsigned kind; } AmbientDiffuseDraw;
static int source_ambient_valid(void) {
    return legacy_ambient_diffuse_enabled && ambient_frame==present_calls && ambient_sky_draws==1 &&
        ambient_probe_draws>0 && !ambient_failures && legacy_ssao_ready && ssao_ready_present==present_calls;
}
static void capture_ambient_sky_output(ID3D11DeviceContext *context,unsigned kind) {
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id && GetTickCount()-applied_at>=camera.settle_ms;
    LeaveCriticalSection(&lock);
    if (!take || ambient_sky_capture_id==camera.id) return;
    char base[MAX_PATH+48],path[MAX_PATH+80]; snprintf(base,sizeof(base),"%s.sky-encoded",camera.output);
    snprintf(path,sizeof(path),"%s.targets.json",base); FILE *file=fopen(path,"w");
    if (file) { fputs("[",file); dump_texture(legacy_local_diffuse_texture,base,file,"source_encoded_skylight",0,NULL); fputs("]\n",file); fclose(file); }
    snprintf(path,sizeof(path),"%s.json",base); file=fopen(path,"w");
    if (file) {
        fprintf(file,"{\"present\":%u,\"ao_present\":%u,\"probe_kind\":%u,\"scope\":\"source skylight before first probe; inputs from matching first probe-draw and ssao-live\",\"source_stencil\":{\"reference\":0,\"read_mask\":255,\"write_mask\":0,\"comparison\":6},\"native_probe_state\":",present_calls,ssao_ready_present,kind);
        ID3D11DepthStencilState *state=NULL; UINT reference=0;
        ID3D11DeviceContext_OMGetDepthStencilState(context,&state,&reference);
        if (state) {
            D3D11_DEPTH_STENCIL_DESC d; ID3D11DepthStencilState_GetDesc(state,&d);
            fprintf(file,"{\"reference\":%u,\"depth\":[%u,%u,%u],\"stencil\":[%u,%u,%u],\"front\":[%u,%u,%u,%u],\"back\":[%u,%u,%u,%u]}",
                    reference,d.DepthEnable,d.DepthWriteMask,d.DepthFunc,d.StencilEnable,d.StencilReadMask,d.StencilWriteMask,
                    d.FrontFace.StencilFailOp,d.FrontFace.StencilDepthFailOp,d.FrontFace.StencilPassOp,d.FrontFace.StencilFunc,
                    d.BackFace.StencilFailOp,d.BackFace.StencilDepthFailOp,d.BackFace.StencilPassOp,d.BackFace.StencilFunc);
            ID3D11DepthStencilState_Release(state);
        } else fputs("null",file);
        fputs("}\n",file);
        fclose(file);
    }
    ambient_sky_capture_id=camera.id;
}

static int configure_ambient_diffuse(void) {
    char directory[MAX_PATH],path[MAX_PATH];
    DWORD length=GetEnvironmentVariableA("SRP_LEGACY_AMBIENT_DIFFUSE_DIR",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || !legacy_ssao_early || !legacy_local_diffuse_code || strcmp(build,"026g")) {
        error("source ambient replay requires early SSAO and source local replay on 026g"); return 0;
    }
    const char *names[]={"legacy-probe-ambient-aux.dxbc","legacy-probe-box-ambient-aux.dxbc","legacy-skylight-ambient-aux.dxbc"};
    static const unsigned char digests[3][16]={
        {0xa4,0x1b,0x35,0x08,0x95,0xa5,0xa5,0xa9,0x6e,0x40,0xff,0x0d,0xa7,0x85,0x71,0xd5},
        {0x34,0xdf,0x0e,0xd1,0x2c,0x58,0x80,0xe0,0x36,0x06,0x5e,0x14,0xe2,0xc3,0x8a,0x8e},
        {0x09,0xf0,0x60,0x97,0xa6,0x02,0xb4,0x49,0x2c,0x83,0x50,0x73,0x79,0x7a,0x15,0x45}
    };
    static const unsigned char last_mip_digests[2][16]={
        {0xf7,0xb4,0x40,0xcc,0x9e,0x44,0xe7,0xdc,0x10,0x26,0xc6,0x51,0x4f,0x2f,0x86,0xb7},
        {0x80,0xed,0x6f,0x07,0x0d,0xd3,0x33,0x42,0xeb,0x46,0xcf,0x2c,0xb0,0xbb,0xb5,0x0c}
    };
    char flag[8];
    ambient_last_mip=GetEnvironmentVariableA("SRP_DIAGNOSTIC_SOURCE_AMBIENT_LAST_MIP",flag,sizeof(flag))==1 && flag[0]=='1';
    for (unsigned i=0;i<3;++i) {
        if (snprintf(path,sizeof(path),"%s/%s",directory,names[i])>=(int)sizeof(path)) return 0;
        FILE *file=fopen(path,"rb"); if (!file) { error("source ambient shader missing"); return 0; }
        int ok=!fseek(file,0,SEEK_END); long size=ftell(file); rewind(file);
        if (!ok || size<32 || size>1024*1024) { fclose(file); return 0; }
        ambient_code[i]=malloc((size_t)size); ambient_code_size[i]=(size_t)size;
        ok=ambient_code[i] && fread(ambient_code[i],1,(size_t)size,file)==(size_t)size;
        fclose(file);
        const unsigned char *digest=ambient_last_mip && i<2?last_mip_digests[i]:digests[i];
        if (!ok || memcmp(ambient_code[i],"DXBC",4) || memcmp((char *)ambient_code[i]+4,digest,16) ||
            *(uint32_t *)((char *)ambient_code[i]+24)!=(uint32_t)size) {
            error("source ambient shader identity mismatch"); return 0;
        }
    }
    if (snprintf(path,sizeof(path),"%s/source-skylight.bin",directory)>=(int)sizeof(path)) return 0;
    FILE *file=fopen(path,"rb"); if (!file) { error("source sky constants missing"); return 0; }
    int ok=fread(ambient_sky_parameters,1,sizeof(ambient_sky_parameters),file)==sizeof(ambient_sky_parameters) && fgetc(file)==EOF;
    fclose(file);
    for (unsigned i=0;i<32;++i) if (!isfinite(ambient_sky_parameters[i])) ok=0;
    if (!ok || ambient_sky_parameters[5]<=0 || ambient_sky_parameters[6]<0 || fabsf(ambient_sky_parameters[24])>1) {
        error("source sky constants contract failed"); return 0;
    }
    legacy_ambient_diffuse_enabled=1; return 1;
}
static int init_ambient_diffuse(ID3D11DeviceContext *context) {
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    HRESULT hr=S_OK;
    for (unsigned i=0;i<3 && SUCCEEDED(hr);++i)
        if (!ambient_shaders[i]) hr=original_create_ps(device,ambient_code[i],ambient_code_size[i],NULL,&ambient_shaders[i]);
    if (SUCCEEDED(hr) && !ambient_sky_constants) {
        D3D11_BUFFER_DESC desc={0}; desc.ByteWidth=sizeof(ambient_sky_parameters); desc.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
        D3D11_SUBRESOURCE_DATA initial={ambient_sky_parameters,0,0};
        hr=ID3D11Device_CreateBuffer(device,&desc,&initial,&ambient_sky_constants);
    }
    D3D11_BLEND_DESC blend={0};
    blend.RenderTarget[0].RenderTargetWriteMask=7;
    blend.RenderTarget[0].SrcBlend=blend.RenderTarget[0].SrcBlendAlpha=D3D11_BLEND_ONE;
    blend.RenderTarget[0].DestBlend=blend.RenderTarget[0].DestBlendAlpha=D3D11_BLEND_ZERO;
    blend.RenderTarget[0].BlendOp=blend.RenderTarget[0].BlendOpAlpha=D3D11_BLEND_OP_ADD;
    if (SUCCEEDED(hr) && !ambient_sky_blend) hr=ID3D11Device_CreateBlendState(device,&blend,&ambient_sky_blend);
    blend.RenderTarget[0].BlendEnable=TRUE;
    blend.RenderTarget[0].SrcBlend=D3D11_BLEND_SRC_ALPHA; blend.RenderTarget[0].DestBlend=D3D11_BLEND_INV_SRC_ALPHA;
    if (SUCCEEDED(hr) && !ambient_probe_blend) hr=ID3D11Device_CreateBlendState(device,&blend,&ambient_probe_blend);
    if (SUCCEEDED(hr) && !ambient_sky_depth) {
        D3D11_DEPTH_STENCIL_DESC depth={0}; depth.DepthFunc=D3D11_COMPARISON_ALWAYS;
        depth.StencilEnable=TRUE; depth.StencilReadMask=0xff;
        depth.FrontFace.StencilFunc=depth.BackFace.StencilFunc=D3D11_COMPARISON_NOT_EQUAL;
        depth.FrontFace.StencilFailOp=depth.FrontFace.StencilDepthFailOp=depth.FrontFace.StencilPassOp=D3D11_STENCIL_OP_KEEP;
        depth.BackFace=depth.FrontFace;
        hr=ID3D11Device_CreateDepthStencilState(device,&depth,&ambient_sky_depth);
    }
    if (SUCCEEDED(hr) && !ambient_sky_raster) {
        D3D11_RASTERIZER_DESC raster={0}; raster.FillMode=D3D11_FILL_SOLID; raster.CullMode=D3D11_CULL_NONE; raster.DepthClipEnable=TRUE;
        hr=ID3D11Device_CreateRasterizerState(device,&raster,&ambient_sky_raster);
    }
    ID3D11Device_Release(device); return SUCCEEDED(hr);
}
static void draw_ambient_sky(ID3D11DeviceContext *context) {
    ID3D11VertexShader *vs=NULL; ID3D11GeometryShader *gs=NULL; ID3D11InputLayout *layout=NULL;
    ID3D11RasterizerState *raster=NULL; ID3D11DepthStencilState *depth=NULL;
    ID3D11Buffer *constants=NULL; UINT reference=0; D3D11_PRIMITIVE_TOPOLOGY topology;
    ID3D11DeviceContext_VSGetShader(context,&vs,NULL,NULL); ID3D11DeviceContext_GSGetShader(context,&gs,NULL,NULL);
    ID3D11DeviceContext_IAGetInputLayout(context,&layout); ID3D11DeviceContext_IAGetPrimitiveTopology(context,&topology);
    ID3D11DeviceContext_RSGetState(context,&raster); ID3D11DeviceContext_OMGetDepthStencilState(context,&depth,&reference);
    ID3D11DeviceContext_PSGetConstantBuffers(context,3,1,&constants);
    ID3D11DeviceContext_VSSetShader(context,legacy_ssao_fullscreen_shader(),NULL,0);
    ID3D11DeviceContext_GSSetShader(context,NULL,NULL,0); ID3D11DeviceContext_IASetInputLayout(context,NULL);
    ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_RSSetState(context,ambient_sky_raster); ID3D11DeviceContext_OMSetDepthStencilState(context,ambient_sky_depth,0);
    ID3D11DeviceContext_OMSetBlendState(context,ambient_sky_blend,NULL,0xffffffff);
    ID3D11DeviceContext_PSSetConstantBuffers(context,3,1,&ambient_sky_constants);
    ID3D11DeviceContext_PSSetShader(context,ambient_shaders[2],NULL,0);
    original_draw(context,3,0); ++ambient_sky_draws;
    ID3D11DeviceContext_PSSetConstantBuffers(context,3,1,&constants);
    ID3D11DeviceContext_VSSetShader(context,vs,NULL,0); ID3D11DeviceContext_GSSetShader(context,gs,NULL,0);
    ID3D11DeviceContext_IASetInputLayout(context,layout); ID3D11DeviceContext_IASetPrimitiveTopology(context,topology);
    ID3D11DeviceContext_RSSetState(context,raster); ID3D11DeviceContext_OMSetDepthStencilState(context,depth,reference);
    if (vs) ID3D11VertexShader_Release(vs);
    if (gs) ID3D11GeometryShader_Release(gs);
    if (layout) ID3D11InputLayout_Release(layout);
    if (raster) ID3D11RasterizerState_Release(raster);
    if (depth) ID3D11DepthStencilState_Release(depth);
    if (constants) ID3D11Buffer_Release(constants);
}
static void prepare_ambient_diffuse(ID3D11DeviceContext *context,AmbientDiffuseDraw *state) {
    memset(state,0,sizeof(*state)); if (!legacy_ambient_diffuse_enabled) return;
    begin_local_diffuse_frame(context);
    if (ambient_frame!=present_calls) {
        ambient_frame=present_calls; ambient_probe_draws=ambient_failures=ambient_sky_draws=0; ambient_sealed=0;
    }
    ID3D11DeviceContext_PSGetShader(context,&state->draw.shader,NULL,NULL);
    if (!state->draw.shader) return;
    UINT size=sizeof(state->kind);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(state->draw.shader,&early_ssao_probe_guid,&size,&state->kind);
    if (FAILED(hr) || size!=sizeof(state->kind) || !state->kind) { release_local_draw(&state->draw); return; }
    if (state->kind>2) {
        ++ambient_failures; error("unhandled source ambient probe identity"); release_local_draw(&state->draw); return;
    }
    if (ambient_last_mip) {
        ID3D11ShaderResourceView *view=NULL; ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL;
        D3D11_SHADER_RESOURCE_VIEW_DESC srv; D3D11_TEXTURE2D_DESC desc; int valid=0;
        ID3D11DeviceContext_PSGetShaderResources(context,5,1,&view);
        if (view) {
            ID3D11ShaderResourceView_GetDesc(view,&srv);
            ID3D11ShaderResourceView_GetResource(view,&resource);
            if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) {
                ID3D11Texture2D_GetDesc(texture,&desc);
                valid=srv.ViewDimension==D3D11_SRV_DIMENSION_TEXTURECUBE && srv.Format==DXGI_FORMAT_BC3_UNORM &&
                    srv.TextureCube.MostDetailedMip==0 && srv.TextureCube.MipLevels==3 &&
                    desc.Width==16 && desc.Height==16 && desc.ArraySize==6 && desc.MipLevels==3;
                ID3D11Texture2D_Release(texture);
            }
            ID3D11Resource_Release(resource); ID3D11ShaderResourceView_Release(view);
        }
        if (!valid) {
            ++ambient_failures; error("source last-mip diffuse cube contract changed"); release_local_draw(&state->draw); return;
        }
    }
    ID3D11DeviceContext_OMGetRenderTargets(context,8,state->draw.targets,&state->draw.depth);
    if (ambient_sealed || !legacy_ssao_ready || ssao_ready_present!=present_calls || !legacy_ssao_fullscreen_shader() ||
        !state->draw.targets[0] || !state->draw.depth || !prepare_local_targets(context,&state->draw) || !init_ambient_diffuse(context)) {
        ++ambient_failures; error("source ambient replay contract failed"); release_local_draw(&state->draw); return;
    }
    state->draw.ready=1;
}
static void finish_ambient_diffuse(ID3D11DeviceContext *context,AmbientDiffuseDraw *state,UINT count,UINT start,INT base,int indexed) {
    if (state->draw.ready) {
        ID3D11ShaderResourceView *previous_ao=NULL; ID3D11BlendState *blend=NULL; float factors[4]; UINT mask;
        ID3D11DeviceContext_PSGetShaderResources(context,14,1,&previous_ao);
        ID3D11DeviceContext_OMGetBlendState(context,&blend,factors,&mask);
        ID3D11DeviceContext_PSSetShaderResources(context,14,1,&legacy_ssao_view);
        ID3D11DeviceContext_OMSetRenderTargets(context,1,&legacy_local_diffuse_target,legacy_local_depth_view);
        if (!ambient_sky_draws) {
            draw_ambient_sky(context);
            capture_ambient_sky_output(context,state->kind);
        }
        ID3D11DeviceContext_OMSetBlendState(context,ambient_probe_blend,NULL,0xffffffff);
        ID3D11DeviceContext_PSSetShader(context,ambient_shaders[state->kind-1],NULL,0);
        if (indexed) original_draw_indexed(context,count,start,base); else original_draw(context,count,start);
        ++ambient_probe_draws;
        ID3D11DeviceContext_PSSetShaderResources(context,14,1,&previous_ao);
        ID3D11DeviceContext_OMSetBlendState(context,blend,factors,mask);
        ID3D11DeviceContext_PSSetShader(context,state->draw.shader,NULL,0);
        ID3D11DeviceContext_OMSetRenderTargets(context,8,state->draw.targets,state->draw.depth);
        if (previous_ao) ID3D11ShaderResourceView_Release(previous_ao);
        if (blend) ID3D11BlendState_Release(blend);
    }
    release_local_draw(&state->draw);
}
// Seal before the first local light. Future source sun/scattering helpers must
// likewise seal before adding their contribution, including frames with no locals.
static void capture_ambient_diffuse(ID3D11DeviceContext *context) {
    (void)context;
    if (!legacy_ambient_diffuse_enabled || ambient_sealed) return;
    ambient_sealed=1;
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id && GetTickCount()-applied_at>=camera.settle_ms;
    LeaveCriticalSection(&lock);
    if (!take || ambient_capture_id==camera.id) return;
    char base[MAX_PATH+48],path[MAX_PATH+80]; snprintf(base,sizeof(base),"%s.ambient-encoded",camera.output);
    snprintf(path,sizeof(path),"%s.json",base); FILE *file=fopen(path,"w");
    if (file) {
        fprintf(file,"{\"present\":%u,\"valid\":%s,\"sky_draws\":%u,\"probe_draws\":%u,\"failed_draws\":%u,\"ao_present\":%u,\"diffuse_cube_mip\":%u,\"scope\":\"source sky and probes before local lights; native scene unchanged\",\"source_sky\":[",
                present_calls,source_ambient_valid()?"true":"false",ambient_sky_draws,ambient_probe_draws,ambient_failures,ssao_ready_present,ambient_last_mip?2:1);
        for (unsigned i=0;i<32;++i) fprintf(file,"%s%.9g",i?",":"",ambient_sky_parameters[i]);
        fputs("]}\n",file); fclose(file);
    }
    if (legacy_local_diffuse_texture) {
        snprintf(path,sizeof(path),"%s.targets.json",base); file=fopen(path,"w");
        if (file) { fputs("[",file); dump_texture(legacy_local_diffuse_texture,base,file,"source_encoded_ambient_diffuse",0,NULL); fputs("]\n",file); fclose(file); }
    }
    ambient_capture_id=camera.id;
}
