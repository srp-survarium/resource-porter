// Original 011e SSAO + skylight on current native inputs, into private targets.
// Diagnostic only: native probes/composition are not replaced by this module.
typedef struct {
    PostReplay *passes[4];
    struct {
        enum { AMBIENT_HISTORY_EMPTY, AMBIENT_HISTORY_VALID } phase;
        union { struct { unsigned present,camera;float view[16]; } valid; } data;
    } history;
} OriginalAmbientReady;
static struct {
    enum { ORIGINAL_AMBIENT_OFF, ORIGINAL_AMBIENT_WAITING, ORIGINAL_AMBIENT_READY, ORIGINAL_AMBIENT_FAILED } phase;
    union {
        struct { void *bytes[4];size_t sizes[4]; } waiting;
        OriginalAmbientReady ready;
    } data;
    unsigned frames,resets,rejected,captured;
} original_ambient;

static void report_original_ambient(void) {
    char path[MAX_PATH+64];snprintf(path,sizeof(path),"%s.original-ambient-live.json",control);
    FILE *file=fopen(path,"w");if (!file) return;
    fprintf(file,"{\"phase\":%u,\"frames\":%u,\"resets\":%u,\"rejected\":%u,\"present\":%u,\"scope\":\"Original temporal SSAO and skylight on live native depth/normal/stencil and camera constants; private output. Native probes and composition unchanged.\"}\n",
            original_ambient.phase,original_ambient.frames,original_ambient.resets,original_ambient.rejected,present_calls);fclose(file);
}
static void fail_original_ambient(const char *reason) {
    if (original_ambient.phase==ORIGINAL_AMBIENT_WAITING)
        for (unsigned i=0;i<4;++i) free(original_ambient.data.waiting.bytes[i]);
    if (original_ambient.phase==ORIGINAL_AMBIENT_READY)
        for (unsigned i=0;i<4;++i) post_replay_release(original_ambient.data.ready.passes[i]);
    original_ambient.phase=ORIGINAL_AMBIENT_FAILED;++original_ambient.rejected;
    error(reason);report_original_ambient();
}
static int configure_original_ambient(void) {
    char directory[MAX_PATH],path[MAX_PATH+32];
    DWORD length=GetEnvironmentVariableA("SRP_DIAGNOSTIC_ORIGINAL_AMBIENT_DIR",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || strcmp(build,"026g")) return 0;
    const char *names[]={"accum.bin","mask.bin","temporal.bin","sky.bin"};
    const UINT sizes[]={11096,3016,5060,3908};
    const unsigned char digests[4][16]={
        {0x1b,0x00,0x3e,0x11,0x01,0x17,0x5f,0x53,0xcb,0x5b,0x17,0x66,0xe4,0x85,0xcf,0x27},
        {0x64,0x77,0x0e,0xf8,0xc4,0xee,0xf2,0x4b,0x7f,0x98,0xc6,0xd5,0x14,0x66,0x5c,0x03},
        {0x13,0x5b,0xd2,0x9a,0xc4,0xfb,0xd3,0x99,0xa7,0xd7,0xab,0x06,0x4e,0xa1,0xfb,0x96},
        {0x21,0xc2,0x8f,0x59,0xef,0x73,0x8c,0x2d,0xb3,0x10,0x65,0x65,0x0f,0xf8,0x24,0x9f}
    };
    original_ambient.phase=ORIGINAL_AMBIENT_WAITING;
    for (unsigned i=0;i<4;++i) {
        int n=snprintf(path,sizeof(path),"%s/%s",directory,names[i]);
        if (n<0 || n>=(int)sizeof(path)) { fail_original_ambient("original ambient packet path too long");return 0; }
        original_ambient.data.waiting.bytes[i]=load_original_post_packet(path,sizes[i],digests[i],&original_ambient.data.waiting.sizes[i]);
        if (!original_ambient.data.waiting.bytes[i]) { fail_original_ambient("original ambient packet identity failed");return 0; }
    }
    return 1;
}
static int ambient_same_texture(ID3D11Texture2D *a,ID3D11Texture2D *b) {
    if (!a || !b) return 0;
    D3D11_TEXTURE2D_DESC x,y;ID3D11Texture2D_GetDesc(a,&x);ID3D11Texture2D_GetDesc(b,&y);
    return x.Width==y.Width && x.Height==y.Height && x.Format==y.Format && x.MipLevels==y.MipLevels &&
        x.ArraySize==y.ArraySize && x.SampleDesc.Count==1 && y.SampleDesc.Count==1;
}
static int start_original_ambient(ID3D11DeviceContext *context) {
    ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
    PostReplay *passes[4]={0};int ok=1;
    for (unsigned i=0;i<4 && ok;++i) {
        passes[i]=post_replay_create_contract(device,original_ambient.data.waiting.bytes[i],original_ambient.data.waiting.sizes[i],
                                             draw_indexed,original_draw_indexed,i==3?POST_NATIVE_SKYLIGHT_NORMAL:POST_CAPTURED_INPUTS);
        ok=passes[i]!=NULL;
    }
    ID3D11Device_Release(device);
    if (ok) {
        ok=ambient_same_texture(passes[0]->output,(ID3D11Texture2D *)passes[2]->inputs[1]) &&
            ambient_same_texture(passes[1]->output,(ID3D11Texture2D *)passes[2]->inputs[3]) &&
            ambient_same_texture(passes[2]->output,(ID3D11Texture2D *)passes[3]->inputs[2]) &&
            ambient_same_texture(passes[2]->output,(ID3D11Texture2D *)passes[2]->inputs[2]) &&
            ambient_same_texture(passes[2]->secondary_output,(ID3D11Texture2D *)passes[1]->inputs[1]) &&
            ambient_same_texture(passes[2]->depth_input,passes[3]->depth_input);
        for (unsigned i=0;i<4 && ok;++i) ok=passes[i]->pixel_sizes[1]==304 &&
            ambient_same_texture((ID3D11Texture2D *)passes[i]->inputs[0],(ID3D11Texture2D *)passes[3]->inputs[0]);
        ok=ok && passes[0]->eye_vertices && passes[1]->pixel_sizes[0]==96 && passes[2]->pixel_sizes[0]==96 && passes[3]->pixel_sizes[0]==160;
    }
    if (!ok) {
        for (unsigned i=0;i<4;++i) post_replay_release(passes[i]);
        fail_original_ambient("original ambient graph contract failed");return 0;
    }
    for (unsigned i=0;i<4;++i) free(original_ambient.data.waiting.bytes[i]);
    original_ambient.phase=ORIGINAL_AMBIENT_READY;
    original_ambient.data.ready=(OriginalAmbientReady){0};
    for (unsigned i=0;i<4;++i) original_ambient.data.ready.passes[i]=passes[i];
    return 1;
}
static int ambient_read_constants(ID3D11DeviceContext *context,UINT slot,void *output,UINT size) {
    ID3D11Buffer *source=NULL,*staging=NULL;ID3D11DeviceContext_PSGetConstantBuffers(context,slot,1,&source);
    if (!source) return 0;
    D3D11_BUFFER_DESC desc;ID3D11Buffer_GetDesc(source,&desc);int ok=desc.ByteWidth==size;
    if (ok) {
        desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;desc.MiscFlags=0;
        ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
        ok=SUCCEEDED(ID3D11Device_CreateBuffer(device,&desc,NULL,&staging));ID3D11Device_Release(device);
    }
    if (ok) {
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)source);
        D3D11_MAPPED_SUBRESOURCE mapped;ok=SUCCEEDED(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped));
        if (ok) { memcpy(output,mapped.pData,size);ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0); }
    }
    if (staging) ID3D11Buffer_Release(staging);
    ID3D11Buffer_Release(source);return ok;
}
static void capture_original_ambient(ID3D11DeviceContext *context,OriginalAmbientReady *ready,const Camera *camera,const unsigned char globals[304],int reset) {
    if (!camera->enabled) return;
    if (!reset && (capture_frame_id!=camera->id || captured_id==camera->id || original_ambient.captured==camera->id || GetTickCount()-applied_at<camera->settle_ms)) return;
    (void)context;
    char base[MAX_PATH+64],path[MAX_PATH+96];if (reset) snprintf(base,sizeof(base),"%s.original-ambient-reset-%u",camera->output,present_calls);
    else snprintf(base,sizeof(base),"%s.original-ambient",camera->output);
    snprintf(path,sizeof(path),"%s.targets.json",base);FILE *file=fopen(path,"w");
    PostReplay **p=ready->passes;
    ID3D11Texture2D *textures[]={p[0]->output,p[1]->output,p[2]->output,p[2]->secondary_output,p[3]->output,
                               (ID3D11Texture2D *)p[2]->inputs[2],(ID3D11Texture2D *)p[1]->inputs[1],
                               (ID3D11Texture2D *)p[0]->inputs[0],(ID3D11Texture2D *)p[3]->inputs[1],p[3]->depth_input};
    const char *names[]={"accumulation","history_mask","temporal_ao","temporal_depth","skylight","previous_ao","previous_depth","current_depth","current_normal","current_depth_stencil"};
    if (file) {
        fputs("[",file);
        for (unsigned i=0;i<10;++i) { if (i) fputs(",",file);dump_texture(textures[i],base,file,names[i],i,NULL); }
        fputs("]\n",file);fclose(file);
    }
    snprintf(path,sizeof(path),"%s.vertices.bin",base);write_bytes(path,p[0]->vertex_data,p[0]->vertex_size);
    snprintf(path,sizeof(path),"%s.globals.bin",base);write_bytes(path,globals,304);
    for (unsigned i=1;i<4;++i) {
        snprintf(path,sizeof(path),"%s.pass%u.cb0.bin",base,i);write_bytes(path,p[i]->pixel_data[0],p[i]->pixel_sizes[0]);
    }
    snprintf(path,sizeof(path),"%s.json",base);file=fopen(path,"w");
    if (file) {
        fprintf(file,"{\"present\":%u,\"camera\":%u,\"history_reset\":%s,\"frames\":%u,\"resets\":%u,\"scope\":\"Original shaders on live native inputs; private outputs, native lighting unchanged\"}\n",
                present_calls,camera->id,reset?"true":"false",original_ambient.frames,original_ambient.resets);fclose(file);
    }
    if (!reset) original_ambient.captured=camera->id;
    report_original_ambient();
}
static void draw_original_ambient(ID3D11DeviceContext *context) {
    if (original_ambient.phase==ORIGINAL_AMBIENT_OFF || original_ambient.phase==ORIGINAL_AMBIENT_FAILED) return;
    Camera camera;EnterCriticalSection(&lock);camera=applied;LeaveCriticalSection(&lock);
    if (!camera.enabled) return;
    if (original_ambient.phase==ORIGINAL_AMBIENT_READY && original_ambient.data.ready.history.phase==AMBIENT_HISTORY_VALID &&
        original_ambient.data.ready.history.data.valid.present==present_calls) return;
    ID3D11PixelShader *shader=NULL;ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);if (!shader) return;
    UINT size=0;HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,NULL);
    unsigned char code[6116];int known=SUCCEEDED(hr) && (size==6104 || size==6116);
    if (known) known=SUCCEEDED(ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,code));
    ID3D11PixelShader_Release(shader);if (!known) return;
    const unsigned char sphere[]={0xa7,0x0e,0x27,0x2e,0x38,0xe4,0x26,0x66,0xe1,0x39,0xc6,0xc9,0xdb,0x33,0x51,0x66};
    const unsigned char box[]={0x4f,0x5e,0x7d,0xc1,0xcf,0x82,0xff,0xa2,0x51,0x2c,0x12,0x5a,0x42,0xfe,0x1b,0xe5};
    if (!((size==6104 && !memcmp(code+4,sphere,16)) || (size==6116 && !memcmp(code+4,box,16)))) return;
    if (original_ambient.phase==ORIGINAL_AMBIENT_WAITING && !start_original_ambient(context)) return;
    OriginalAmbientReady *ready=&original_ambient.data.ready;PostReplay **p=ready->passes;
    ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
    int ok=device==p[0]->device;ID3D11Device_Release(device);
    ID3D11ShaderResourceView *views[3]={0};ID3D11DeviceContext_PSGetShaderResources(context,0,3,views);
    for (unsigned i=0;i<3;i+=2) {
        D3D11_SHADER_RESOURCE_VIEW_DESC desc={0};
        if (views[i]) ID3D11ShaderResourceView_GetDesc(views[i],&desc);
        ok=ok && desc.Format==(i?DXGI_FORMAT_R16_FLOAT:DXGI_FORMAT_R16G16_FLOAT);
    }
    ID3D11Resource *normal=original_post_source(views[0],p[3]->inputs[1]);
    ID3D11Resource *position=original_post_source(views[2],p[3]->inputs[0]);
    ID3D11DepthStencilView *depth=NULL;ID3D11DeviceContext_OMGetRenderTargets(context,0,NULL,&depth);
    ID3D11Resource *depth_resource=NULL;ID3D11Texture2D *depth_texture=NULL;
    if (depth) {
        D3D11_DEPTH_STENCIL_VIEW_DESC d;ID3D11DepthStencilView_GetDesc(depth,&d);
        ok=ok && d.Format==DXGI_FORMAT_D24_UNORM_S8_UINT && d.ViewDimension==D3D11_DSV_DIMENSION_TEXTURE2D && !d.Texture2D.MipSlice;
        ID3D11DepthStencilView_GetResource(depth,&depth_resource);
        ok=ok && SUCCEEDED(ID3D11Resource_QueryInterface(depth_resource,&IID_ID3D11Texture2D,(void **)&depth_texture));
    } else ok=0;
    unsigned char native_globals[416],native_parameters[208],globals[304];
    ok=ok && normal && position && ambient_same_texture(depth_texture,p[2]->depth_input) &&
        ambient_read_constants(context,0,native_parameters,sizeof(native_parameters)) && ambient_read_constants(context,1,native_globals,sizeof(native_globals));
    if (ok) {
        memcpy(globals,p[3]->pixel_data[1],304);memcpy(globals,native_globals,224);memcpy(globals+288,native_globals+224,16);
        float screen[4];memcpy(screen,globals+288,16);
        ok=screen[0]==p[3]->width && screen[1]==p[3]->height;
        for (unsigned i=0;i<56;++i) { float value;memcpy(&value,globals+i*4,4);ok=ok && isfinite(value); }
        for (unsigned i=0;i<3;++i) { float value;memcpy(&value,native_parameters+112+i*4,4);ok=ok && isfinite(value); }
    }
    if (ok) {
        int reset=ready->history.phase==AMBIENT_HISTORY_EMPTY || ready->history.data.valid.present+1!=present_calls || ready->history.data.valid.camera!=camera.id;
        float previous[16]={0};
        if (reset) { memcpy(previous,native_globals+128,48);previous[15]=1;++original_ambient.resets; }
        else memcpy(previous,ready->history.data.valid.view,64);
        const float zero[4]={0},white[4]={1,1,1,1};
        if (reset) {
            if (!zero_original_post_texture(context,p[1]->inputs[1]) || !zero_original_post_texture(context,p[2]->inputs[2])) {
                ok=0;goto release_inputs;
            }
            ID3D11DeviceContext_ClearRenderTargetView(context,p[2]->secondary_view,zero);
        }
        for (unsigned i=0;i<4;++i) {
            ID3D11DeviceContext_CopyResource(context,p[i]->inputs[0],position);
            ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)p[i]->pixel_constants[1],0,NULL,globals,0,0);
        }
        ID3D11DeviceContext_CopyResource(context,p[3]->inputs[1],normal);
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)p[2]->depth_input,depth_resource);
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)p[3]->depth_input,depth_resource);
        // post_process0.vs passes these view-space eye rays to accumulation.
        // Preserve its original screen quad and refresh all four corners.
        float eye[3];memcpy(eye,native_parameters+112,12);
        for (unsigned corner=0;corner<4;++corner) {
            float ray[3]={corner<2?eye[0]:-eye[0],corner%2?eye[1]:-eye[1],eye[2]};
            memcpy(p[0]->eye_vertices+corner*36+16,ray,sizeof(ray));
        }
        ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)p[0]->vertices,0,NULL,p[0]->vertex_data,0,0);
        memcpy(p[1]->pixel_data[0]+16,native_parameters+112,12);memcpy(p[1]->pixel_data[0]+32,previous,64);
        memcpy(p[2]->pixel_data[0]+20,native_parameters+112,12);memcpy(p[2]->pixel_data[0]+32,previous,64);
        memcpy(p[3]->pixel_data[0]+16,native_parameters+112,12);
        for (unsigned i=1;i<4;++i) ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)p[i]->pixel_constants[0],0,NULL,p[i]->pixel_data[0],0,0);
        ID3D11DeviceContext_ExecuteCommandList(context,p[0]->commands,TRUE);
        ID3D11DeviceContext_ExecuteCommandList(context,p[1]->commands,TRUE);
        // Original temporal bytecode takes current AO only when every mask tap
        // is zero. A discontinuity cannot sample history from the source packet.
        if (reset) ID3D11DeviceContext_ClearRenderTargetView(context,p[1]->target_view,zero);
        ID3D11DeviceContext_CopyResource(context,p[2]->inputs[1],(ID3D11Resource *)p[0]->output);
        ID3D11DeviceContext_CopyResource(context,p[2]->inputs[3],(ID3D11Resource *)p[1]->output);
        ID3D11DeviceContext_ClearRenderTargetView(context,p[2]->target_view,white);
        ID3D11DeviceContext_ExecuteCommandList(context,p[2]->commands,TRUE);
        ID3D11DeviceContext_CopyResource(context,p[3]->inputs[2],(ID3D11Resource *)p[2]->output);
        ID3D11DeviceContext_ClearRenderTargetView(context,p[3]->target_view,zero);
        ID3D11DeviceContext_ExecuteCommandList(context,p[3]->commands,TRUE);
        ++original_ambient.frames;capture_original_ambient(context,ready,&camera,globals,reset);
        ID3D11DeviceContext_CopyResource(context,p[2]->inputs[2],(ID3D11Resource *)p[2]->output);
        ID3D11DeviceContext_CopyResource(context,p[1]->inputs[1],(ID3D11Resource *)p[2]->secondary_output);
        ready->history.phase=AMBIENT_HISTORY_VALID;ready->history.data.valid.present=present_calls;ready->history.data.valid.camera=camera.id;
        memcpy(ready->history.data.valid.view,native_globals+128,48);
        memset(ready->history.data.valid.view+12,0,16);ready->history.data.valid.view[15]=1;
        if (original_ambient.frames==1 || original_ambient.frames%60==0) report_original_ambient();
    }
release_inputs:
    if (normal) ID3D11Resource_Release(normal);
    if (position) ID3D11Resource_Release(position);
    if (depth_texture) ID3D11Texture2D_Release(depth_texture);
    if (depth_resource) ID3D11Resource_Release(depth_resource);
    if (depth) ID3D11DepthStencilView_Release(depth);
    for (unsigned i=0;i<3;++i) if (views[i]) ID3D11ShaderResourceView_Release(views[i]);
    if (!ok) fail_original_ambient("native input contract failed for original ambient diagnostic");
}
