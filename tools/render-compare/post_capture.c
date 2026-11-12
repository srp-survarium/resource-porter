// Read-only evidence needed to replay an original post draw without a client.
// Existing mip-zero dumps keep their format; additional mip files are separate.
static int capture_post_geometry_enabled;

static void capture_post_mips(ID3D11DeviceContext *context,const char *base) {
    ID3D11ShaderResourceView *views[16]={0};
    ID3D11DeviceContext_PSGetShaderResources(context,0,16,views);
    for (unsigned slot=0;slot<16;++slot) if (views[slot]) {
        ID3D11Resource *resource=NULL;
        ID3D11ShaderResourceView_GetResource(views[slot],&resource);
        ID3D11Texture2D *texture=NULL;
        if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) {
            D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc);
            unsigned block=1,bytes=0;
            switch (desc.Format) {
                case DXGI_FORMAT_BC1_TYPELESS: case DXGI_FORMAT_BC1_UNORM: case DXGI_FORMAT_BC1_UNORM_SRGB: block=4;bytes=8;break;
                case DXGI_FORMAT_BC2_TYPELESS: case DXGI_FORMAT_BC2_UNORM: case DXGI_FORMAT_BC2_UNORM_SRGB:
                case DXGI_FORMAT_BC3_TYPELESS: case DXGI_FORMAT_BC3_UNORM: case DXGI_FORMAT_BC3_UNORM_SRGB: block=4;bytes=16;break;
                case DXGI_FORMAT_R8G8B8A8_UNORM: case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
                case DXGI_FORMAT_B8G8R8A8_TYPELESS: case DXGI_FORMAT_B8G8R8A8_UNORM: case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: bytes=4;break;
                default: break;
            }
            if (desc.MipLevels>1 && bytes && desc.ArraySize==1 && desc.SampleDesc.Count==1 && desc.Width<=4096 && desc.Height<=4096) {
                ID3D11Device *device=NULL; ID3D11Texture2D *staging=NULL;
                ID3D11DeviceContext_GetDevice(context,&device);
                desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;desc.MiscFlags=0;
                HRESULT hr=ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging);
                if (SUCCEEDED(hr)) {
                    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,resource);
                    for (UINT mip=1;mip<desc.MipLevels;++mip) {
                        D3D11_MAPPED_SUBRESOURCE mapped;
                        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,mip,D3D11_MAP_READ,0,&mapped);
                        if (FAILED(hr)) { error("cannot map post texture mip"); break; }
                        UINT width=desc.Width>>mip,height=desc.Height>>mip;
                        if (!width) width=1;
                        if (!height) height=1;
                        UINT rows=(height+block-1)/block,pitch=((width+block-1)/block)*bytes;
                        char path[MAX_PATH+96];snprintf(path,sizeof(path),"%s.rt%02u.mip%02u.bin",base,slot,mip);
                        FILE *file=fopen(path,"wb");int ok=file!=NULL;
                        if (file) {
                            for (UINT y=0;y<rows;++y) if (fwrite((char *)mapped.pData+y*mapped.RowPitch,pitch,1,file)!=1) ok=0;
                            if (fclose(file)) ok=0;
                        }
                        ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,mip);
                        if (!ok) { error("cannot save post texture mip"); break; }
                    }
                    ID3D11Texture2D_Release(staging);
                } else error("cannot stage post texture mips");
                ID3D11Device_Release(device);
            }
            ID3D11Texture2D_Release(texture);
        }
        ID3D11Resource_Release(resource); ID3D11ShaderResourceView_Release(views[slot]);
    }
}

static void capture_post_state(ID3D11DeviceContext *context,const char *base,UINT count,UINT start,INT vertex,int indexed) {
    capture_material_vertex(context,base);
    capture_rain_geometry(context,base,count,start,vertex,indexed);
    capture_post_mips(context,base);
    D3D11_VIEWPORT viewport={0};UINT n=1;
    ID3D11DeviceContext_RSGetViewports(context,&n,&viewport);
    ID3D11RenderTargetView *target=NULL;
    ID3D11DepthStencilView *depth=NULL;
    ID3D11DeviceContext_OMGetRenderTargets(context,1,&target,&depth);
    int depth_bound=depth!=NULL;
    if (depth) ID3D11DepthStencilView_Release(depth);
    ID3D11BlendState *blend=NULL;float factor[4];UINT sample_mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&sample_mask);
    D3D11_BLEND_DESC bs={0};bs.RenderTarget[0].RenderTargetWriteMask=15;
    if (blend) { ID3D11BlendState_GetDesc(blend,&bs);ID3D11BlendState_Release(blend); }
    D3D11_RENDER_TARGET_VIEW_DESC rt={0};
    if (target) { ID3D11RenderTargetView_GetDesc(target,&rt);ID3D11RenderTargetView_Release(target); }
    ID3D11RasterizerState *raster=NULL;D3D11_RASTERIZER_DESC rs={0};
    ID3D11DeviceContext_RSGetState(context,&raster);
    if (raster) { ID3D11RasterizerState_GetDesc(raster,&rs); ID3D11RasterizerState_Release(raster); }
    else { rs.FillMode=D3D11_FILL_SOLID;rs.CullMode=D3D11_CULL_BACK;rs.DepthClipEnable=TRUE; }
    char path[MAX_PATH+96];snprintf(path,sizeof(path),"%s.post-state.json",base);
    FILE *file=fopen(path,"w");
    if (!file) { error("cannot save post draw state"); return; }
    fprintf(file,"{\"viewport_count\":%u,\"viewport\":[%.9g,%.9g,%.9g,%.9g,%.9g,%.9g],\"target_view_format\":%u,\"target_dimension\":%u,\"fill_mode\":%u,\"cull_mode\":%u,\"front_ccw\":%s,\"depth_clip\":%s,\"scissor\":%s,\"depth_bias\":%d,\"depth_bias_clamp\":%.9g,\"slope_bias\":%.9g,\"depth_bound\":%s,\"blend_enabled\":%s,\"write_mask\":%u,\"sample_mask\":%u}\n",n,viewport.TopLeftX,viewport.TopLeftY,viewport.Width,viewport.Height,viewport.MinDepth,viewport.MaxDepth,rt.Format,rt.ViewDimension,rs.FillMode,rs.CullMode,rs.FrontCounterClockwise?"true":"false",rs.DepthClipEnable?"true":"false",rs.ScissorEnable?"true":"false",rs.DepthBias,rs.DepthBiasClamp,rs.SlopeScaledDepthBias,depth_bound?"true":"false",bs.RenderTarget[0].BlendEnable?"true":"false",bs.RenderTarget[0].RenderTargetWriteMask,sample_mask);
    fclose(file);
}
