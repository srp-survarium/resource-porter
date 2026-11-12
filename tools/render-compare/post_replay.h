// Own an unchanged original draw as a reusable D3D11 command list.
// Diagnostic packets: replay_post.py / replay_skylight.py. No engine addresses.
// Creating a draw never modifies the caller's immediate-context state.
// The caller may update compatible owned textures before executing the list.
#ifndef SRP_POST_REPLAY_H
#define SRP_POST_REPLAY_H

typedef struct {
    const unsigned char *bytes;
    size_t size, at;
} PostReader;

typedef struct {
    IUnknown *objects[96];
    unsigned object_count;
    ID3D11Device *device;
    ID3D11CommandList *commands;
    ID3D11Texture2D *output;
    ID3D11Texture2D *secondary_output;
    UINT secondary_pixel_bytes;
    ID3D11Resource *inputs[16];
    ID3D11Buffer *pixel_constants[4];
    unsigned char *pixel_data[4];
    UINT pixel_sizes[4];
    unsigned width, height, pixel_bytes;
    DXGI_FORMAT format;
} PostReplay;

static const void *post_take(PostReader *reader,size_t length) {
    if (length>reader->size-reader->at) return NULL;
    const void *result=reader->bytes+reader->at; reader->at+=length; return result;
}
static int post_word(PostReader *reader,UINT *value) {
    const void *data=post_take(reader,4);
    if (!data) return 0;
    memcpy(value,data,4); return 1;
}
static const void *post_blob(PostReader *reader,UINT *size) {
    if (!post_word(reader,size) || !*size || *size>64*1024*1024) return NULL;
    return post_take(reader,*size);
}
static void post_replay_release(PostReplay *replay) {
    if (!replay) return;
    for (unsigned i=0;i<4;++i) free(replay->pixel_data[i]);
    for (unsigned i=replay->object_count;i;--i) IUnknown_Release(replay->objects[i-1]);
    free(replay);
}
// Takes the newly created reference, including on capacity failure.
static int post_keep(PostReplay *replay,void *object) {
    if (replay->object_count==sizeof(replay->objects)/sizeof(replay->objects[0])) {
        IUnknown_Release((IUnknown *)object); return 0;
    }
    replay->objects[replay->object_count++]=(IUnknown *)object; return 1;
}
static int post_texel_layout(DXGI_FORMAT format,UINT *block,UINT *bytes) {
    *block=1;
    switch (format) {
        case DXGI_FORMAT_R32G32B32A32_FLOAT: *bytes=16;break;
        case DXGI_FORMAT_R16G16B16A16_FLOAT: case DXGI_FORMAT_R32G32_FLOAT:
        case DXGI_FORMAT_R32G8X24_TYPELESS: case DXGI_FORMAT_D32_FLOAT_S8X24_UINT: *bytes=8;break;
        case DXGI_FORMAT_R24G8_TYPELESS: case DXGI_FORMAT_D24_UNORM_S8_UINT:
        case DXGI_FORMAT_R16G16_FLOAT: case DXGI_FORMAT_R32_FLOAT:
        case DXGI_FORMAT_R10G10B10A2_UNORM: case DXGI_FORMAT_R11G11B10_FLOAT:
        case DXGI_FORMAT_R8G8B8A8_UNORM: case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
        case DXGI_FORMAT_B8G8R8A8_TYPELESS: case DXGI_FORMAT_B8G8R8A8_UNORM: case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: *bytes=4;break;
        case DXGI_FORMAT_R16_FLOAT: case DXGI_FORMAT_R16_TYPELESS: case DXGI_FORMAT_R16_UNORM: case DXGI_FORMAT_R8G8_UNORM: *bytes=2;break;
        case DXGI_FORMAT_R8_UNORM: *bytes=1;break;
        case DXGI_FORMAT_BC1_TYPELESS: case DXGI_FORMAT_BC1_UNORM: case DXGI_FORMAT_BC1_UNORM_SRGB: *block=4;*bytes=8;break;
        case DXGI_FORMAT_BC2_TYPELESS: case DXGI_FORMAT_BC2_UNORM: case DXGI_FORMAT_BC2_UNORM_SRGB:
        case DXGI_FORMAT_BC3_TYPELESS: case DXGI_FORMAT_BC3_UNORM: case DXGI_FORMAT_BC3_UNORM_SRGB: *block=4;*bytes=16;break;
        default: return 0;
    }
    return 1;
}
static ID3D11Buffer *post_buffer(PostReplay *replay,PostReader *reader,ID3D11Device *device,UINT flags) {
    UINT size; const void *data=post_blob(reader,&size);
    if (!data || (flags==D3D11_BIND_CONSTANT_BUFFER && (size%16 || size>65536))) return NULL;
    D3D11_BUFFER_DESC desc={0}; desc.ByteWidth=size;desc.BindFlags=flags;
    D3D11_SUBRESOURCE_DATA initial={data,0,0}; ID3D11Buffer *buffer=NULL;
    if (FAILED(ID3D11Device_CreateBuffer(device,&desc,&initial,&buffer))) return NULL;
    if (!post_keep(replay,buffer)) return NULL;
    return buffer;
}

typedef void (STDMETHODCALLTYPE *PostDrawIndexed)(ID3D11DeviceContext *,UINT,UINT,INT);

static PostReplay *post_replay_create(ID3D11Device *device,const void *packet,size_t packet_size,
                                     PostDrawIndexed intercepted,PostDrawIndexed original) {
    PostReader reader={packet,packet_size,0};
    PostReplay *replay=calloc(1,sizeof(*replay));
    ID3D11DeviceContext *deferred=NULL;
    if (!replay) return NULL;
#define POST_REQUIRE(condition) do { if (!(condition)) goto failure; } while (0)
#define POST_WORD(value) POST_REQUIRE(post_word(&reader,&(value)))
#define POST_CREATE(call,object) do { POST_REQUIRE(SUCCEEDED(call)); POST_REQUIRE(post_keep(replay,(object))); } while (0)
    ID3D11Device_AddRef(device);POST_REQUIRE(post_keep(replay,device));replay->device=device;
    const void *magic=post_take(&reader,8);
    POST_REQUIRE(magic);
    int skylight_state=!memcmp(magic,"SKY011E2",8);
    int skylight=skylight_state || !memcmp(magic,"SKY011E1",8);
    int ssao=!memcmp(magic,"SSA011E1",8);
    POST_REQUIRE(skylight || ssao || !memcmp(magic,"PST011E1",8));
    UINT h[15]; for (unsigned i=0;i<15;++i) POST_WORD(h[i]);
    replay->width=h[0];replay->height=h[1];replay->format=h[2];replay->pixel_bytes=h[4];
    UINT width=h[0],height=h[1],pixel_bytes=h[4],block,element;
    POST_REQUIRE(width && height && width<=4096 && height<=4096 && h[8]==6 &&
                 (ssao?(h[11]==24 || h[11]==36):h[11]==(skylight?24u:28u)) && (h[13]==42 || h[13]==57));
    POST_REQUIRE(h[5]>=1 && h[5]<=3 && h[6]<=1 && h[7]<=1);
    POST_REQUIRE(post_texel_layout(h[2],&block,&element) && block==1 && element==pixel_bytes);
    POST_REQUIRE(SUCCEEDED(ID3D11Device_CreateDeferredContext(device,0,&deferred)));
    UINT size; const void *code=post_blob(&reader,&size); POST_REQUIRE(code);
    ID3D11PixelShader *ps=NULL;
    POST_CREATE(ID3D11Device_CreatePixelShader(device,code,size,NULL,&ps),ps);
    ID3D11DeviceContext_PSSetShader(deferred,ps,NULL,0);
    code=post_blob(&reader,&size); POST_REQUIRE(code);
    ID3D11VertexShader *vs=NULL;
    POST_CREATE(ID3D11Device_CreateVertexShader(device,code,size,NULL,&vs),vs);
    const D3D11_INPUT_ELEMENT_DESC elements[]={
        {"POSITIONT",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"COLOR",0,DXGI_FORMAT_R8G8B8A8_UNORM,0,16,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,20,D3D11_INPUT_PER_VERTEX_DATA,0},
    };
    const D3D11_INPUT_ELEMENT_DESC sky_elements[]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D11_INPUT_PER_VERTEX_DATA,0},
    };
    const D3D11_INPUT_ELEMENT_DESC eye_elements[]={
        {"POSITIONT",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"POSITION",0,DXGI_FORMAT_R32G32B32_FLOAT,0,16,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,28,D3D11_INPUT_PER_VERTEX_DATA,0},
    };
    ID3D11InputLayout *layout=NULL;
    const D3D11_INPUT_ELEMENT_DESC *selected=h[11]==24?sky_elements:h[11]==36?eye_elements:elements;
    POST_CREATE(ID3D11Device_CreateInputLayout(device,selected,h[11]==24?2:3,code,size,&layout),layout);
    ID3D11DeviceContext_VSSetShader(deferred,vs,NULL,0);
    ID3D11DeviceContext_IASetInputLayout(deferred,layout);
    for (unsigned stage=0;stage<2;++stage) {
        UINT count; POST_WORD(count); POST_REQUIRE(count<=4); unsigned seen=0;
        for (unsigned i=0;i<count;++i) {
            UINT slot; POST_WORD(slot); POST_REQUIRE(slot<4 && !(seen&(1u<<slot)));seen|=1u<<slot;
            size_t begin=reader.at;
            ID3D11Buffer *cb=post_buffer(replay,&reader,device,D3D11_BIND_CONSTANT_BUFFER);POST_REQUIRE(cb);
            if (!stage) ID3D11DeviceContext_VSSetConstantBuffers(deferred,slot,1,&cb);
            else {
                ID3D11DeviceContext_PSSetConstantBuffers(deferred,slot,1,&cb);replay->pixel_constants[slot]=cb;
                replay->pixel_sizes[slot]=(UINT)(reader.at-begin-4);
                replay->pixel_data[slot]=malloc(replay->pixel_sizes[slot]);POST_REQUIRE(replay->pixel_data[slot]);
                memcpy(replay->pixel_data[slot],reader.bytes+begin+4,replay->pixel_sizes[slot]);
            }
        }
    }
    ID3D11Buffer *vertices=post_buffer(replay,&reader,device,D3D11_BIND_VERTEX_BUFFER); POST_REQUIRE(vertices);
    ID3D11Buffer *indices=post_buffer(replay,&reader,device,D3D11_BIND_INDEX_BUFFER); POST_REQUIRE(indices);
    ID3D11DeviceContext_IASetVertexBuffers(deferred,0,1,&vertices,&h[11],&h[12]);
    ID3D11DeviceContext_IASetIndexBuffer(deferred,indices,h[13],h[14]);
    ID3D11DeviceContext_IASetPrimitiveTopology(deferred,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    UINT count;POST_WORD(count);POST_REQUIRE(count<=16);
    for (unsigned i=0;i<count;++i) {
        UINT t[10];for (unsigned j=0;j<10;++j) POST_WORD(t[j]);
        UINT slot=t[0],dimension=t[1],w=t[2],he=t[3],depth=t[4],mips=t[5];
        POST_REQUIRE(slot<16 && !replay->inputs[slot] && w && he && w<=4096 && he<=4096 && mips && mips<=13);
        POST_REQUIRE(t[8]<mips && t[9] && t[9]<=mips-t[8]);
        POST_REQUIRE((dimension==2 && depth==1) || (dimension==3 && depth && depth<=64 && w<=64 && he<=64));
        POST_REQUIRE(post_texel_layout(t[6],&block,&element));
        D3D11_SUBRESOURCE_DATA initial[13]={0};
        for (unsigned mip=0;mip<mips;++mip) {
            POST_WORD(initial[mip].SysMemPitch);POST_WORD(initial[mip].SysMemSlicePitch);
            initial[mip].pSysMem=post_blob(&reader,&size);POST_REQUIRE(initial[mip].pSysMem);
            UINT mw=w>>mip,mh=he>>mip,md=depth>>mip;if (!mw) mw=1;if (!mh) mh=1;if (!md) md=1;
            UINT pitch=((mw+block-1)/block)*element,slice=pitch*((mh+block-1)/block);
            POST_REQUIRE(initial[mip].SysMemPitch==pitch && initial[mip].SysMemSlicePitch==slice && (uint64_t)slice*md==size);
        }
        ID3D11Resource *resource=NULL;D3D11_SHADER_RESOURCE_VIEW_DESC view={0};view.Format=t[7];
        if (dimension==2) {
            D3D11_TEXTURE2D_DESC desc={0};desc.Width=w;desc.Height=he;desc.MipLevels=mips;desc.ArraySize=1;
            desc.Format=t[6];desc.SampleDesc.Count=1;desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
            ID3D11Texture2D *texture=NULL;
            POST_CREATE(ID3D11Device_CreateTexture2D(device,&desc,initial,&texture),texture);
            resource=(ID3D11Resource *)texture;view.ViewDimension=D3D11_SRV_DIMENSION_TEXTURE2D;
            view.Texture2D.MostDetailedMip=t[8];view.Texture2D.MipLevels=t[9];
        } else {
            D3D11_TEXTURE3D_DESC desc={0};desc.Width=w;desc.Height=he;desc.Depth=depth;desc.MipLevels=mips;
            desc.Format=t[6];desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
            ID3D11Texture3D *texture=NULL;
            POST_CREATE(ID3D11Device_CreateTexture3D(device,&desc,initial,&texture),texture);
            resource=(ID3D11Resource *)texture;view.ViewDimension=D3D11_SRV_DIMENSION_TEXTURE3D;
            view.Texture3D.MostDetailedMip=t[8];view.Texture3D.MipLevels=t[9];
        }
        replay->inputs[slot]=resource;ID3D11ShaderResourceView *srv=NULL;
        POST_CREATE(ID3D11Device_CreateShaderResourceView(device,resource,&view,&srv),srv);
        ID3D11DeviceContext_PSSetShaderResources(deferred,slot,1,&srv);
    }
    POST_WORD(count);POST_REQUIRE(count<=16);unsigned seen=0;
    _Static_assert(sizeof(D3D11_SAMPLER_DESC)==52,"post sampler layout");
    for (unsigned i=0;i<count;++i) {
        UINT slot;POST_WORD(slot);POST_REQUIRE(slot<16 && !(seen&(1u<<slot)));seen|=1u<<slot;
        const void *data=post_take(&reader,52);POST_REQUIRE(data);
        D3D11_SAMPLER_DESC desc;memcpy(&desc,data,sizeof(desc));ID3D11SamplerState *sampler=NULL;
        POST_CREATE(ID3D11Device_CreateSamplerState(device,&desc,&sampler),sampler);
        ID3D11DeviceContext_PSSetSamplers(deferred,slot,1,&sampler);
    }
    const void *before=post_blob(&reader,&size);
    POST_REQUIRE(before && (uint64_t)width*height*pixel_bytes==size);
    ID3D11DepthStencilView *depth_view=NULL;
    ID3D11RenderTargetView *secondary_target=NULL;
    D3D11_VIEWPORT viewport={0,0,(float)width,(float)height,0,1};
    if (ssao) {
        const void *vp=post_take(&reader,sizeof(viewport));POST_REQUIRE(vp);memcpy(&viewport,vp,sizeof(viewport));
        POST_REQUIRE(viewport.TopLeftX==0 && viewport.TopLeftY==0 && viewport.MinDepth==0 && viewport.MaxDepth==1 &&
                     (viewport.Width==width || viewport.Width==width*2) && (viewport.Height==height || viewport.Height==height*2));
        UINT extra;POST_WORD(extra);POST_REQUIRE(extra<=1);
        if (extra) {
            UINT format,bytes;POST_WORD(format);POST_WORD(bytes);
            POST_REQUIRE(format==DXGI_FORMAT_R16_FLOAT && bytes==2);
            const void *data=post_blob(&reader,&size);POST_REQUIRE(data && (uint64_t)width*height*bytes==size);
            D3D11_TEXTURE2D_DESC td={0};td.Width=width;td.Height=height;td.MipLevels=td.ArraySize=td.SampleDesc.Count=1;
            td.Format=format;td.BindFlags=D3D11_BIND_RENDER_TARGET;
            D3D11_SUBRESOURCE_DATA initial={data,width*bytes,0};
            POST_CREATE(ID3D11Device_CreateTexture2D(device,&td,&initial,&replay->secondary_output),replay->secondary_output);
            POST_CREATE(ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)replay->secondary_output,NULL,&secondary_target),secondary_target);
            replay->secondary_pixel_bytes=bytes;
        }
        UINT bound;POST_WORD(bound);POST_REQUIRE(bound<=1);
        if (bound) {
            UINT reference;POST_WORD(reference);POST_REQUIRE(reference==0);
            const void *data=post_blob(&reader,&size);POST_REQUIRE(data && size==sizeof(D3D11_DEPTH_STENCIL_DESC));
            D3D11_DEPTH_STENCIL_DESC ds;memcpy(&ds,data,sizeof(ds));
            POST_REQUIRE(ds.DepthEnable && ds.DepthFunc==D3D11_COMPARISON_LESS_EQUAL && !ds.DepthWriteMask &&
                         ds.StencilEnable && ds.StencilReadMask==255 && !ds.StencilWriteMask &&
                         ds.FrontFace.StencilFunc==D3D11_COMPARISON_NOT_EQUAL && ds.BackFace.StencilFunc==D3D11_COMPARISON_NOT_EQUAL &&
                         ds.FrontFace.StencilFailOp==D3D11_STENCIL_OP_KEEP && ds.FrontFace.StencilDepthFailOp==D3D11_STENCIL_OP_KEEP && ds.FrontFace.StencilPassOp==D3D11_STENCIL_OP_KEEP &&
                         ds.BackFace.StencilFailOp==D3D11_STENCIL_OP_KEEP && ds.BackFace.StencilDepthFailOp==D3D11_STENCIL_OP_KEEP && ds.BackFace.StencilPassOp==D3D11_STENCIL_OP_KEEP);
            ID3D11DepthStencilState *state=NULL;
            POST_CREATE(ID3D11Device_CreateDepthStencilState(device,&ds,&state),state);
            ID3D11DeviceContext_OMSetDepthStencilState(deferred,state,reference);
            data=post_blob(&reader,&size);POST_REQUIRE(data && (uint64_t)width*height*4==size);
            D3D11_TEXTURE2D_DESC td={0};td.Width=width;td.Height=height;td.MipLevels=td.ArraySize=td.SampleDesc.Count=1;
            td.Format=DXGI_FORMAT_R24G8_TYPELESS;td.BindFlags=D3D11_BIND_DEPTH_STENCIL;
            D3D11_SUBRESOURCE_DATA initial={data,width*4,0};ID3D11Texture2D *texture=NULL;
            POST_CREATE(ID3D11Device_CreateTexture2D(device,&td,&initial,&texture),texture);
            D3D11_DEPTH_STENCIL_VIEW_DESC vd={0};vd.Format=DXGI_FORMAT_D24_UNORM_S8_UINT;vd.ViewDimension=D3D11_DSV_DIMENSION_TEXTURE2D;
            POST_CREATE(ID3D11Device_CreateDepthStencilView(device,(ID3D11Resource *)texture,&vd,&depth_view),depth_view);
        }
    }
    if (skylight_state) {
        UINT write_mask,reference;POST_WORD(write_mask);POST_WORD(reference);
        POST_REQUIRE(write_mask==7 && reference==0);
        const void *state=post_blob(&reader,&size);POST_REQUIRE(state && size==sizeof(D3D11_DEPTH_STENCIL_DESC));
        D3D11_DEPTH_STENCIL_DESC ds;memcpy(&ds,state,sizeof(ds));
        POST_REQUIRE(!ds.DepthEnable && ds.StencilEnable && ds.StencilReadMask==255 && !ds.StencilWriteMask &&
                     ds.FrontFace.StencilFunc==D3D11_COMPARISON_NOT_EQUAL && ds.BackFace.StencilFunc==D3D11_COMPARISON_NOT_EQUAL &&
                     ds.FrontFace.StencilFailOp==D3D11_STENCIL_OP_KEEP && ds.FrontFace.StencilDepthFailOp==D3D11_STENCIL_OP_KEEP && ds.FrontFace.StencilPassOp==D3D11_STENCIL_OP_KEEP &&
                     ds.BackFace.StencilFailOp==D3D11_STENCIL_OP_KEEP && ds.BackFace.StencilDepthFailOp==D3D11_STENCIL_OP_KEEP && ds.BackFace.StencilPassOp==D3D11_STENCIL_OP_KEEP);
        ID3D11DepthStencilState *depth_state=NULL;
        POST_CREATE(ID3D11Device_CreateDepthStencilState(device,&ds,&depth_state),depth_state);
        ID3D11DeviceContext_OMSetDepthStencilState(deferred,depth_state,reference);
        const void *depth_data=post_blob(&reader,&size);POST_REQUIRE(depth_data && (uint64_t)width*height*4==size);
        D3D11_TEXTURE2D_DESC dd={0};dd.Width=width;dd.Height=height;dd.MipLevels=dd.ArraySize=1;
        dd.Format=DXGI_FORMAT_R24G8_TYPELESS;dd.SampleDesc.Count=1;dd.BindFlags=D3D11_BIND_DEPTH_STENCIL;
        D3D11_SUBRESOURCE_DATA di={depth_data,width*4,0};ID3D11Texture2D *depth_texture=NULL;
        POST_CREATE(ID3D11Device_CreateTexture2D(device,&dd,&di,&depth_texture),depth_texture);
        D3D11_DEPTH_STENCIL_VIEW_DESC vd={0};vd.Format=DXGI_FORMAT_D24_UNORM_S8_UINT;vd.ViewDimension=D3D11_DSV_DIMENSION_TEXTURE2D;
        POST_CREATE(ID3D11Device_CreateDepthStencilView(device,(ID3D11Resource *)depth_texture,&vd,&depth_view),depth_view);
        D3D11_BLEND_DESC bd={0};bd.RenderTarget[0].RenderTargetWriteMask=(UINT8)write_mask;
        ID3D11BlendState *blend=NULL;
        POST_CREATE(ID3D11Device_CreateBlendState(device,&bd,&blend),blend);
        ID3D11DeviceContext_OMSetBlendState(deferred,blend,NULL,0xffffffff);
    }
    POST_REQUIRE(reader.at==reader.size);
    D3D11_TEXTURE2D_DESC desc={0};desc.Width=width;desc.Height=height;desc.MipLevels=1;desc.ArraySize=1;
    desc.Format=h[2];desc.SampleDesc.Count=1;desc.BindFlags=D3D11_BIND_RENDER_TARGET;
    D3D11_SUBRESOURCE_DATA initial={before,width*pixel_bytes,0};
    POST_CREATE(ID3D11Device_CreateTexture2D(device,&desc,&initial,&replay->output),replay->output);
    D3D11_RENDER_TARGET_VIEW_DESC rtd={0};rtd.Format=h[3];rtd.ViewDimension=D3D11_RTV_DIMENSION_TEXTURE2D;
    ID3D11RenderTargetView *target=NULL;
    POST_CREATE(ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)replay->output,&rtd,&target),target);
    ID3D11RenderTargetView *targets[]={target,secondary_target};
    ID3D11DeviceContext_OMSetRenderTargets(deferred,secondary_target?2:1,targets,depth_view);
    ID3D11DeviceContext_RSSetViewports(deferred,1,&viewport);
    D3D11_RASTERIZER_DESC raster={0};raster.FillMode=D3D11_FILL_SOLID;raster.CullMode=h[5];raster.FrontCounterClockwise=h[6];raster.DepthClipEnable=h[7];
    ID3D11RasterizerState *rs=NULL;
    POST_CREATE(ID3D11Device_CreateRasterizerState(device,&raster,&rs),rs);
    ID3D11DeviceContext_RSSetState(deferred,rs);
    // DXVK may use different implementations for immediate/deferred contexts.
    // Bypass the observer only if this context actually shares its hooked slot.
    PostDrawIndexed draw=deferred->lpVtbl->DrawIndexed;
    if (draw==intercepted) { POST_REQUIRE(original);draw=original; }
    draw(deferred,h[8],h[9],(INT)h[10]);
    POST_CREATE(ID3D11DeviceContext_FinishCommandList(deferred,FALSE,&replay->commands),replay->commands);
    ID3D11DeviceContext_Release(deferred);
    return replay;
failure:
    if (deferred) ID3D11DeviceContext_Release(deferred);
    post_replay_release(replay);return NULL;
#undef POST_REQUIRE
#undef POST_WORD
#undef POST_CREATE
}
#endif
