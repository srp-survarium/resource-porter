// Opt-in visual experiment: original two-pass atmospheric scattering on native
// depth/camera inputs. Weather LUTs are frozen source captures, not production assets.
typedef struct {
    ID3D11Device *device;
    ID3D11PixelShader *shaders[2];
    ID3D11Buffer *constants[2];
    ID3D11ShaderResourceView *tables[2];
    ID3D11SamplerState *samplers[2];
    ID3D11BlendState *blends[2];
    float parameters[8];
} OriginalFogReady;
static struct {
    enum { ORIGINAL_FOG_OFF, ORIGINAL_FOG_WAITING, ORIGINAL_FOG_READY, ORIGINAL_FOG_FAILED } phase;
    union { char directory[MAX_PATH]; OriginalFogReady ready; } state;
    unsigned draws,rejected;
} original_fog;

static void report_original_fog(void) {
    char path[MAX_PATH+48];snprintf(path,sizeof(path),"%s.original-fog-live.json",control);
    FILE *file=fopen(path,"w");if (!file) return;
    fprintf(file,"{\"phase\":%u,\"draws\":%u,\"rejected\":%u,\"present\":%u,\"scope\":\"Original fog shaders and captured weather LUTs; live native depth/camera; native draw geometry and coverage. Diagnostic only.\"}\n",
            original_fog.phase,original_fog.draws,original_fog.rejected,present_calls);
    fclose(file);
}
static void release_original_fog(OriginalFogReady *ready) {
    for (unsigned i=0;i<2;++i) {
        if (ready->shaders[i]) ID3D11PixelShader_Release(ready->shaders[i]);
        if (ready->constants[i]) ID3D11Buffer_Release(ready->constants[i]);
        if (ready->tables[i]) ID3D11ShaderResourceView_Release(ready->tables[i]);
        if (ready->samplers[i]) ID3D11SamplerState_Release(ready->samplers[i]);
        if (ready->blends[i]) ID3D11BlendState_Release(ready->blends[i]);
    }
    if (ready->device) ID3D11Device_Release(ready->device);
}
static int configure_original_fog(void) {
    char directory[MAX_PATH];DWORD n=GetEnvironmentVariableA("SRP_DIAGNOSTIC_ORIGINAL_FOG_DIR",directory,sizeof(directory));
    if (!n) return 1;
    if (n>=sizeof(directory) || strcmp(build,"026g")) return 0;
    original_fog.phase=ORIGINAL_FOG_WAITING;
    memcpy(original_fog.state.directory,directory,n+1);return 1;
}
static void *read_original_fog_asset(const char *directory,const char *name,size_t bytes) {
    char path[MAX_PATH];int n=snprintf(path,sizeof(path),"%s/%s",directory,name);
    if (n<0 || n>=(int)sizeof(path)) return NULL;
    FILE *file=fopen(path,"rb");if (!file) return NULL;
    void *data=malloc(bytes);int ok=data && fread(data,1,bytes,file)==bytes && fgetc(file)==EOF;
    fclose(file);if (!ok) { free(data);return NULL; }return data;
}
static int start_original_fog(ID3D11DeviceContext *context) {
    OriginalFogReady ready={0};ID3D11DeviceContext_GetDevice(context,&ready.device);
    const char *directory=original_fog.state.directory;
    const char *shader_names[]={"fog-mul.dxbc","fog-add.dxbc"};
    const unsigned sizes[]={1580,3896};
    const unsigned char digests[2][16]={
        {0x34,0x66,0xf5,0x7b,0x33,0x8c,0xfa,0x52,0xf4,0xdb,0xd4,0xd6,0x87,0x25,0xf3,0x4e},
        {0x9e,0x56,0xf4,0xa4,0xdc,0xb4,0x5e,0x14,0x39,0x7f,0x98,0x3d,0x37,0xaa,0x4a,0xf2}};
    void *parameters=read_original_fog_asset(directory,"parameters.bin",sizeof(ready.parameters));
    int ok=parameters!=NULL;
    if (ok) memcpy(ready.parameters,parameters,sizeof(ready.parameters));
    free(parameters);
    for (unsigned i=0;i<8 && ok;++i) ok=isfinite(ready.parameters[i]);
    ok=ok && ready.parameters[4]>0 && ready.parameters[5]>0;
    for (unsigned i=0;i<2 && ok;++i) {
        void *code=read_original_fog_asset(directory,shader_names[i],sizes[i]);
        ok=code && !memcmp(code,"DXBC",4) && !memcmp((char *)code+4,digests[i],16);
        if (ok) ok=SUCCEEDED(original_create_ps(ready.device,code,sizes[i],NULL,&ready.shaders[i]));
        free(code);
        D3D11_BUFFER_DESC buffer={0};buffer.ByteWidth=i?304:128;buffer.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
        if (ok) ok=SUCCEEDED(ID3D11Device_CreateBuffer(ready.device,&buffer,NULL,&ready.constants[i]));
        void *pixels=read_original_fog_asset(directory,i?"mie.bin":"rayleigh.bin",256*128*8);
        if (!pixels) ok=0;
        D3D11_TEXTURE2D_DESC texture={0};texture.Width=256;texture.Height=128;texture.MipLevels=1;texture.ArraySize=1;
        texture.Format=DXGI_FORMAT_R16G16B16A16_FLOAT;texture.SampleDesc.Count=1;texture.BindFlags=D3D11_BIND_SHADER_RESOURCE;
        D3D11_SUBRESOURCE_DATA initial={pixels,256*8,0};ID3D11Texture2D *table=NULL;
        if (ok) ok=SUCCEEDED(ID3D11Device_CreateTexture2D(ready.device,&texture,&initial,&table));
        if (ok) ok=SUCCEEDED(ID3D11Device_CreateShaderResourceView(ready.device,(ID3D11Resource *)table,NULL,&ready.tables[i]));
        if (table) ID3D11Texture2D_Release(table);
        free(pixels);
        D3D11_SAMPLER_DESC sampler={0};sampler.Filter=i?D3D11_FILTER_MIN_MAG_MIP_LINEAR:D3D11_FILTER_MIN_MAG_MIP_POINT;
        sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.MaxAnisotropy=1;sampler.ComparisonFunc=D3D11_COMPARISON_NEVER;sampler.MaxLOD=D3D11_FLOAT32_MAX;
        if (ok) ok=SUCCEEDED(ID3D11Device_CreateSamplerState(ready.device,&sampler,&ready.samplers[i]));
        D3D11_BLEND_DESC blend={0};D3D11_RENDER_TARGET_BLEND_DESC *target=&blend.RenderTarget[0];
        target->BlendEnable=TRUE;target->SrcBlend=i?D3D11_BLEND_ONE:D3D11_BLEND_ZERO;
        target->DestBlend=i?D3D11_BLEND_ONE:D3D11_BLEND_SRC_COLOR;target->BlendOp=D3D11_BLEND_OP_ADD;
        target->SrcBlendAlpha=D3D11_BLEND_ONE;target->DestBlendAlpha=D3D11_BLEND_ZERO;target->BlendOpAlpha=D3D11_BLEND_OP_ADD;
        target->RenderTargetWriteMask=D3D11_COLOR_WRITE_ENABLE_ALL;
        if (ok) ok=SUCCEEDED(ID3D11Device_CreateBlendState(ready.device,&blend,&ready.blends[i]));
    }
    if (!ok) {
        release_original_fog(&ready);original_fog.phase=ORIGINAL_FOG_FAILED;
        error("original fog diagnostic initialization failed");report_original_fog();return 0;
    }
    original_fog.state.ready=ready;original_fog.phase=ORIGINAL_FOG_READY;return 1;
}
static int draw_original_fog(ID3D11DeviceContext *context,UINT count,UINT start,INT base,int indexed) {
    if (original_fog.phase==ORIGINAL_FOG_OFF || original_fog.phase==ORIGINAL_FOG_FAILED) return 0;
    EnterCriticalSection(&lock);int enabled=applied.enabled;LeaveCriticalSection(&lock);if (!enabled) return 0;
    ID3D11PixelShader *native_shader=NULL;ID3D11DeviceContext_PSGetShader(context,&native_shader,NULL,NULL);
    if (!native_shader) return 0;
    UINT size=0;unsigned char code[3608];
    int known=SUCCEEDED(ID3D11PixelShader_GetPrivateData(native_shader,&post_code_guid,&size,NULL)) && size==sizeof(code);
    if (known) known=SUCCEEDED(ID3D11PixelShader_GetPrivateData(native_shader,&post_code_guid,&size,code));
    const unsigned char digest[]={0x0f,0x99,0x55,0x0b,0xd7,0x4e,0x29,0x0a,0xcf,0x08,0xab,0x46,0xdd,0xc5,0xed,0x20};
    known=known && !memcmp(code+4,digest,16);
    if (!known) { ID3D11PixelShader_Release(native_shader);return 0; }
    if (original_fog.phase==ORIGINAL_FOG_WAITING && !start_original_fog(context)) { ID3D11PixelShader_Release(native_shader);return 0; }
    OriginalFogReady *ready=&original_fog.state.ready;
    ID3D11Device *device=NULL;ID3D11DeviceContext_GetDevice(context,&device);
    int ok=device==ready->device;ID3D11Device_Release(device);
    float parameters[32]={0},globals[76]={0},native_parameters[20],native_globals[104];
    D3D11_VIEWPORT viewport;UINT viewports=1;ID3D11DeviceContext_RSGetViewports(context,&viewports,&viewport);
    ok=ok && viewports==1 && viewport.TopLeftX==0 && viewport.TopLeftY==0 && viewport.Width>0 && viewport.Height>0 &&
        ambient_read_constants(context,0,native_parameters,sizeof(native_parameters)) && ambient_read_constants(context,1,native_globals,sizeof(native_globals));
    if (ok) {
        memcpy(parameters+20,ready->parameters,32);memcpy(parameters+28,native_parameters,12);
        memcpy(globals+44,native_globals+44,48);memcpy(globals+60,native_globals+80,12);
        globals[72]=viewport.Width;globals[73]=viewport.Height;globals[74]=1/viewport.Width;globals[75]=1/viewport.Height;
        for (unsigned i=0;i<32;++i) ok=ok && isfinite(parameters[i]);
        for (unsigned i=0;i<76;++i) ok=ok && isfinite(globals[i]);
    }
    ID3D11ShaderResourceView *native_views[3]={0};ID3D11DeviceContext_PSGetShaderResources(context,0,3,native_views);
    ok=ok && native_views[0]!=NULL;
    if (ok) {
        ID3D11Buffer *native_constants[2]={0};ID3D11SamplerState *native_samplers[2]={0};
        ID3D11BlendState *native_blend=NULL;FLOAT factors[4];UINT mask;
        ID3D11DeviceContext_PSGetConstantBuffers(context,0,2,native_constants);
        ID3D11DeviceContext_PSGetSamplers(context,0,2,native_samplers);
        ID3D11DeviceContext_OMGetBlendState(context,&native_blend,factors,&mask);
        ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)ready->constants[0],0,NULL,parameters,0,0);
        ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)ready->constants[1],0,NULL,globals,0,0);
        ID3D11DeviceContext_PSSetConstantBuffers(context,0,2,ready->constants);
        ID3D11ShaderResourceView *views[]={native_views[0],ready->tables[0],ready->tables[1]};
        ID3D11DeviceContext_PSSetShaderResources(context,0,3,views);
        ID3D11DeviceContext_PSSetSamplers(context,0,2,ready->samplers);
        for (unsigned i=0;i<2;++i) {
            ID3D11DeviceContext_PSSetShader(context,ready->shaders[i],NULL,0);
            ID3D11DeviceContext_OMSetBlendState(context,ready->blends[i],factors,mask);
            if (indexed) original_draw_indexed(context,count,start,base);else original_draw(context,count,start);
        }
        ID3D11DeviceContext_PSSetShader(context,native_shader,NULL,0);
        ID3D11DeviceContext_PSSetConstantBuffers(context,0,2,native_constants);
        ID3D11DeviceContext_PSSetShaderResources(context,0,3,native_views);
        ID3D11DeviceContext_PSSetSamplers(context,0,2,native_samplers);
        ID3D11DeviceContext_OMSetBlendState(context,native_blend,factors,mask);
        for (unsigned i=0;i<2;++i) {
            if (native_constants[i]) ID3D11Buffer_Release(native_constants[i]);
            if (native_samplers[i]) ID3D11SamplerState_Release(native_samplers[i]);
        }
        if (native_blend) ID3D11BlendState_Release(native_blend);
        ++original_fog.draws;
    } else ++original_fog.rejected;
    for (unsigned i=0;i<3;++i) if (native_views[i]) ID3D11ShaderResourceView_Release(native_views[i]);
    ID3D11PixelShader_Release(native_shader);
    if (original_fog.draws==1 || original_fog.draws%60==0 || original_fog.rejected==1) report_original_fog();
    return ok;
}
