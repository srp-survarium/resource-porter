// Replay one original 0.11e post draw from replay_post.py's validated packet.
// Uses unchanged VS/PS, captured geometry, constants, textures and samplers.
// No game, window, shader recompilation or presentation conversion is involved.
#define COBJMACROS
#include <windows.h>
#include <d3d11.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>

static ID3D11Device *device;
static ID3D11DeviceContext *context;
static FILE *packet;
static void fail(const char *reason) { fprintf(stderr,"%s\n",reason);exit(1); }
static void check(HRESULT result,const char *operation) {
    if (FAILED(result)) { fprintf(stderr,"%s: %08lx\n",operation,(unsigned long)result);exit(1); }
}
static void read_exact(void *data,size_t size) { if (fread(data,1,size,packet)!=size) fail("truncated replay packet"); }
static UINT number(void) { UINT result;read_exact(&result,4);return result; }
static void texel_layout(DXGI_FORMAT format,UINT *block,UINT *bytes) {
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
        default: fail("unsupported texture format");
    }
}
static void *blob(UINT *length) {
    *length=number();if (!*length || *length>64*1024*1024) fail("invalid replay blob size");
    void *data=malloc(*length);if (!data) fail("out of memory");read_exact(data,*length);return data;
}
static ID3D11Buffer *buffer(UINT flags) {
    UINT size;void *data=blob(&size);
    if (flags==D3D11_BIND_CONSTANT_BUFFER && (size%16 || size>65536)) fail("invalid constant buffer extent");
    D3D11_BUFFER_DESC desc={0};desc.ByteWidth=size;desc.BindFlags=flags;desc.Usage=D3D11_USAGE_IMMUTABLE;
    D3D11_SUBRESOURCE_DATA initial={data,0,0};ID3D11Buffer *result=NULL;
    check(ID3D11Device_CreateBuffer(device,&desc,&initial,&result),"CreateBuffer");free(data);return result;
}
static void constant_buffers(int vertex) {
    UINT count=number();if (count>4) fail("too many constant buffers");
    for (UINT i=0;i<count;++i) {
        UINT slot=number();if (slot>=4) fail("invalid constant buffer slot");
        ID3D11Buffer *cb=buffer(D3D11_BIND_CONSTANT_BUFFER);
        if (vertex) ID3D11DeviceContext_VSSetConstantBuffers(context,slot,1,&cb);
        else ID3D11DeviceContext_PSSetConstantBuffers(context,slot,1,&cb);
        ID3D11Buffer_Release(cb);
    }
}
static void textures(void) {
    UINT count=number();if (count>16) fail("too many textures");
    for (UINT i=0;i<count;++i) {
        UINT slot=number(),dimension=number(),width=number(),height=number(),depth=number(),mips=number();
        DXGI_FORMAT format=(DXGI_FORMAT)number(),view_format=(DXGI_FORMAT)number();
        UINT first_mip=number(),view_mips=number();
        if (slot>=16 || !width || !height || width>4096 || height>4096 || !mips || mips>13 || first_mip>=mips || !view_mips || view_mips>mips-first_mip) fail("invalid texture contract");
        D3D11_SUBRESOURCE_DATA initial[13]={0};
        UINT block,bytes;texel_layout(format,&block,&bytes);
        for (UINT mip=0;mip<mips;++mip) {
            initial[mip].SysMemPitch=number();initial[mip].SysMemSlicePitch=number();
            UINT size;initial[mip].pSysMem=blob(&size);
            UINT slices=dimension==3?(depth>>mip):1;if (!slices) slices=1;
            UINT w=width>>mip,h=height>>mip;if (!w) w=1;if (!h) h=1;
            UINT pitch=((w+block-1)/block)*bytes,slice=pitch*((h+block-1)/block);
            if (initial[mip].SysMemPitch!=pitch || initial[mip].SysMemSlicePitch!=slice || (uint64_t)slice*slices!=size) fail("invalid texture payload extent");
        }
        ID3D11Resource *resource=NULL;
        D3D11_SHADER_RESOURCE_VIEW_DESC view={0};view.Format=view_format;
        if (dimension==2 && depth==1) {
            D3D11_TEXTURE2D_DESC desc={0};desc.Width=width;desc.Height=height;desc.MipLevels=mips;desc.ArraySize=1;
            desc.Format=format;desc.SampleDesc.Count=1;desc.Usage=D3D11_USAGE_IMMUTABLE;desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
            ID3D11Texture2D *texture=NULL;
            check(ID3D11Device_CreateTexture2D(device,&desc,initial,&texture),"CreateTexture2D input");resource=(ID3D11Resource *)texture;
            view.ViewDimension=D3D11_SRV_DIMENSION_TEXTURE2D;view.Texture2D.MostDetailedMip=first_mip;view.Texture2D.MipLevels=view_mips;
        } else if (dimension==3 && depth && depth<=64 && width<=64 && height<=64) {
            D3D11_TEXTURE3D_DESC desc={0};desc.Width=width;desc.Height=height;desc.Depth=depth;desc.MipLevels=mips;
            desc.Format=format;desc.Usage=D3D11_USAGE_IMMUTABLE;desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
            ID3D11Texture3D *texture=NULL;
            check(ID3D11Device_CreateTexture3D(device,&desc,initial,&texture),"CreateTexture3D input");resource=(ID3D11Resource *)texture;
            view.ViewDimension=D3D11_SRV_DIMENSION_TEXTURE3D;view.Texture3D.MostDetailedMip=first_mip;view.Texture3D.MipLevels=view_mips;
        } else fail("unsupported texture dimension");
        for (UINT mip=0;mip<mips;++mip) free((void *)initial[mip].pSysMem);
        ID3D11ShaderResourceView *srv=NULL;
        check(ID3D11Device_CreateShaderResourceView(device,resource,&view,&srv),"CreateShaderResourceView");
        ID3D11DeviceContext_PSSetShaderResources(context,slot,1,&srv);
        ID3D11ShaderResourceView_Release(srv);ID3D11Resource_Release(resource);
    }
}
int main(int argc,char **argv) {
    if (argc!=3) { fprintf(stderr,"usage: replay_post PACKET OUTPUT_RAW\n");return 2; }
    packet=fopen(argv[1],"rb");if (!packet) fail("cannot open replay packet");
    char magic[8];read_exact(magic,8);if (memcmp(magic,"PST011E1",8)) fail("unknown replay packet version");
    UINT width=number(),height=number();DXGI_FORMAT format=(DXGI_FORMAT)number(),view_format=(DXGI_FORMAT)number();
    UINT pixel_bytes=number(),cull=number(),front_ccw=number(),depth_clip=number();
    UINT count=number(),start=number();INT base=(INT)number();
    UINT stride=number(),vertex_offset=number();DXGI_FORMAT index_format=(DXGI_FORMAT)number();UINT index_offset=number();
    if (!width || !height || width>4096 || height>4096 || (pixel_bytes!=4 && pixel_bytes!=8 && pixel_bytes!=16) || stride!=28 || count!=6 || (index_format!=DXGI_FORMAT_R16_UINT && index_format!=DXGI_FORMAT_R32_UINT)) fail("unsupported original fullscreen draw");
    UINT block,bytes;texel_layout(format,&block,&bytes);
    if (block!=1 || bytes!=pixel_bytes) fail("invalid target format extent");
    D3D_FEATURE_LEVEL level;
    check(D3D11CreateDevice(NULL,D3D_DRIVER_TYPE_HARDWARE,NULL,0,NULL,0,D3D11_SDK_VERSION,&device,&level,&context),"D3D11CreateDevice");
    UINT size;void *code=blob(&size);ID3D11PixelShader *ps=NULL;
    check(ID3D11Device_CreatePixelShader(device,code,size,NULL,&ps),"Create original PS");free(code);
    ID3D11DeviceContext_PSSetShader(context,ps,NULL,0);ID3D11PixelShader_Release(ps);
    code=blob(&size);ID3D11VertexShader *vs=NULL;
    check(ID3D11Device_CreateVertexShader(device,code,size,NULL,&vs),"Create original VS");
    const D3D11_INPUT_ELEMENT_DESC elements[]={
        {"POSITIONT",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"COLOR",0,DXGI_FORMAT_R8G8B8A8_UNORM,0,16,D3D11_INPUT_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,20,D3D11_INPUT_PER_VERTEX_DATA,0},
    };
    ID3D11InputLayout *layout=NULL;
    check(ID3D11Device_CreateInputLayout(device,elements,3,code,size,&layout),"Create original TL layout");free(code);
    ID3D11DeviceContext_VSSetShader(context,vs,NULL,0);ID3D11VertexShader_Release(vs);
    ID3D11DeviceContext_IASetInputLayout(context,layout);ID3D11InputLayout_Release(layout);
    constant_buffers(1);constant_buffers(0);
    ID3D11Buffer *vertices=buffer(D3D11_BIND_VERTEX_BUFFER),*indices=buffer(D3D11_BIND_INDEX_BUFFER);
    ID3D11DeviceContext_IASetVertexBuffers(context,0,1,&vertices,&stride,&vertex_offset);
    ID3D11DeviceContext_IASetIndexBuffer(context,indices,index_format,index_offset);
    ID3D11Buffer_Release(vertices);ID3D11Buffer_Release(indices);
    ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    textures();UINT sampler_count=number();if (sampler_count>16) fail("too many samplers");
    _Static_assert(sizeof(D3D11_SAMPLER_DESC)==52,"sampler packet layout");
    for (UINT i=0;i<sampler_count;++i) {
        UINT slot=number();if (slot>=16) fail("invalid sampler slot");D3D11_SAMPLER_DESC desc;read_exact(&desc,sizeof(desc));
        ID3D11SamplerState *sampler=NULL;check(ID3D11Device_CreateSamplerState(device,&desc,&sampler),"CreateSamplerState");
        ID3D11DeviceContext_PSSetSamplers(context,slot,1,&sampler);ID3D11SamplerState_Release(sampler);
    }
    void *before=blob(&size);if ((uint64_t)width*height*pixel_bytes!=size) fail("invalid output extent");
    if (fgetc(packet)!=EOF) fail("unexpected trailing packet data");
    fclose(packet);
    D3D11_TEXTURE2D_DESC desc={0};desc.Width=width;desc.Height=height;desc.MipLevels=1;desc.ArraySize=1;
    desc.Format=format;desc.SampleDesc.Count=1;desc.BindFlags=D3D11_BIND_RENDER_TARGET;
    D3D11_SUBRESOURCE_DATA initial={before,width*pixel_bytes,0};ID3D11Texture2D *target=NULL;
    check(ID3D11Device_CreateTexture2D(device,&desc,&initial,&target),"Create output");free(before);
    D3D11_RENDER_TARGET_VIEW_DESC rtd={0};rtd.Format=view_format;rtd.ViewDimension=D3D11_RTV_DIMENSION_TEXTURE2D;
    ID3D11RenderTargetView *rt=NULL;check(ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)target,&rtd,&rt),"Create output view");
    ID3D11DeviceContext_OMSetRenderTargets(context,1,&rt,NULL);ID3D11RenderTargetView_Release(rt);
    D3D11_VIEWPORT viewport={0,0,(float)width,(float)height,0,1};ID3D11DeviceContext_RSSetViewports(context,1,&viewport);
    D3D11_RASTERIZER_DESC raster={0};raster.FillMode=D3D11_FILL_SOLID;raster.CullMode=cull;raster.FrontCounterClockwise=front_ccw;raster.DepthClipEnable=depth_clip;
    ID3D11RasterizerState *rs=NULL;check(ID3D11Device_CreateRasterizerState(device,&raster,&rs),"Create rasterizer");
    ID3D11DeviceContext_RSSetState(context,rs);ID3D11RasterizerState_Release(rs);
    ID3D11DeviceContext_DrawIndexed(context,count,start,base);
    ID3D11DeviceContext_OMSetRenderTargets(context,0,NULL,NULL);
    desc.BindFlags=0;desc.Usage=D3D11_USAGE_STAGING;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    ID3D11Texture2D *staging=NULL;check(ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging),"Create readback");
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)target);
    D3D11_MAPPED_SUBRESOURCE mapped;check(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped),"Map output");
    FILE *output=fopen(argv[2],"wb");if (!output) fail("cannot create output");
    for (UINT y=0;y<height;++y) if (fwrite((char *)mapped.pData+y*mapped.RowPitch,width*pixel_bytes,1,output)!=1) fail("cannot save output");
    if (fclose(output)) fail("cannot close output");
    ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
    ID3D11DeviceContext_ClearState(context);ID3D11DeviceContext_Flush(context);
    ID3D11Texture2D_Release(staging);ID3D11Texture2D_Release(target);ID3D11DeviceContext_Release(context);ID3D11Device_Release(device);
    printf("replayed unchanged original post: %ux%u, %u bytes per pixel\n",width,height,pixel_bytes);return 0;
}
