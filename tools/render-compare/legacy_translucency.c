// Preserve the native cascade projection on the GPU for source translucency.
#include "sun_projection_index.h"
static void *legacy_sun_projection_code;
static size_t legacy_sun_projection_size;
static float legacy_translucency_color[4];
static ID3D11PixelShader *legacy_sun_projection_shader;
static ID3D11Texture2D *legacy_sun_projection_texture;
static ID3D11RenderTargetView *legacy_sun_projection_target;
static ID3D11ShaderResourceView *legacy_sun_projection_view;
static ID3D11Buffer *legacy_translucency_constants;
static unsigned legacy_sun_projection_frame, legacy_sun_projection_draws;
static const GUID sun_projection_guid={0xbde8cef1,0x3189,0x475a,{0x93,0x64,0xc2,0x27,0xf1,0x69,0x64,0x21}};
static D3D11_DEPTH_STENCIL_DESC projection_depth_desc;
static ID3D11DepthStencilState *projection_depth_state;

static void mark_sun_projection(ID3D11PixelShader *shader,const void *code,size_t size) {
    if (!legacy_sun_projection_code || !contains(code,size,"sun_direction_parameter") ||
        !contains(code,size,"shadow_filter_radius")) return;
    unsigned kind=2;
    for (unsigned i=0;i<sizeof(sun_projection_shaders)/sizeof(sun_projection_shaders[0]);++i)
        if (size==sun_projection_shaders[i].size && !memcmp((const char *)code+4,sun_projection_shaders[i].digest,16)) { kind=1; break; }
    if (kind==2) {
        char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.sun-unverified-%u.dxbc",control,(unsigned)size);
        FILE *file=fopen(path,"wb");
        if (file) { fwrite(code,1,size,file); fclose(file); }
    }
    ID3D11PixelShader_SetPrivateData(shader,&sun_projection_guid,sizeof(kind),&kind);
}

static int allocate_sun_projection(ID3D11DeviceContext *context,ID3D11RenderTargetView *target) {
    ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL;
    ID3D11RenderTargetView_GetResource(target,&resource);
    HRESULT hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource);
    if (FAILED(hr)) return 0;
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc); ID3D11Texture2D_Release(texture);
    if (desc.SampleDesc.Count!=1 || desc.ArraySize!=1 || desc.Format!=DXGI_FORMAT_R8G8_UNORM) return 0;
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    if (legacy_sun_projection_texture) {
        D3D11_TEXTURE2D_DESC existing; ID3D11Texture2D_GetDesc(legacy_sun_projection_texture,&existing);
        if (existing.Width!=desc.Width || existing.Height!=desc.Height) {
            ID3D11ShaderResourceView_Release(legacy_sun_projection_view);
            ID3D11RenderTargetView_Release(legacy_sun_projection_target);
            ID3D11Texture2D_Release(legacy_sun_projection_texture);
            legacy_sun_projection_view=NULL; legacy_sun_projection_target=NULL; legacy_sun_projection_texture=NULL;
            legacy_sun_projection_draws=0;
        }
    }
    if (!legacy_sun_projection_shader)
        hr=original_create_ps(device,legacy_sun_projection_code,legacy_sun_projection_size,NULL,&legacy_sun_projection_shader);
    if (SUCCEEDED(hr) && !legacy_translucency_constants) {
        D3D11_BUFFER_DESC buffer={0}; buffer.ByteWidth=sizeof(legacy_translucency_color);
        buffer.Usage=D3D11_USAGE_IMMUTABLE; buffer.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
        D3D11_SUBRESOURCE_DATA initial={legacy_translucency_color,0,0};
        hr=ID3D11Device_CreateBuffer(device,&buffer,&initial,&legacy_translucency_constants);
    }
    if (SUCCEEDED(hr) && !legacy_sun_projection_texture) {
        desc.Format=DXGI_FORMAT_R32_FLOAT; desc.MipLevels=1;
        desc.Usage=D3D11_USAGE_DEFAULT; desc.BindFlags=D3D11_BIND_RENDER_TARGET|D3D11_BIND_SHADER_RESOURCE;
        desc.CPUAccessFlags=desc.MiscFlags=0;
        hr=ID3D11Device_CreateTexture2D(device,&desc,NULL,&legacy_sun_projection_texture);
        if (SUCCEEDED(hr)) hr=ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)legacy_sun_projection_texture,NULL,&legacy_sun_projection_target);
        if (SUCCEEDED(hr)) hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)legacy_sun_projection_texture,NULL,&legacy_sun_projection_view);
    }
    if (FAILED(hr)) {
        if (legacy_sun_projection_view) ID3D11ShaderResourceView_Release(legacy_sun_projection_view);
        if (legacy_sun_projection_target) ID3D11RenderTargetView_Release(legacy_sun_projection_target);
        if (legacy_sun_projection_texture) ID3D11Texture2D_Release(legacy_sun_projection_texture);
        legacy_sun_projection_view=NULL; legacy_sun_projection_target=NULL; legacy_sun_projection_texture=NULL;
    }
    ID3D11Device_Release(device);
    return SUCCEEDED(hr) && legacy_sun_projection_target && legacy_sun_projection_view;
}

static void capture_sun_projection(ID3D11DeviceContext *context) {
    Camera camera;
    EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id;
    LeaveCriticalSection(&lock);
    if (!take) return;
    char base[MAX_PATH+64],path[MAX_PATH+80];
    snprintf(base,sizeof(base),"%s.sun-projection-%u",camera.output,legacy_sun_projection_draws);
    snprintf(path,sizeof(path),"%s.targets.json",base); FILE *file=fopen(path,"w");
    if (file) {
        fputs("[",file); dump_texture(legacy_sun_projection_texture,base,file,"projected_depth",0,NULL);
        fputs("]\n",file); fclose(file);
    }
    ID3D11Buffer *buffer=NULL,*staging=NULL; ID3D11DeviceContext_PSGetConstantBuffers(context,0,1,&buffer);
    if (!buffer) { error("sun projection constants absent"); return; }
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    D3D11_BUFFER_DESC desc; ID3D11Buffer_GetDesc(buffer,&desc);
    desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ; desc.MiscFlags=0;
    HRESULT hr=ID3D11Device_CreateBuffer(device,&desc,NULL,&staging);
    if (SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)buffer);
        D3D11_MAPPED_SUBRESOURCE mapped;
        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
        if (SUCCEEDED(hr)) {
            snprintf(path,sizeof(path),"%s.cb0.bin",base); file=fopen(path,"wb");
            if (!file || fwrite(mapped.pData,1,desc.ByteWidth,file)!=desc.ByteWidth) error("cannot save sun projection constants");
            if (file) fclose(file);
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        }
    }
    if (FAILED(hr)) error("cannot read sun projection constants");
    if (staging) ID3D11Buffer_Release(staging);
    ID3D11Buffer_Release(buffer); ID3D11Device_Release(device);
}

static void render_sun_projection(ID3D11DeviceContext *context,UINT count,UINT start,INT base,int indexed) {
    if (!legacy_sun_projection_code) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned kind=0; UINT size=sizeof(kind);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&sun_projection_guid,&size,&kind);
    if (FAILED(hr)) { ID3D11PixelShader_Release(shader); return; }
    if (kind!=1) { error("unverified sun-shadow permutation for source translucency"); ID3D11PixelShader_Release(shader); return; }
    ID3D11RenderTargetView *targets[8]={0}; ID3D11DepthStencilView *depth=NULL;
    ID3D11DepthStencilState *depth_state=NULL; UINT stencil;
    ID3D11DeviceContext_OMGetRenderTargets(context,8,targets,&depth);
    ID3D11DeviceContext_OMGetDepthStencilState(context,&depth_state,&stencil);
    if (!targets[0] || !depth_state || targets[1]) { error("unexpected sun-shadow targets/state"); goto done; }
    ID3D11BlendState *blend=NULL; float factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    if (blend) {
        D3D11_BLEND_DESC desc; ID3D11BlendState_GetDesc(blend,&desc); ID3D11BlendState_Release(blend);
        if (desc.RenderTarget[0].BlendEnable) { error("sun projection requires overwrite blending"); goto done; }
    }
    if (!allocate_sun_projection(context,targets[0])) { error("cannot allocate sun projection helper"); goto done; }
    D3D11_DEPTH_STENCIL_DESC desc={0}; ID3D11DepthStencilState_GetDesc(depth_state,&desc);
    // Replay the same tests before the native draw, but never mutate its depth/stencil.
    desc.DepthWriteMask=D3D11_DEPTH_WRITE_MASK_ZERO; desc.StencilWriteMask=0;
    if (!projection_depth_state || memcmp(&desc,&projection_depth_desc,sizeof(desc))) {
        ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
        if (projection_depth_state) ID3D11DepthStencilState_Release(projection_depth_state);
        projection_depth_state=NULL;
        hr=ID3D11Device_CreateDepthStencilState(device,&desc,&projection_depth_state);
        ID3D11Device_Release(device);
        if (FAILED(hr)) { error("cannot create nonwriting shadow stencil state"); goto done; }
        projection_depth_desc=desc;
    }
    ID3D11ShaderResourceView *previous=NULL; ID3D11DeviceContext_PSGetShaderResources(context,13,1,&previous);
    ID3D11ShaderResourceView *empty=NULL; ID3D11DeviceContext_PSSetShaderResources(context,13,1,&empty);
    if (!legacy_sun_projection_draws || legacy_sun_projection_frame!=present_calls) {
        const float clear[4]={0}; ID3D11DeviceContext_ClearRenderTargetView(context,legacy_sun_projection_target,clear);
        legacy_sun_projection_frame=present_calls; legacy_sun_projection_draws=0;
    }
    ID3D11DeviceContext_OMSetRenderTargets(context,1,&legacy_sun_projection_target,depth);
    ID3D11DeviceContext_OMSetDepthStencilState(context,projection_depth_state,stencil);
    ID3D11DeviceContext_PSSetShader(context,legacy_sun_projection_shader,NULL,0);
    if (indexed) original_draw_indexed(context,count,start,base); else original_draw(context,count,start);
    ++legacy_sun_projection_draws;
    capture_sun_projection(context);
    ID3D11DeviceContext_PSSetShader(context,shader,NULL,0);
    ID3D11DeviceContext_OMSetDepthStencilState(context,depth_state,stencil);
    ID3D11DeviceContext_OMSetRenderTargets(context,8,targets,depth);
    ID3D11DeviceContext_PSSetShaderResources(context,13,1,&previous);
    if (previous) ID3D11ShaderResourceView_Release(previous);
done:
    for (unsigned i=0;i<8;++i) if (targets[i]) ID3D11RenderTargetView_Release(targets[i]);
    if (depth) ID3D11DepthStencilView_Release(depth);
    if (depth_state) ID3D11DepthStencilState_Release(depth_state);
    ID3D11PixelShader_Release(shader);
}

typedef struct { ID3D11ShaderResourceView *view; ID3D11Buffer *constants; int bound; } TranslucencyState;
static void bind_translucency(ID3D11DeviceContext *context,TranslucencyState *state) {
    memset(state,0,sizeof(*state));
    if (!legacy_sun_projection_code) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&composition_guid,&size,&flag);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || !flag) return;
    if (!legacy_sun_projection_draws || legacy_sun_projection_frame!=present_calls) {
        error("source translucency requires this frame's sun projection"); return;
    }
    ID3D11DeviceContext_PSGetShaderResources(context,13,1,&state->view);
    ID3D11DeviceContext_PSGetConstantBuffers(context,2,1,&state->constants);
    ID3D11DeviceContext_PSSetShaderResources(context,13,1,&legacy_sun_projection_view);
    ID3D11DeviceContext_PSSetConstantBuffers(context,2,1,&legacy_translucency_constants);
    state->bound=1;
    if (applied.enabled && capture_frame_id==applied.id && draw_ids[7]!=applied.id) {
        char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.translucency-helper.json",applied.output);
        FILE *file=fopen(path,"w");
        if (file) {
            fprintf(file,"{\"present\":%u,\"helper_present\":%u,\"cascade_draws\":%u,\"sun_color\":[%.9g,%.9g,%.9g]}\n",present_calls,legacy_sun_projection_frame,legacy_sun_projection_draws,legacy_translucency_color[0],legacy_translucency_color[1],legacy_translucency_color[2]);
            fclose(file);
        }
    }
}
static void restore_translucency(ID3D11DeviceContext *context,TranslucencyState *state) {
    if (!state->bound) return;
    ID3D11DeviceContext_PSSetShaderResources(context,13,1,&state->view);
    ID3D11DeviceContext_PSSetConstantBuffers(context,2,1,&state->constants);
    if (state->view) ID3D11ShaderResourceView_Release(state->view);
    if (state->constants) ID3D11Buffer_Release(state->constants);
}
