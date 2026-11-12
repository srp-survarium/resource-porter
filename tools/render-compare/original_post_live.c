// Opt-in, fixed-resolution diagnostic bridge for the unchanged 011e post.
// Not a production renderer: scene constants come from a validated packet,
// lens flares are absent, and the native downstream presentation still runs.
#include "post_replay.h"

typedef struct {
    PostReplay *pass;
    ID3D11ShaderResourceView *position;
    unsigned position_present;
} OriginalPostReady;
static struct {
    enum { ORIGINAL_POST_OFF, ORIGINAL_POST_WAITING, ORIGINAL_POST_READY, ORIGINAL_POST_FAILED } phase;
    union {
        struct { void *bytes;size_t size; } waiting;
        OriginalPostReady ready;
    } state;
    unsigned draws,rejected;
} original_post;

static int configure_original_post(void) {
    char path[MAX_PATH];DWORD length=GetEnvironmentVariableA("SRP_DIAGNOSTIC_ORIGINAL_POST_PACKET",path,sizeof(path));
    if (!length) return 1;
    if (length>=sizeof(path) || strcmp(build,"026g")) return 0;
    FILE *file=fopen(path,"rb");if (!file) return 0;
    int ok=!fseek(file,0,SEEK_END);long size=ftell(file);rewind(file);
    if (!ok || size<72 || size>128*1024*1024) { fclose(file);return 0; }
    void *bytes=malloc((size_t)size);
    ok=bytes && fread(bytes,1,(size_t)size,file)==(size_t)size;fclose(file);
    // Quality-3 grain+bokeh program observed in both original weather scenes.
    const unsigned char digest[]={0x2d,0xc8,0xb9,0xac,0xba,0xfe,0x20,0xb8,0x39,0xdf,0x4c,0xa8,0x0b,0xc8,0xac,0xd1};
    UINT code_size=0;if (ok) memcpy(&code_size,(char *)bytes+68,4);
    if (!ok || size<72+6744 || code_size!=6744 || memcmp((char *)bytes+72,"DXBC",4) || memcmp((char *)bytes+76,digest,16)) {
        free(bytes);return 0;
    }
    original_post.phase=ORIGINAL_POST_WAITING;
    original_post.state.waiting.bytes=bytes;original_post.state.waiting.size=(size_t)size;
    return 1;
}
static void report_original_post(void) {
    char path[MAX_PATH+48];snprintf(path,sizeof(path),"%s.original-post-live.json",control);
    FILE *file=fopen(path,"w");if (!file) return;
    fprintf(file,"{\"phase\":%u,\"draws\":%u,\"rejected\":%u,\"present\":%u,\"scope\":\"Diagnostic original q3 post: live frame/depth/luminance/grading, converted live scattering, original grain/bokeh assets and authored constants. Lens flares absent; native downstream presentation retained. Not full renderer parity.\"}\n",
            original_post.phase,original_post.draws,original_post.rejected,present_calls);fclose(file);
}
static void fail_original_post(const char *reason) {
    if (original_post.phase==ORIGINAL_POST_WAITING) free(original_post.state.waiting.bytes);
    if (original_post.phase==ORIGINAL_POST_READY) {
        if (original_post.state.ready.position) ID3D11ShaderResourceView_Release(original_post.state.ready.position);
        post_replay_release(original_post.state.ready.pass);
    }
    original_post.phase=ORIGINAL_POST_FAILED;error(reason);report_original_post();
}
static int zero_original_post_texture(ID3D11DeviceContext *context,ID3D11Resource *resource) {
    ID3D11Texture2D *texture=NULL;
    if (!resource || FAILED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) return 0;
    D3D11_TEXTURE2D_DESC desc;ID3D11Texture2D_GetDesc(texture,&desc);ID3D11Texture2D_Release(texture);
    UINT block,bytes;if (!post_texel_layout(desc.Format,&block,&bytes) || block!=1 || desc.MipLevels!=1 || desc.ArraySize!=1) return 0;
    size_t length=(size_t)desc.Width*desc.Height*bytes;void *zero=calloc(1,length);if (!zero) return 0;
    ID3D11DeviceContext_UpdateSubresource(context,resource,0,NULL,zero,desc.Width*bytes,0);free(zero);return 1;
}
static int start_original_post(ID3D11DeviceContext *context) {
    ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
    PostReplay *pass=post_replay_create(device,original_post.state.waiting.bytes,original_post.state.waiting.size,draw_indexed,original_draw_indexed);
    ID3D11Device_Release(device);
    if (!pass) { fail_original_post("original post packet could not create GPU resources");return 0; }
    free(original_post.state.waiting.bytes);
    original_post.phase=ORIGINAL_POST_READY;
    original_post.state.ready=(OriginalPostReady){pass,NULL,0};
    if (!pass->pixel_data[0] || pass->pixel_sizes[0]<304 || !zero_original_post_texture(context,pass->inputs[6])) {
        fail_original_post("original post packet lacks required constants or flare target");return 0;
    }
    report_original_post();return 1;
}
// Keep source resources until all contracts have passed, before copying any.
static ID3D11Resource *original_post_source(ID3D11ShaderResourceView *view,ID3D11Resource *destination) {
    if (!view || !destination) return NULL;
    D3D11_SHADER_RESOURCE_VIEW_DESC vd;ID3D11ShaderResourceView_GetDesc(view,&vd);
    ID3D11Resource *source=NULL;ID3D11ShaderResourceView_GetResource(view,&source);
    D3D11_RESOURCE_DIMENSION a,b;ID3D11Resource_GetType(source,&a);ID3D11Resource_GetType(destination,&b);
    int same=0;
    if (a==b && a==D3D11_RESOURCE_DIMENSION_TEXTURE2D && vd.ViewDimension==D3D11_SRV_DIMENSION_TEXTURE2D) {
        D3D11_TEXTURE2D_DESC x,y;ID3D11Texture2D_GetDesc((ID3D11Texture2D *)source,&x);ID3D11Texture2D_GetDesc((ID3D11Texture2D *)destination,&y);
        same=x.Width==y.Width && x.Height==y.Height && x.Format==y.Format && x.ArraySize==y.ArraySize && x.MipLevels==y.MipLevels &&
            x.SampleDesc.Count==1 && y.SampleDesc.Count==1 && vd.Texture2D.MostDetailedMip==0 &&
            (vd.Texture2D.MipLevels==x.MipLevels || vd.Texture2D.MipLevels==0xffffffff);
    } else if (a==b && a==D3D11_RESOURCE_DIMENSION_TEXTURE3D && vd.ViewDimension==D3D11_SRV_DIMENSION_TEXTURE3D) {
        D3D11_TEXTURE3D_DESC x,y;ID3D11Texture3D_GetDesc((ID3D11Texture3D *)source,&x);ID3D11Texture3D_GetDesc((ID3D11Texture3D *)destination,&y);
        same=x.Width==y.Width && x.Height==y.Height && x.Depth==y.Depth && x.Format==y.Format && x.MipLevels==y.MipLevels &&
            vd.Texture3D.MostDetailedMip==0 && (vd.Texture3D.MipLevels==x.MipLevels || vd.Texture3D.MipLevels==0xffffffff);
    }
    if (!same) { ID3D11Resource_Release(source);return NULL; }
    return source;
}
static float original_post_unsigned_float(unsigned code,unsigned mantissa) {
    unsigned exponent=code>>mantissa,fraction=code&((1u<<mantissa)-1);
    if (exponent==31) return NAN;
    return exponent?ldexpf(1.0f+(float)fraction/(1u<<mantissa),(int)exponent-15):ldexpf((float)fraction,-14-(int)mantissa);
}
// Diagnostic CPU transfer. The production adapter must perform this conversion
// on the GPU. The original shader decodes PACK_LIGHTING_COLOR as 8*E*E.
static int original_post_scattering(ID3D11DeviceContext *context,ID3D11ShaderResourceView *view,ID3D11Resource *destination) {
    if (!view || !destination) return 0;
    ID3D11Resource *resource=NULL;ID3D11ShaderResourceView_GetResource(view,&resource);
    ID3D11Texture2D *source=NULL,*target=NULL,*staging=NULL;
    int ok=SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&source)) &&
        SUCCEEDED(ID3D11Resource_QueryInterface(destination,&IID_ID3D11Texture2D,(void **)&target));
    ID3D11Resource_Release(resource);if (!ok) goto done;
    D3D11_TEXTURE2D_DESC a,b;ID3D11Texture2D_GetDesc(source,&a);ID3D11Texture2D_GetDesc(target,&b);
    ok=a.Width==b.Width && a.Height==b.Height && a.Format==DXGI_FORMAT_R11G11B10_FLOAT && b.Format==DXGI_FORMAT_R10G10B10A2_UNORM &&
        a.MipLevels==1 && a.ArraySize==1 && a.SampleDesc.Count==1 && b.MipLevels==1 && b.ArraySize==1;
    if (!ok) goto done;
    a.Usage=D3D11_USAGE_STAGING;a.BindFlags=0;a.CPUAccessFlags=D3D11_CPU_ACCESS_READ;a.MiscFlags=0;
    ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
    ok=SUCCEEDED(ID3D11Device_CreateTexture2D(device,&a,NULL,&staging));ID3D11Device_Release(device);if (!ok) goto done;
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)source);
    D3D11_MAPPED_SUBRESOURCE mapped;ok=SUCCEEDED(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped));if (!ok) goto done;
    uint32_t *packed=malloc((size_t)a.Width*a.Height*4);ok=packed!=NULL;
    for (unsigned y=0;ok && y<a.Height;++y) for (unsigned x=0;x<a.Width;++x) {
        uint32_t bits;memcpy(&bits,(char *)mapped.pData+y*mapped.RowPitch+x*4,4);
        float values[]={original_post_unsigned_float(bits&2047,6),original_post_unsigned_float((bits>>11)&2047,6),original_post_unsigned_float(bits>>22,5)};
        uint32_t result=0;
        for (unsigned c=0;c<3;++c) {
            if (!isfinite(values[c])) { ok=0;break; }
            result|=(uint32_t)lroundf(1023*sqrtf(fminf(values[c]*0.125f,1)))<<(c*10);
        }
        packed[y*a.Width+x]=result;
    }
    ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
    if (ok) ID3D11DeviceContext_UpdateSubresource(context,destination,0,NULL,packed,a.Width*4,0);
    free(packed);
done:
    if (staging) ID3D11Texture2D_Release(staging);
    if (source) ID3D11Texture2D_Release(source);
    if (target) ID3D11Texture2D_Release(target);
    return ok;
}
static int original_post_grain(ID3D11DeviceContext *context,PostReplay *pass) {
    ID3D11Buffer *native=NULL,*staging=NULL;ID3D11DeviceContext_PSGetConstantBuffers(context,0,1,&native);
    if (!native) return 0;
    D3D11_BUFFER_DESC desc;ID3D11Buffer_GetDesc(native,&desc);int ok=desc.ByteWidth>=184;
    if (ok) {
        desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;desc.MiscFlags=0;
        ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
        ok=SUCCEEDED(ID3D11Device_CreateBuffer(device,&desc,NULL,&staging));ID3D11Device_Release(device);
    }
    if (ok) {
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)native);
        D3D11_MAPPED_SUBRESOURCE mapped;ok=SUCCEEDED(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped));
        if (ok) {
            float phase[2];memcpy(phase,(char *)mapped.pData+176,8);ok=isfinite(phase[0]) && isfinite(phase[1]);
            if (ok) memcpy(pass->pixel_data[0]+256,phase,8);
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
            if (ok) ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)pass->pixel_constants[0],0,NULL,pass->pixel_data[0],0,0);
        }
    }
    if (staging) ID3D11Buffer_Release(staging);
    ID3D11Buffer_Release(native);return ok;
}
static int draw_original_post(ID3D11DeviceContext *context) {
    if (original_post.phase==ORIGINAL_POST_OFF || original_post.phase==ORIGINAL_POST_FAILED) return 0;
    EnterCriticalSection(&lock);int enabled=applied.enabled;LeaveCriticalSection(&lock);
    if (!enabled) return 0;
    ID3D11PixelShader *shader=NULL;ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);if (!shader) return 0;
    UINT size=0;HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,NULL);
    unsigned char code[8416];int known=SUCCEEDED(hr) && (size==3972 || size==8416);
    if (known) known=SUCCEEDED(ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,code));
    ID3D11PixelShader_Release(shader);if (!known) return 0;
    const unsigned char composition[]={0xde,0x3e,0x7a,0x63,0x18,0x34,0xe1,0x5d,0xb4,0xae,0x34,0x73,0x59,0x86,0x7c,0x30};
    const unsigned char post[]={0x22,0xa1,0xd1,0x39,0x98,0x9d,0x32,0x3c,0x71,0xb8,0x68,0x2b,0x30,0xbe,0xeb,0x48};
    int is_composition=size==8416 && !memcmp(code+4,composition,16);
    int is_post=size==3972 && !memcmp(code+4,post,16);
    if (!is_composition && !is_post) return 0;
    if (original_post.phase==ORIGINAL_POST_WAITING && !start_original_post(context)) return 0;
    OriginalPostReady *ready=&original_post.state.ready;PostReplay *pass=ready->pass;
    ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
    int same_device=device==pass->device;ID3D11Device_Release(device);
    if (!same_device) { fail_original_post("original post diagnostic device changed");return 0; }
    if (is_composition) {
        if (ready->position) ID3D11ShaderResourceView_Release(ready->position);
        ready->position=NULL;ID3D11DeviceContext_PSGetShaderResources(context,3,1,&ready->position);
        ready->position_present=present_calls;return 0;
    }
    ID3D11ShaderResourceView *native[4]={0};ID3D11DeviceContext_PSGetShaderResources(context,0,4,native);
    ID3D11Resource *sources[4]={0};unsigned slots[]={0,2,3,7};
    if (ready->position_present==present_calls) sources[0]=original_post_source(ready->position,pass->inputs[0]);
    sources[1]=original_post_source(native[0],pass->inputs[2]);sources[2]=original_post_source(native[2],pass->inputs[3]);sources[3]=original_post_source(native[1],pass->inputs[7]);
    ID3D11RenderTargetView *targets[8]={0};ID3D11DepthStencilView *depth=NULL;ID3D11DeviceContext_OMGetRenderTargets(context,8,targets,&depth);
    ID3D11Resource *output=NULL;if (targets[0]) ID3D11RenderTargetView_GetResource(targets[0],&output);
    ID3D11Texture2D *output_texture=NULL;int ok=output && SUCCEEDED(ID3D11Resource_QueryInterface(output,&IID_ID3D11Texture2D,(void **)&output_texture));
    if (ok) {
        D3D11_TEXTURE2D_DESC desc;ID3D11Texture2D_GetDesc(output_texture,&desc);
        ok=desc.Width==pass->width && desc.Height==pass->height && desc.Format==pass->format && desc.SampleDesc.Count==1 && desc.MipLevels==1 && desc.ArraySize==1;
        D3D11_VIEWPORT viewport={0};UINT count=1;ID3D11DeviceContext_RSGetViewports(context,&count,&viewport);
        ok=ok && count==1 && viewport.TopLeftX==0 && viewport.TopLeftY==0 && viewport.Width==pass->width && viewport.Height==pass->height && viewport.MinDepth==0 && viewport.MaxDepth==1;
        for (unsigned i=1;i<8;++i) ok=ok && !targets[i];
    }
    for (unsigned i=0;i<4;++i) ok=ok && sources[i]!=NULL;
    if (ok) ok=original_post_grain(context,pass) && original_post_scattering(context,native[3],pass->inputs[4]);
    if (ok) {
        for (unsigned i=0;i<4;++i) ID3D11DeviceContext_CopyResource(context,pass->inputs[slots[i]],sources[i]);
        ID3D11DeviceContext_ExecuteCommandList(context,pass->commands,TRUE);
        ID3D11DeviceContext_OMSetRenderTargets(context,0,NULL,NULL);
        ID3D11DeviceContext_CopyResource(context,output,(ID3D11Resource *)pass->output);
        ID3D11DeviceContext_OMSetRenderTargets(context,8,targets,depth);++original_post.draws;
    } else ++original_post.rejected;
    for (unsigned i=0;i<4;++i) { if (native[i]) ID3D11ShaderResourceView_Release(native[i]);if (sources[i]) ID3D11Resource_Release(sources[i]); }
    for (unsigned i=0;i<8;++i) if (targets[i]) ID3D11RenderTargetView_Release(targets[i]);
    if (depth) ID3D11DepthStencilView_Release(depth);
    if (output_texture) ID3D11Texture2D_Release(output_texture);
    if (output) ID3D11Resource_Release(output);
    if (original_post.draws==1 || (original_post.draws && original_post.draws%60==0) || original_post.rejected==1) report_original_post();
    return ok;
}
