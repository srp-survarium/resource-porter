// Diagnostic source sun and optional material-gated translucency on native inputs.
// The result stays private until original shadow/stencil equivalence is proved.
static void *sun_aux_code[2];
static size_t sun_aux_size[2];
static float sun_aux_values[8];
static ID3D11PixelShader *sun_aux_shaders[2];
static ID3D11Buffer *sun_aux_constants;
static ID3D11BlendState *sun_aux_blend;
static ID3D11DepthStencilState *sun_aux_depth[1];
static unsigned sun_aux_frame,sun_aux_draws,sun_aux_capture_id;
static int sun_aux_enabled,sun_aux_ready,sun_aux_translucency;
static ID3D11ShaderResourceView *source_full_diffuse_view(void) {
    return sun_aux_enabled && sun_aux_translucency && sun_aux_ready && sun_aux_frame==present_calls &&
        sun_aux_draws==2 && source_ambient_valid() && !legacy_local_failures && !legacy_local_unhandled
        ?legacy_local_diffuse_view:NULL;
}
static int configure_sun_aux(void) {
    char directory[MAX_PATH],path[MAX_PATH];
    DWORD length=GetEnvironmentVariableA("SRP_DIAGNOSTIC_SOURCE_SUN_DIR",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || strcmp(build,"026g") || !legacy_ambient_diffuse_enabled || !legacy_sun_projection_code) {
        error("source sun diagnostic requires source ambient/local and sun projection on 026g"); return 0;
    }
    char gate[8];
    sun_aux_translucency=GetEnvironmentVariableA("SRP_DIAGNOSTIC_TRANSLUCENCY_PARAMETER_GATE",gate,sizeof(gate))==1 && gate[0]=='1';
    const char *names[]={"legacy_sun_diffuse_encoded.dxbc","legacy_translucency_encoded.dxbc"};
    const size_t sizes[]={3000,3340};
    const unsigned char digests[2][16]={
        {0xc2,0xde,0x59,0x5e,0x39,0x58,0xfb,0xa5,0x61,0x98,0xb2,0xf7,0x4b,0x9b,0x12,0xfc},
        {0x4e,0xa9,0xa9,0xd8,0xfe,0x79,0x84,0x3f,0xc0,0x20,0x99,0xf9,0x6e,0x02,0x98,0x98}
    };
    for (unsigned i=0;i<(sun_aux_translucency?2u:1u);++i) {
        if (snprintf(path,sizeof(path),"%s/%s",directory,names[i])>=(int)sizeof(path)) return 0;
        FILE *file=fopen(path,"rb"); if (!file) { error("source sun auxiliary shader missing"); return 0; }
        sun_aux_code[i]=malloc(sizes[i]); sun_aux_size[i]=sizes[i];
        int ok=sun_aux_code[i] && fread(sun_aux_code[i],1,sizes[i],file)==sizes[i] && fgetc(file)==EOF;
        fclose(file);
        if (!ok || memcmp(sun_aux_code[i],"DXBC",4) || memcmp((char *)sun_aux_code[i]+4,digests[i],16) ||
            *(uint32_t *)((char *)sun_aux_code[i]+24)!=sizes[i]) {
            error("source sun auxiliary shader identity mismatch"); return 0;
        }
    }
    if (snprintf(path,sizeof(path),"%s/source-sun-diffuse.bin",directory)>=(int)sizeof(path)) return 0;
    FILE *file=fopen(path,"rb"); if (!file) { error("source sun auxiliary constants missing"); return 0; }
    int ok=fread(sun_aux_values,1,sizeof(sun_aux_values),file)==sizeof(sun_aux_values) && fgetc(file)==EOF;
    fclose(file);
    for (unsigned i=0;i<8;++i) if (!isfinite(sun_aux_values[i]) || sun_aux_values[i]<0) ok=0;
    if (!ok || sun_aux_values[3]>1) { error("invalid source sun auxiliary constants"); return 0; }
    sun_aux_enabled=1; return 1;
}
static int init_sun_aux(ID3D11DeviceContext *context) {
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device); HRESULT hr=S_OK;
    for (unsigned i=0;i<(sun_aux_translucency?2u:1u) && SUCCEEDED(hr);++i)
        if (!sun_aux_shaders[i]) hr=original_create_ps(device,sun_aux_code[i],sun_aux_size[i],NULL,&sun_aux_shaders[i]);
    if (SUCCEEDED(hr) && !sun_aux_constants) {
        D3D11_BUFFER_DESC desc={0}; desc.ByteWidth=sizeof(sun_aux_values); desc.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
        D3D11_SUBRESOURCE_DATA initial={sun_aux_values,0,0};
        hr=ID3D11Device_CreateBuffer(device,&desc,&initial,&sun_aux_constants);
    }
    if (SUCCEEDED(hr) && !sun_aux_blend) {
        D3D11_BLEND_DESC blend={0}; D3D11_RENDER_TARGET_BLEND_DESC *b=&blend.RenderTarget[0];
        b->BlendEnable=TRUE; b->SrcBlend=b->DestBlend=b->SrcBlendAlpha=b->DestBlendAlpha=D3D11_BLEND_ONE;
        b->BlendOp=b->BlendOpAlpha=D3D11_BLEND_OP_ADD; b->RenderTargetWriteMask=7;
        hr=ID3D11Device_CreateBlendState(device,&blend,&sun_aux_blend);
    }
    for (unsigned i=0;i<1 && SUCCEEDED(hr);++i) if (!sun_aux_depth[i]) {
        D3D11_DEPTH_STENCIL_DESC depth={0}; depth.DepthFunc=D3D11_COMPARISON_ALWAYS;
        depth.StencilEnable=TRUE; depth.StencilReadMask=255;
        depth.FrontFace.StencilFunc=D3D11_COMPARISON_NOT_EQUAL;
        depth.FrontFace.StencilFailOp=depth.FrontFace.StencilDepthFailOp=depth.FrontFace.StencilPassOp=D3D11_STENCIL_OP_KEEP;
        depth.BackFace=depth.FrontFace;
        hr=ID3D11Device_CreateDepthStencilState(device,&depth,&sun_aux_depth[i]);
    }
    ID3D11Device_Release(device); return SUCCEEDED(hr);
}
static void capture_sun_aux_step(const Camera *camera,unsigned step) {
    char base[MAX_PATH+48],path[MAX_PATH+80]; snprintf(base,sizeof(base),"%s.sun-encoded-%u",camera->output,step);
    snprintf(path,sizeof(path),"%s.targets.json",base); FILE *file=fopen(path,"w");
    if (!file) { error("cannot save sun auxiliary targets"); return; }
    fputs("[",file); dump_texture(legacy_local_diffuse_texture,base,file,"encoded_diffuse",0,NULL);
    if (!step) { fputs(",",file); dump_texture(legacy_local_depth_texture,base,file,"native_depth_stencil_at_composition",1,NULL); }
    fputs("]\n",file); fclose(file);
}
static void render_sun_aux(ID3D11DeviceContext *context,UINT count,UINT start,INT base,int indexed) {
    if (!sun_aux_enabled) return;
    if (sun_aux_frame!=present_calls) { sun_aux_frame=present_calls; sun_aux_draws=0; sun_aux_ready=0; }
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&composition_guid,&size,&flag);
    if (FAILED(hr) || flag!=1 || sun_aux_draws) { ID3D11PixelShader_Release(shader); return; }
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id && sun_aux_capture_id!=camera.id && GetTickCount()-applied_at>=camera.settle_ms;
    LeaveCriticalSection(&lock);
    // Loading/menu composition precedes the first scene probe. It has no
    // source accumulator to extend; an armed settled capture must have one.
    if (ambient_frame!=present_calls || !ambient_probe_draws) {
        if (take) error("source sun capture has no initialized scene ambient");
        ID3D11PixelShader_Release(shader); return;
    }
    capture_ambient_diffuse(context);
    if (!source_ambient_valid() || legacy_local_failures || legacy_local_unhandled || !legacy_scene_depth_view ||
        legacy_scene_depth_frame!=present_calls || !legacy_sun_projection_draws || legacy_sun_projection_frame!=present_calls ||
        !init_sun_aux(context)) {
        error("source sun auxiliary frame inputs not ready"); ID3D11PixelShader_Release(shader); return;
    }
    // Read the actual native stencil AFTER all local-light inversions. The last
    // private pre-light snapshot is not equivalent to this final stencil.
    ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL;
    ID3D11DepthStencilView_GetResource(legacy_scene_depth_view,&resource);
    hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource);
    if (FAILED(hr)) { error("source sun native depth resource missing"); ID3D11PixelShader_Release(shader); return; }
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc);
    if (desc.Width!=legacy_local_depth_desc.Width || desc.Height!=legacy_local_depth_desc.Height ||
        desc.Format!=legacy_local_depth_desc.Format || desc.SampleDesc.Count!=1) {
        error("source sun native depth contract changed"); ID3D11Texture2D_Release(texture); ID3D11PixelShader_Release(shader); return;
    }
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)legacy_local_depth_texture,(ID3D11Resource *)texture);
    ID3D11Texture2D_Release(texture);
    if (take) { capture_local_diffuse(context,&camera); capture_sun_aux_step(&camera,0); }
    ID3D11RenderTargetView *targets[8]={0}; ID3D11DepthStencilView *depth=NULL;
    ID3D11DepthStencilState *depth_state=NULL; UINT reference,mask; ID3D11BlendState *blend=NULL; float factors[4];
    ID3D11Buffer *constants=NULL;
    ID3D11DeviceContext_OMGetRenderTargets(context,8,targets,&depth);
    ID3D11DeviceContext_OMGetDepthStencilState(context,&depth_state,&reference);
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factors,&mask);
    ID3D11DeviceContext_PSGetConstantBuffers(context,3,1,&constants);
    ID3D11DeviceContext_OMSetRenderTargets(context,1,&legacy_local_diffuse_target,legacy_local_depth_view);
    ID3D11DeviceContext_OMSetBlendState(context,sun_aux_blend,NULL,0xffffffff);
    ID3D11DeviceContext_PSSetConstantBuffers(context,3,1,&sun_aux_constants);
    for (unsigned i=0;i<(sun_aux_translucency?2u:1u);++i) {
        ID3D11DeviceContext_OMSetDepthStencilState(context,sun_aux_depth[0],0);
        ID3D11DeviceContext_PSSetShader(context,sun_aux_shaders[i],NULL,0);
        if (indexed) original_draw_indexed(context,count,start,base); else original_draw(context,count,start);
        ++sun_aux_draws;
        if (take) capture_sun_aux_step(&camera,i+1);
    }
    sun_aux_ready=1;
    ID3D11DeviceContext_PSSetConstantBuffers(context,3,1,&constants);
    ID3D11DeviceContext_PSSetShader(context,shader,NULL,0);
    ID3D11DeviceContext_OMSetRenderTargets(context,8,targets,depth);
    ID3D11DeviceContext_OMSetDepthStencilState(context,depth_state,reference);
    ID3D11DeviceContext_OMSetBlendState(context,blend,factors,mask);
    if (take) {
        char path[MAX_PATH+64]; snprintf(path,sizeof(path),"%s.sun-encoded.json",camera.output); FILE *file=fopen(path,"w");
        if (file) {
            fprintf(file,"{\"present\":%u,\"draws\":%u,\"ready\":true,\"native_depth_present\":%u,\"projection_present\":%u,\"scope\":\"source sun; optional translucency uses native positive parameter and nonzero geometry, not original stencil; private diagnostic only\",\"source_constants\":[",present_calls,sun_aux_draws,legacy_scene_depth_frame,legacy_sun_projection_frame);
            for (unsigned i=0;i<8;++i) fprintf(file,"%s%.9g",i?",":"",sun_aux_values[i]);
            fputs("]}\n",file); fclose(file);
        }
        sun_aux_capture_id=camera.id;
    }
    if (constants) ID3D11Buffer_Release(constants);
    if (blend) ID3D11BlendState_Release(blend);
    if (depth_state) ID3D11DepthStencilState_Release(depth_state);
    if (depth) ID3D11DepthStencilView_Release(depth);
    for (unsigned i=0;i<8;++i) if (targets[i]) ID3D11RenderTargetView_Release(targets[i]);
    ID3D11PixelShader_Release(shader);
}
