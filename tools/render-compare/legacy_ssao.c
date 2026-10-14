// Archived Factory-rain SSAO in an isolated D3D11 context state. No temporal pass.
static char ssao_asset_root[MAX_PATH];
static ID3D11Device *ssao_device;
static int ssao_init_failed;
static ID3D11DeviceContext1 *ssao_context1;
static ID3DDeviceContextState *ssao_state;
static ID3D11VertexShader *ssao_vs;
static ID3D11PixelShader *ssao_accum, *ssao_upsample;
static ID3D11Texture2D *ssao_half, *ssao_full, *ssao_noise;
static ID3D11ShaderResourceView *ssao_half_view, *ssao_noise_view;
static ID3D11RenderTargetView *ssao_half_rt, *ssao_full_rt;
static ID3D11SamplerState *ssao_samplers[2];
static ID3D11Buffer *ssao_constants[2];
static ID3D11RasterizerState *ssao_raster;
static unsigned ssao_captured_id;
static float ssao_source_saturation=2.f,ssao_source_radius=1.5f;
static ID3D11Texture2D *ssao_ready_depth;
static ID3D11VertexShader *legacy_ssao_fullscreen_shader(void) { return ssao_vs; }

static void *ssao_file(const char *root,const char *name,size_t *size,int shader) {
    char path[MAX_PATH];
    if (snprintf(path,sizeof(path),"%s/%s",root,name)>=(int)sizeof(path)) return NULL;
    FILE *file=fopen(path,"rb"); if (!file) return NULL;
    if (fseek(file,0,SEEK_END)) { fclose(file); return NULL; }
    long length=ftell(file); rewind(file);
    if (length<32 || length>1024*1024) { fclose(file); return NULL; }
    void *data=malloc((size_t)length);
    int ok=data && fread(data,1,(size_t)length,file)==(size_t)length;
    fclose(file);
    if (ok && shader) ok=!memcmp(data,"DXBC",4) && *(uint32_t *)((char *)data+24)==(uint32_t)length;
    if (!ok) { free(data); return NULL; }
    *size=(size_t)length; return data;
}
static int configure_legacy_ssao(void) {
    DWORD length=GetEnvironmentVariableA("SRP_LEGACY_SSAO_DIR",ssao_asset_root,sizeof(ssao_asset_root));
    int decoder=legacy_ambient_decode_code && contains(legacy_ambient_decode_code,legacy_ambient_decode_size,"t_legacy_ssao");
    if (!!length!=decoder || length>=sizeof(ssao_asset_root)) { error("original SSAO requires its paired ambient decoder and asset directory"); return 0; }
    legacy_ssao_enabled=length!=0;
    char parameters[96],extra;
    DWORD parameter_length=GetEnvironmentVariableA("SRP_LEGACY_SSAO_PARAMETERS",parameters,sizeof(parameters));
    if (parameter_length && (!legacy_ssao_enabled || parameter_length>=sizeof(parameters) ||
        sscanf(parameters,"%f,%f%c",&ssao_source_saturation,&ssao_source_radius,&extra)!=2 ||
        !isfinite(ssao_source_saturation) || !isfinite(ssao_source_radius) ||
        ssao_source_saturation<=0 || ssao_source_radius<=0)) {
        error("source SSAO parameters require positive finite saturation,radius and paired assets"); return 0;
    }
    char early[8];
    DWORD early_length=GetEnvironmentVariableA("SRP_LEGACY_SSAO_EARLY_PROBES",early,sizeof(early));
    legacy_ssao_early=early_length==1 && early[0]=='1';
    if (legacy_ssao_early && (!legacy_ssao_enabled || strcmp(build,"026g") || !legacy_probe_code || !legacy_probe_box_code)) {
        error("early SSAO requires paired source probes and source SSAO on 026g"); return 0;
    }
    return 1;
}
static int init_legacy_ssao(ID3D11Device *device,ID3D11DeviceContext *context) {
    // install_shader_capture sees a temporary device used to find shared vtables.
    // Allocate only once the actual scene depth supplies the game's device.
    const char *root=ssao_asset_root;
    HRESULT hr; ID3D11Device1 *device1=NULL; void *data=NULL; size_t size=0;
#define SSAO_TRY(call) do { hr=(call); if (FAILED(hr)) goto fail; } while (0)
    SSAO_TRY(ID3D11Device_QueryInterface(device,&IID_ID3D11Device1,(void **)&device1));
    SSAO_TRY(ID3D11DeviceContext_QueryInterface(context,&IID_ID3D11DeviceContext1,(void **)&ssao_context1));
    D3D_FEATURE_LEVEL level=ID3D11Device_GetFeatureLevel(device);
    UINT flags=(ID3D11Device_GetCreationFlags(device)&D3D11_CREATE_DEVICE_SINGLETHREADED)?D3D11_1_CREATE_DEVICE_CONTEXT_STATE_SINGLETHREADED:0;
    SSAO_TRY(ID3D11Device1_CreateDeviceContextState(device1,flags,&level,1,D3D11_SDK_VERSION,&IID_ID3D11Device,NULL,&ssao_state));
    data=ssao_file(root,"fullscreen-vs.dxbc",&size,1); if (!data) goto fail;
    SSAO_TRY(ID3D11Device_CreateVertexShader(device,data,size,NULL,&ssao_vs)); free(data); data=NULL;
    data=ssao_file(root,"old-ssao-3.dxbc",&size,1); if (!data) goto fail;
    SSAO_TRY(original_create_ps(device,data,size,NULL,&ssao_accum)); free(data); data=NULL;
    data=ssao_file(root,"old-ssao-upsample.dxbc",&size,1); if (!data) goto fail;
    SSAO_TRY(original_create_ps(device,data,size,NULL,&ssao_upsample)); free(data); data=NULL;
    data=ssao_file(root,"old-ssao-noise.bgra",&size,0); if (!data || size!=64) goto fail;
    D3D11_TEXTURE2D_DESC texture={0};
    texture.Width=texture.Height=4; texture.MipLevels=texture.ArraySize=texture.SampleDesc.Count=1;
    texture.Format=DXGI_FORMAT_B8G8R8A8_UNORM; texture.BindFlags=D3D11_BIND_SHADER_RESOURCE;
    D3D11_SUBRESOURCE_DATA initial={data,16,0};
    SSAO_TRY(ID3D11Device_CreateTexture2D(device,&texture,&initial,&ssao_noise)); free(data); data=NULL;
    SSAO_TRY(ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)ssao_noise,NULL,&ssao_noise_view));
    texture.Width=640; texture.Height=360; texture.Format=DXGI_FORMAT_R16G16_FLOAT;
    texture.BindFlags=D3D11_BIND_SHADER_RESOURCE|D3D11_BIND_RENDER_TARGET;
    SSAO_TRY(ID3D11Device_CreateTexture2D(device,&texture,NULL,&ssao_half));
    SSAO_TRY(ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)ssao_half,NULL,&ssao_half_view));
    SSAO_TRY(ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)ssao_half,NULL,&ssao_half_rt));
    texture.Width=1280; texture.Height=720; texture.Format=DXGI_FORMAT_R8G8_UNORM;
    SSAO_TRY(ID3D11Device_CreateTexture2D(device,&texture,NULL,&ssao_full));
    SSAO_TRY(ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)ssao_full,NULL,&legacy_ssao_view));
    SSAO_TRY(ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)ssao_full,NULL,&ssao_full_rt));
    D3D11_SAMPLER_DESC sampler={0}; sampler.Filter=D3D11_FILTER_MIN_MAG_MIP_LINEAR;
    sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D11_TEXTURE_ADDRESS_WRAP;
    sampler.MaxLOD=FLT_MAX; sampler.ComparisonFunc=D3D11_COMPARISON_NEVER; sampler.MaxAnisotropy=1;
    SSAO_TRY(ID3D11Device_CreateSamplerState(device,&sampler,&ssao_samplers[0]));
    sampler.Filter=D3D11_FILTER_MIN_MAG_MIP_POINT;
    sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D11_TEXTURE_ADDRESS_CLAMP;
    SSAO_TRY(ID3D11Device_CreateSamplerState(device,&sampler,&ssao_samplers[1]));
    float params[12]={0}; params[4]=ssao_source_saturation; params[5]=ssao_source_radius;
    float globals[76]={0}; globals[72]=1280; globals[73]=720; globals[74]=1.f/1280; globals[75]=1.f/720;
    D3D11_BUFFER_DESC buffer={0}; buffer.ByteWidth=sizeof(params); buffer.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
    initial.pSysMem=params;
    SSAO_TRY(ID3D11Device_CreateBuffer(device,&buffer,&initial,&ssao_constants[0]));
    buffer.ByteWidth=sizeof(globals); initial.pSysMem=globals;
    SSAO_TRY(ID3D11Device_CreateBuffer(device,&buffer,&initial,&ssao_constants[1]));
    D3D11_RASTERIZER_DESC raster={0}; raster.FillMode=D3D11_FILL_SOLID; raster.CullMode=D3D11_CULL_NONE; raster.DepthClipEnable=TRUE;
    SSAO_TRY(ID3D11Device_CreateRasterizerState(device,&raster,&ssao_raster));
    ID3D11Device1_Release(device1);
    ssao_device=device; ID3D11Device_AddRef(device); return 1;
fail:
    ssao_init_failed=1; free(data); if (device1) ID3D11Device1_Release(device1);
    error("cannot initialize archived SSAO experiment (requires D3D11.1 context state)"); return 0;
#undef SSAO_TRY
}
static void capture_legacy_ssao_depth(ID3D11Texture2D *depth,const Camera *camera,int take) {
    if (!take || !legacy_ssao_ready || ssao_ready_present!=present_calls || ssao_captured_id==camera->id) return;
    char base[MAX_PATH+32],path[MAX_PATH+64];
    snprintf(base,sizeof(base),"%s.ssao-live",camera->output);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("[",file); dump_texture(depth,base,file,"depth",0,NULL);
        fputs(",",file); dump_texture(ssao_half,base,file,"archived_accumulation",1,NULL);
        fputs(",",file); dump_texture(ssao_full,base,file,"archived_spatial_upsample",2,NULL);
        fputs("]\n",file); fclose(file); ssao_captured_id=camera->id;
        snprintf(path,sizeof(path),"%s.state.json",base);
        file=fopen(path,"w");
        if (file) {
            fprintf(file,"{\"capture_id\":%u,\"rendered_present\":%u,\"capture_present\":%u,\"early_probes\":%s,\"source_saturation\":%.9g,\"source_radius\":%.9g}\n",
                    camera->id,ssao_ready_present,present_calls,legacy_ssao_early?"true":"false",
                    ssao_source_saturation,ssao_source_radius);
            fclose(file);
        }
    }
}
static void run_legacy_ssao_depth(ID3D11Texture2D *depth,const Camera *camera,int take) {
    if (!legacy_ssao_enabled) return;
    if (legacy_ssao_ready && ssao_ready_present==present_calls && depth==ssao_ready_depth) {
        capture_legacy_ssao_depth(depth,camera,take); return;
    }
    legacy_ssao_ready=0;
    if (!depth) { error("original SSAO depth texture null"); return; }
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(depth,&desc);
    if (desc.Width!=1280 || desc.Height!=720 || desc.Format!=DXGI_FORMAT_R16_FLOAT || desc.ArraySize!=1 || desc.SampleDesc.Count!=1) {
        error("original SSAO requires single-sample 1280x720 R16F depth"); return;
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext *context=NULL; ID3D11ShaderResourceView *depth_view=NULL;
    ID3D11Texture2D_GetDevice(depth,&device); ID3D11Device_GetImmediateContext(device,&context);
    if ((!ssao_device && (ssao_init_failed || !init_legacy_ssao(device,context))) || (ssao_device && ssao_device!=device)) {
        error("original SSAO device initialization failed or scene device changed");
        ID3D11DeviceContext_Release(context); ID3D11Device_Release(device); return;
    }
    HRESULT hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)depth,NULL,&depth_view);
    if (SUCCEEDED(hr)) {
        ID3DDeviceContextState *previous=NULL;
        ID3D11DeviceContext1_SwapDeviceContextState(ssao_context1,ssao_state,&previous);
        ID3D11DeviceContext_ClearState(context);
        ID3D11DeviceContext_VSSetShader(context,ssao_vs,NULL,0);
        ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ID3D11DeviceContext_RSSetState(context,ssao_raster);
        ID3D11DeviceContext_PSSetSamplers(context,0,2,ssao_samplers);
        ID3D11ShaderResourceView *inputs[2]={depth_view,ssao_noise_view};
        ID3D11DeviceContext_PSSetShaderResources(context,0,2,inputs);
        ID3D11DeviceContext_PSSetConstantBuffers(context,0,2,ssao_constants);
        D3D11_VIEWPORT vp={0,0,640,360,0,1}; const float clear[4]={0};
        ID3D11DeviceContext_RSSetViewports(context,1,&vp);
        ID3D11DeviceContext_OMSetRenderTargets(context,1,&ssao_half_rt,NULL);
        ID3D11DeviceContext_ClearRenderTargetView(context,ssao_half_rt,clear);
        ID3D11DeviceContext_PSSetShader(context,ssao_accum,NULL,0);
        original_draw(context,3,0);
        ID3D11DeviceContext_OMSetRenderTargets(context,0,NULL,NULL);
        ID3D11DeviceContext_PSSetShaderResources(context,1,1,&ssao_half_view);
        ID3D11DeviceContext_PSSetConstantBuffers(context,0,1,&ssao_constants[1]);
        vp.Width=1280; vp.Height=720;
        ID3D11DeviceContext_RSSetViewports(context,1,&vp);
        ID3D11DeviceContext_OMSetRenderTargets(context,1,&ssao_full_rt,NULL);
        ID3D11DeviceContext_ClearRenderTargetView(context,ssao_full_rt,clear);
        ID3D11DeviceContext_PSSetShader(context,ssao_upsample,NULL,0);
        original_draw(context,3,0);
        ID3D11DeviceContext_ClearState(context);
        ID3D11DeviceContext1_SwapDeviceContextState(ssao_context1,previous,NULL);
        if (previous) ID3DDeviceContextState_Release(previous);
        legacy_ssao_ready=1; ssao_ready_present=present_calls;
        if (ssao_ready_depth!=depth) {
            if (ssao_ready_depth) ID3D11Texture2D_Release(ssao_ready_depth);
            ssao_ready_depth=depth; ID3D11Texture2D_AddRef(depth);
        }
        capture_legacy_ssao_depth(depth,camera,take);
    } else error("cannot bind original SSAO depth");
    if (depth_view) ID3D11ShaderResourceView_Release(depth_view);
    ID3D11DeviceContext_Release(context); ID3D11Device_Release(device);
}

static void run_legacy_ssao(unsigned char *targets,const Camera *camera,int take) {
    if (!legacy_ssao_enabled) return;
    if (!readable(targets,10*0xa0) || strcmp((char *)targets+9*0xa0+0xc,"$user$position")) {
        error("original SSAO depth target missing"); return;
    }
    unsigned char *target=*(void **)(targets+9*0xa0+0x98);
    if (!readable(target,0x10)) { error("original SSAO depth texture missing"); return; }
    ID3D11Texture2D *depth=*(void **)(target+0xc);
    if (!depth) { error("original SSAO depth texture null"); return; }
    run_legacy_ssao_depth(depth,camera,take);
}
// Called before the verified source probe draws, while their current G-buffer
// depth is already bound. The later stage entry reuses this Present's result.
static void prepare_early_legacy_ssao(ID3D11DeviceContext *context) {
    if (!legacy_ssao_early) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned marker=0; UINT size=sizeof(marker);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&early_ssao_probe_guid,&size,&marker);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || size!=sizeof(marker) || marker<1 || marker>2) return;
    ID3D11ShaderResourceView *view=NULL; ID3D11DeviceContext_PSGetShaderResources(context,2,1,&view);
    if (!view) { error("early SSAO probe depth missing"); return; }
    D3D11_SHADER_RESOURCE_VIEW_DESC desc; ID3D11ShaderResourceView_GetDesc(view,&desc);
    ID3D11Resource *resource=NULL; ID3D11Texture2D *depth=NULL;
    ID3D11ShaderResourceView_GetResource(view,&resource);
    hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&depth);
    ID3D11Resource_Release(resource); ID3D11ShaderResourceView_Release(view);
    if (FAILED(hr) || desc.Format!=DXGI_FORMAT_R16_FLOAT || desc.ViewDimension!=D3D11_SRV_DIMENSION_TEXTURE2D || desc.Texture2D.MostDetailedMip) {
        if (depth) ID3D11Texture2D_Release(depth);
        error("early SSAO probe depth view contract changed"); return;
    }
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id && GetTickCount()-applied_at>=camera.settle_ms;
    LeaveCriticalSection(&lock);
    run_legacy_ssao_depth(depth,&camera,take); ID3D11Texture2D_Release(depth);
}
