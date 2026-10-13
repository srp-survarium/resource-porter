// Original 0.10b Factory rain, replayed into a private pre-rain color copy.
// Native rain/display remain active. Capture inputs and every shell separately.
#include "legacy_rain_math.h"
static const GUID source_rain_guid={0x5f27d621,0x58e2,0x4c62,{0xae,0x10,0xd6,0x30,0x88,0xab,0x44,0x39}};
static int source_rain_enabled,source_rain_gpu_ready;
static void *source_rain_assets[5];
static const unsigned source_rain_sizes[5]={1708,3620,2400,864,262144};
static ID3D11VertexShader *source_rain_vs;
static ID3D11PixelShader *source_rain_ps;
static ID3D11InputLayout *source_rain_layout;
static ID3D11Buffer *source_rain_vertices,*source_rain_indices,*source_rain_world,*source_rain_globals,*source_rain_parameters;
static ID3D11ShaderResourceView *source_rain_texture,*source_rain_position,*source_rain_frame_view;
static ID3D11Texture2D *source_rain_output,*source_rain_frame;
static ID3D11RenderTargetView *source_rain_target;
static ID3D11SamplerState *source_rain_samplers[5];
static ID3D11BlendState *source_rain_blend;
static ID3D11RasterizerState *source_rain_raster;
static ID3D11DepthStencilState *source_rain_depth;
static unsigned source_rain_position_frame,source_rain_motion_frame,source_rain_capture_id;
static LegacyRainMotion source_rain_motion;
static float source_rain_origin[3];

static int configure_source_rain(void) {
    char directory[MAX_PATH],path[MAX_PATH],option[8];
    DWORD length=GetEnvironmentVariableA("SRP_DIAGNOSTIC_SOURCE_RAIN_DIR",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || strcmp(build,"026g") || !sun_aux_enabled || !sun_aux_translucency ||
        GetEnvironmentVariableA("SRP_LEGACY_FACTORY_RAIN_SHADOW",option,sizeof(option))!=1 || option[0]!='1') {
        error("source rain requires complete source diffuse and Factory shadow camera on 026g"); return 0;
    }
    const char *names[]={"rain.vs.dxbc","rain.ps.dxbc","vertices.bin","indices.bin","scroll-rain-bc3-srgb.bin"};
    const uint32_t hashes[]={0xc3e80103,0x8ad83c28,0xf016f90b,0x327dbd65,0x9557da1f};
    for (unsigned i=0;i<5;++i) {
        if (snprintf(path,sizeof(path),"%s/%s",directory,names[i])>=(int)sizeof(path)) return 0;
        FILE *file=fopen(path,"rb"); if (!file) { error("source rain asset missing"); return 0; }
        source_rain_assets[i]=malloc(source_rain_sizes[i]);
        int ok=source_rain_assets[i] && fread(source_rain_assets[i],1,source_rain_sizes[i],file)==source_rain_sizes[i] && fgetc(file)==EOF;
        fclose(file); uint32_t hash=2166136261u;
        if (ok) for (unsigned j=0;j<source_rain_sizes[i];++j) hash=(hash^((unsigned char *)source_rain_assets[i])[j])*16777619u;
        if (!ok || hash!=hashes[i]) { error("source rain asset identity mismatch"); return 0; }
    }
    source_rain_enabled=1; return 1;
}
static void tag_source_rain(ID3D11PixelShader *shader,const void *code,SIZE_T size) {
    const unsigned char digest[16]={0x06,0x35,0x8d,0x2e,0xb0,0x9e,0x93,0x97,0x37,0x71,0xdd,0x43,0x3d,0xac,0xd6,0xc4};
    if (source_rain_enabled && size==1752 && !memcmp((const char *)code+4,digest,16)) {
        unsigned marker=1; ID3D11PixelShader_SetPrivateData(shader,&source_rain_guid,sizeof(marker),&marker);
    }
}
static HRESULT source_rain_buffer(ID3D11Device *device,UINT bytes,UINT bind,const void *data,ID3D11Buffer **buffer) {
    D3D11_BUFFER_DESC desc={0}; desc.ByteWidth=bytes; desc.BindFlags=bind;
    D3D11_SUBRESOURCE_DATA initial={data,0,0};
    return ID3D11Device_CreateBuffer(device,&desc,data?&initial:NULL,buffer);
}
static int init_source_rain(ID3D11DeviceContext *context,ID3D11Texture2D *native) {
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(native,&desc);
    if (desc.Format!=DXGI_FORMAT_R11G11B10_FLOAT || desc.SampleDesc.Count!=1 || desc.ArraySize!=1 || desc.MipLevels!=1) return 0;
    if (source_rain_gpu_ready) {
        D3D11_TEXTURE2D_DESC previous; ID3D11Texture2D_GetDesc(source_rain_output,&previous);
        return previous.Width==desc.Width && previous.Height==desc.Height;
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device); HRESULT hr;
    hr=ID3D11Device_CreateVertexShader(device,source_rain_assets[0],source_rain_sizes[0],NULL,&source_rain_vs);
    if (SUCCEEDED(hr)) hr=original_create_ps(device,source_rain_assets[1],source_rain_sizes[1],NULL,&source_rain_ps);
    D3D11_INPUT_ELEMENT_DESC layout[]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D11_INPUT_PER_VERTEX_DATA,0}};
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateInputLayout(device,layout,2,source_rain_assets[0],source_rain_sizes[0],&source_rain_layout);
    if (SUCCEEDED(hr)) hr=source_rain_buffer(device,2400,D3D11_BIND_VERTEX_BUFFER,source_rain_assets[2],&source_rain_vertices);
    if (SUCCEEDED(hr)) hr=source_rain_buffer(device,864,D3D11_BIND_INDEX_BUFFER,source_rain_assets[3],&source_rain_indices);
    if (SUCCEEDED(hr)) hr=source_rain_buffer(device,160,D3D11_BIND_CONSTANT_BUFFER,NULL,&source_rain_world);
    if (SUCCEEDED(hr)) hr=source_rain_buffer(device,304,D3D11_BIND_CONSTANT_BUFFER,NULL,&source_rain_globals);
    if (SUCCEEDED(hr)) hr=source_rain_buffer(device,112,D3D11_BIND_CONSTANT_BUFFER,NULL,&source_rain_parameters);
    D3D11_TEXTURE2D_DESC texture={0}; texture.Width=texture.Height=512; texture.MipLevels=texture.ArraySize=1;
    texture.Format=DXGI_FORMAT_BC3_UNORM_SRGB; texture.SampleDesc.Count=1; texture.BindFlags=D3D11_BIND_SHADER_RESOURCE;
    D3D11_SUBRESOURCE_DATA pixels={source_rain_assets[4],2048,262144}; ID3D11Texture2D *rain=NULL;
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateTexture2D(device,&texture,&pixels,&rain);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)rain,NULL,&source_rain_texture);
    if (rain) ID3D11Texture2D_Release(rain);
    desc.Usage=D3D11_USAGE_DEFAULT; desc.CPUAccessFlags=desc.MiscFlags=0;
    desc.BindFlags=D3D11_BIND_RENDER_TARGET|D3D11_BIND_SHADER_RESOURCE;
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateTexture2D(device,&desc,NULL,&source_rain_output);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)source_rain_output,NULL,&source_rain_target);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateTexture2D(device,&desc,NULL,&source_rain_frame);
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)source_rain_frame,NULL,&source_rain_frame_view);
    for (unsigned i=0;i<5 && SUCCEEDED(hr);++i) {
        D3D11_SAMPLER_DESC sampler={0}; sampler.MaxLOD=FLT_MAX; sampler.ComparisonFunc=D3D11_COMPARISON_LESS_EQUAL;
        sampler.AddressU=sampler.AddressV=sampler.AddressW=i?D3D11_TEXTURE_ADDRESS_CLAMP:D3D11_TEXTURE_ADDRESS_WRAP;
        sampler.Filter=i==4?D3D11_FILTER_COMPARISON_MIN_MAG_LINEAR_MIP_POINT:(i==0 || i==3)?D3D11_FILTER_MIN_MAG_MIP_LINEAR:D3D11_FILTER_MIN_MAG_MIP_POINT;
        hr=ID3D11Device_CreateSamplerState(device,&sampler,&source_rain_samplers[i]);
    }
    D3D11_BLEND_DESC blend={0}; D3D11_RENDER_TARGET_BLEND_DESC *b=&blend.RenderTarget[0];
    b->BlendEnable=TRUE; b->SrcBlend=D3D11_BLEND_SRC_ALPHA; b->DestBlend=D3D11_BLEND_INV_SRC_ALPHA;
    b->BlendOp=b->BlendOpAlpha=D3D11_BLEND_OP_ADD; b->SrcBlendAlpha=D3D11_BLEND_ONE;
    b->DestBlendAlpha=D3D11_BLEND_ZERO; b->RenderTargetWriteMask=15;
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateBlendState(device,&blend,&source_rain_blend);
    D3D11_RASTERIZER_DESC raster={0}; raster.FillMode=D3D11_FILL_SOLID; raster.CullMode=D3D11_CULL_NONE; raster.DepthClipEnable=TRUE;
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateRasterizerState(device,&raster,&source_rain_raster);
    D3D11_DEPTH_STENCIL_DESC depth={0}; depth.DepthEnable=TRUE; depth.DepthFunc=D3D11_COMPARISON_LESS_EQUAL;
    if (SUCCEEDED(hr)) hr=ID3D11Device_CreateDepthStencilState(device,&depth,&source_rain_depth);
    ID3D11Device_Release(device); source_rain_gpu_ready=SUCCEEDED(hr); return source_rain_gpu_ready;
}
static int source_rain_read_buffer(ID3D11DeviceContext *context,ID3D11Buffer *buffer,void *out,UINT bytes) {
    if (!buffer) return 0;
    D3D11_BUFFER_DESC desc; ID3D11Buffer_GetDesc(buffer,&desc);
    if (desc.ByteWidth<bytes || desc.ByteWidth>16384) return 0;
    desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=desc.MiscFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    ID3D11Device *device=NULL; ID3D11Buffer *copy=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    HRESULT hr=ID3D11Device_CreateBuffer(device,&desc,NULL,&copy); ID3D11Device_Release(device);
    if (SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)copy,(ID3D11Resource *)buffer);
        D3D11_MAPPED_SUBRESOURCE mapped; hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)copy,0,D3D11_MAP_READ,0,&mapped);
        if (SUCCEEDED(hr)) { memcpy(out,mapped.pData,bytes); ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)copy,0); }
    }
    if (copy) ID3D11Buffer_Release(copy);
    return SUCCEEDED(hr);
}
static void source_rain_capture_texture(const Camera *camera,unsigned step) {
    char base[MAX_PATH+48],path[MAX_PATH+80]; snprintf(base,sizeof(base),"%s.source-rain-%u",camera->output,step);
    snprintf(path,sizeof(path),"%s.targets.json",base); FILE *file=fopen(path,"w");
    if (!file) { error("cannot save source rain output"); return; }
    fputs("[",file); dump_texture(source_rain_output,base,file,"private_rain_color",0,NULL); fputs("]\n",file); fclose(file);
}
static void source_rain_capture_inputs(const Camera *camera,ID3D11ShaderResourceView *views[5]) {
    char base[MAX_PATH+48],path[MAX_PATH+80]; snprintf(base,sizeof(base),"%s.source-rain-inputs",camera->output);
    snprintf(path,sizeof(path),"%s.targets.json",base); FILE *file=fopen(path,"w"); if (!file) return;
    fputs("[",file);
    for (unsigned i=0;i<5;++i) {
        ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL; D3D11_SHADER_RESOURCE_VIEW_DESC view;
        ID3D11ShaderResourceView_GetResource(views[i],&resource); ID3D11ShaderResourceView_GetDesc(views[i],&view);
        HRESULT hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture); ID3D11Resource_Release(resource);
        if (i) fputs(",",file);
        if (SUCCEEDED(hr)) { dump_texture(texture,base,file,"source_rain_input",i,&view); ID3D11Texture2D_Release(texture); }
        else fputs("{\"written\":false}",file);
    }
    fputs("]\n",file); fclose(file);
}
static void render_source_rain(ID3D11DeviceContext *context) {
    if (!source_rain_enabled) return;
    ID3D11PixelShader *ps=NULL; ID3D11DeviceContext_PSGetShader(context,&ps,NULL,NULL); if (!ps) return;
    unsigned marker=0; UINT size=sizeof(marker);
    if (SUCCEEDED(ID3D11PixelShader_GetPrivateData(ps,&composition_guid,&size,&marker)) && marker==1) {
        if (source_rain_position) ID3D11ShaderResourceView_Release(source_rain_position);
        source_rain_position=NULL; ID3D11DeviceContext_PSGetShaderResources(context,3,1,&source_rain_position);
        source_rain_position_frame=present_calls;
    }
    size=sizeof(marker); marker=0;
    HRESULT hr=ID3D11PixelShader_GetPrivateData(ps,&source_rain_guid,&size,&marker);
    if (FAILED(hr) || marker!=1) { ID3D11PixelShader_Release(ps); return; }
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id && source_rain_capture_id!=camera.id;
    LeaveCriticalSection(&lock);
    // Keep scroll history even on frames with no readback. Use the actual
    // camera director snapshot; captured GPU globals are checked below.
    if (source_rain_motion_frame!=present_calls) {
        float view[12]={0},length=sqrtf(camera.forward[0]*camera.forward[0]+camera.forward[1]*camera.forward[1]+camera.forward[2]*camera.forward[2]);
        float horizontal=hypotf(camera.forward[0],camera.forward[2]);
        if (!isfinite(length) || length<1e-6f || !isfinite(horizontal) || horizontal<1e-6f) {
            error("source rain camera direction invalid"); ID3D11PixelShader_Release(ps); return;
        }
        for (unsigned j=0;j<3;++j) view[8+j]=camera.forward[j]/length;
        view[0]=camera.forward[2]/horizontal; view[2]=-camera.forward[0]/horizontal;
        rain_source_origin(&source_rain_motion,camera.position,view,source_rain_origin);
        source_rain_motion_frame=present_calls;
    }
    if (!take) { ID3D11PixelShader_Release(ps); return; }
    if (!source_full_diffuse_view() || !source_rain_position || source_rain_position_frame!=present_calls) {
        error("source rain requires current complete lighting and position"); ID3D11PixelShader_Release(ps); return;
    }
    ID3D11GeometryShader *gs=NULL; ID3D11DeviceContext_GSGetShader(context,&gs,NULL,NULL);
    if (gs) { ID3D11GeometryShader_Release(gs); ID3D11PixelShader_Release(ps); error("source rain requires no geometry shader"); return; }
    ID3D11RenderTargetView *targets[8]={0}; ID3D11DepthStencilView *depth=NULL;
    ID3D11Buffer *vcb[2]={0},*pcb[2]={0}; ID3D11ShaderResourceView *views[5]={0};
    ID3D11DeviceContext_OMGetRenderTargets(context,8,targets,&depth);
    ID3D11DeviceContext_VSGetConstantBuffers(context,0,2,vcb); ID3D11DeviceContext_PSGetConstantBuffers(context,0,2,pcb);
    ID3D11DeviceContext_PSGetShaderResources(context,0,5,views);
    ID3D11Texture2D *native=NULL; ID3D11Resource *resource=NULL; float globals[104],shadow[16];
    if (!targets[0] || targets[1] || !depth || !views[1] || !source_rain_read_buffer(context,vcb[1],globals,sizeof(globals)) ||
        !source_rain_read_buffer(context,pcb[0],shadow,sizeof(shadow))) { error("source rain native inputs absent"); goto release; }
    for (unsigned i=0;i<104;++i) if (!isfinite(globals[i])) { error("source rain globals nonfinite"); goto release; }
    for (unsigned i=0;i<16;++i) if (!isfinite(shadow[i])) { error("source rain shadow nonfinite"); goto release; }
    for (unsigned i=0;i<3;++i) if (fabsf(globals[80+i]-camera.position[i])>.001f) { error("source rain camera snapshot differs from GPU"); goto release; }
    // Particle VS globals declare m_V2W but never upload it. Recover the
    // inverse from its actual orthonormal m_V instead of reading those zeros.
    float inverse[16]={0}; inverse[15]=1;
    for (unsigned r=0;r<3;++r) for (unsigned c=0;c<3;++c) {
        double dot=0;
        for (unsigned k=0;k<3;++k) dot+=globals[32+r*4+k]*globals[32+c*4+k];
        if (fabs(dot-(r==c?1:0))>1e-5) { error("source rain view is not orthonormal"); goto release; }
        inverse[r*4+c]=globals[32+c*4+r];
    }
    for (unsigned r=0;r<3;++r) {
        double translation=0;
        for (unsigned k=0;k<3;++k) translation-=inverse[r*4+k]*globals[35+k*4];
        inverse[r*4+3]=(float)translation;
        if (fabsf(inverse[r*4+3]-globals[80+r])>.001f) { error("source rain inverse view differs from eye"); goto release; }
    }
    ID3D11RenderTargetView_GetResource(targets[0],&resource);
    hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&native); ID3D11Resource_Release(resource);
    if (FAILED(hr) || !init_source_rain(context,native)) { error("cannot initialize source rain targets"); goto release; }
    D3D11_TEXTURE2D_DESC color_desc; ID3D11Texture2D_GetDesc(native,&color_desc);
    if (globals[56]!=color_desc.Width || globals[57]!=color_desc.Height) { error("source rain viewport mismatch"); goto release; }
    D3D11_SHADER_RESOURCE_VIEW_DESC position_desc,shadow_desc;
    ID3D11ShaderResourceView_GetDesc(source_rain_position,&position_desc); ID3D11ShaderResourceView_GetDesc(views[1],&shadow_desc);
    if (position_desc.Format!=DXGI_FORMAT_R16_FLOAT || shadow_desc.Format!=DXGI_FORMAT_R16_UNORM ||
        position_desc.ViewDimension!=D3D11_SRV_DIMENSION_TEXTURE2D || shadow_desc.ViewDimension!=D3D11_SRV_DIMENSION_TEXTURE2D) {
        error("source rain depth or shadow format changed"); goto release;
    }
    float old_globals[76]={0},parameters[28]={0},world[5][12],radii[5];
    memcpy(old_globals,globals,176); memcpy(old_globals+44,inverse,48);
    memcpy(old_globals+60,globals+80,12); memcpy(old_globals+68,globals+40,12);
    old_globals[71]=globals[100]; memcpy(old_globals+72,globals+56,16); memcpy(parameters,globals+76,16);
    parameters[4]=parameters[5]=1; parameters[7]=.8f; parameters[8]=2;
    for (unsigned r=0;r<4;++r) for (unsigned c=0;c<4;++c) {
        double value=0;
        for (unsigned k=0;k<4;++k) value+=shadow[r*4+k]*inverse[k*4+c];
        parameters[12+r*4+c]=(float)value;
    }
    rain_source_layers(old_globals[71],source_rain_origin,world,radii);
    ID3D11VertexShader *vs=NULL; ID3D11InputLayout *layout=NULL; ID3D11Buffer *vertex=NULL,*indices=NULL;
    UINT stride,offset,index_offset,reference,mask; DXGI_FORMAT index_format; D3D11_PRIMITIVE_TOPOLOGY topology;
    ID3D11SamplerState *samplers[5]={0}; ID3D11BlendState *blend=NULL; ID3D11RasterizerState *raster=NULL;
    ID3D11DepthStencilState *depth_state=NULL; float factors[4];
    ID3D11DeviceContext_VSGetShader(context,&vs,NULL,NULL); ID3D11DeviceContext_IAGetInputLayout(context,&layout);
    ID3D11DeviceContext_IAGetVertexBuffers(context,0,1,&vertex,&stride,&offset);
    ID3D11DeviceContext_IAGetIndexBuffer(context,&indices,&index_format,&index_offset); ID3D11DeviceContext_IAGetPrimitiveTopology(context,&topology);
    ID3D11DeviceContext_PSGetSamplers(context,0,5,samplers); ID3D11DeviceContext_RSGetState(context,&raster);
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factors,&mask); ID3D11DeviceContext_OMGetDepthStencilState(context,&depth_state,&reference);
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)source_rain_frame,(ID3D11Resource *)native);
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)source_rain_output,(ID3D11Resource *)native);
    ID3D11ShaderResourceView *inputs[5]={source_rain_position,source_rain_texture,source_rain_frame_view,views[1],legacy_local_diffuse_view};
    source_rain_capture_inputs(&camera,inputs); source_rain_capture_texture(&camera,0);
    ID3D11DeviceContext_OMSetRenderTargets(context,1,&source_rain_target,depth);
    ID3D11DeviceContext_OMSetBlendState(context,source_rain_blend,NULL,0xffffffff); ID3D11DeviceContext_OMSetDepthStencilState(context,source_rain_depth,0);
    ID3D11DeviceContext_RSSetState(context,source_rain_raster); ID3D11DeviceContext_IASetInputLayout(context,source_rain_layout);
    UINT shell_stride=24,zero=0;
    ID3D11DeviceContext_IASetVertexBuffers(context,0,1,&source_rain_vertices,&shell_stride,&zero);
    ID3D11DeviceContext_IASetIndexBuffer(context,source_rain_indices,DXGI_FORMAT_R16_UINT,0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_VSSetShader(context,source_rain_vs,NULL,0); ID3D11DeviceContext_PSSetShader(context,source_rain_ps,NULL,0);
    ID3D11Buffer *source_vcb[2]={source_rain_world,source_rain_globals},*source_pcb[2]={source_rain_parameters,source_rain_globals};
    ID3D11DeviceContext_VSSetConstantBuffers(context,0,2,source_vcb); ID3D11DeviceContext_PSSetConstantBuffers(context,0,2,source_pcb);
    ID3D11DeviceContext_PSSetSamplers(context,0,5,source_rain_samplers); ID3D11DeviceContext_PSSetShaderResources(context,0,5,inputs);
    ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)source_rain_globals,0,NULL,old_globals,0,0);
    char path[MAX_PATH+80]; snprintf(path,sizeof(path),"%s.source-rain-globals.bin",camera.output); write_bytes(path,old_globals,sizeof(old_globals));
    for (unsigned i=0;i<5;++i) {
        float transforms[40]={0}; memcpy(transforms+28,world[i],sizeof(world[i])); parameters[6]=radii[i];
        ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)source_rain_world,0,NULL,transforms,0,0);
        ID3D11DeviceContext_UpdateSubresource(context,(ID3D11Resource *)source_rain_parameters,0,NULL,parameters,0,0);
        original_draw_indexed(context,432,0,0); source_rain_capture_texture(&camera,i+1);
        snprintf(path,sizeof(path),"%s.source-rain-%u.world.bin",camera.output,i); write_bytes(path,transforms,sizeof(transforms));
        snprintf(path,sizeof(path),"%s.source-rain-%u.parameters.bin",camera.output,i); write_bytes(path,parameters,sizeof(parameters));
    }
    ID3D11DeviceContext_PSSetShaderResources(context,0,5,views); ID3D11DeviceContext_PSSetSamplers(context,0,5,samplers);
    ID3D11DeviceContext_VSSetConstantBuffers(context,0,2,vcb); ID3D11DeviceContext_PSSetConstantBuffers(context,0,2,pcb);
    ID3D11DeviceContext_VSSetShader(context,vs,NULL,0); ID3D11DeviceContext_PSSetShader(context,ps,NULL,0);
    ID3D11DeviceContext_IASetInputLayout(context,layout); ID3D11DeviceContext_IASetVertexBuffers(context,0,1,&vertex,&stride,&offset);
    ID3D11DeviceContext_IASetIndexBuffer(context,indices,index_format,index_offset); ID3D11DeviceContext_IASetPrimitiveTopology(context,topology);
    ID3D11DeviceContext_RSSetState(context,raster); ID3D11DeviceContext_OMSetRenderTargets(context,8,targets,depth);
    ID3D11DeviceContext_OMSetDepthStencilState(context,depth_state,reference); ID3D11DeviceContext_OMSetBlendState(context,blend,factors,mask);
    if (vs) ID3D11VertexShader_Release(vs);
    if (layout) ID3D11InputLayout_Release(layout);
    if (vertex) ID3D11Buffer_Release(vertex);
    if (indices) ID3D11Buffer_Release(indices);
    if (raster) ID3D11RasterizerState_Release(raster);
    if (blend) ID3D11BlendState_Release(blend);
    if (depth_state) ID3D11DepthStencilState_Release(depth_state);
    for (unsigned i=0;i<5;++i) if (samplers[i]) ID3D11SamplerState_Release(samplers[i]);
    snprintf(path,sizeof(path),"%s.source-rain.json",camera.output); FILE *file=fopen(path,"w");
    if (file) {
        fprintf(file,"{\"present\":%u,\"position_present\":%u,\"diffuse_present\":%u,\"shell_draws\":5,\"indices_per_shell\":432,\"scene_time\":%.9g,\"origin\":[%.9g,%.9g,%.9g],\"scroll\":[%.9g,%.9g],\"scope\":\"original shell shaders and motion on current native pre-rain color/depth and private encoded diffuse; private output only; reference phase/history not synchronized\"}\n",present_calls,source_rain_position_frame,sun_aux_frame,old_globals[71],source_rain_origin[0],source_rain_origin[1],source_rain_origin[2],source_rain_motion.offsets[0],source_rain_motion.offsets[1]);
        fclose(file);
    }
    source_rain_capture_id=camera.id;
release:
    if (native) ID3D11Texture2D_Release(native);
    for (unsigned i=0;i<8;++i) if (targets[i]) ID3D11RenderTargetView_Release(targets[i]);
    if (depth) ID3D11DepthStencilView_Release(depth);
    for (unsigned i=0;i<2;++i) { if (vcb[i]) ID3D11Buffer_Release(vcb[i]); if (pcb[i]) ID3D11Buffer_Release(pcb[i]); }
    for (unsigned i=0;i<5;++i) if (views[i]) ID3D11ShaderResourceView_Release(views[i]);
    ID3D11PixelShader_Release(ps);
}
