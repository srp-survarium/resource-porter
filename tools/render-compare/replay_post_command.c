// Exercise the reusable original-post command list, including state restoration.
#define COBJMACROS
#include <windows.h>
#include <d3d11.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "post_replay.h"

static void check(HRESULT result,const char *name) {
    if (FAILED(result)) { fprintf(stderr,"%s: %08lx\n",name,(unsigned long)result);exit(1); }
}
int main(int argc,char **argv) {
    if (argc!=3) { fprintf(stderr,"usage: replay_post_command PACKET OUTPUT_RAW\n");return 2; }
    FILE *file=fopen(argv[1],"rb");if (!file) return 2;
    if (fseek(file,0,SEEK_END)) return 2;
    long length=ftell(file);rewind(file);if (length<8 || length>256*1024*1024) return 2;
    unsigned char *packet=malloc((size_t)length+1);if (!packet) return 2;
    if (fread(packet,1,(size_t)length,file)!=(size_t)length) return 2;
    fclose(file);
    ID3D11Device *device=NULL;ID3D11DeviceContext *context=NULL;D3D_FEATURE_LEVEL level;
    check(D3D11CreateDevice(NULL,D3D_DRIVER_TYPE_HARDWARE,NULL,0,NULL,0,D3D11_SDK_VERSION,&device,&level,&context),"device");
    const size_t rejected_sizes[]={0,7,67,(size_t)length-1,(size_t)length+1};
    packet[length]=0;
    for (unsigned i=0;i<sizeof(rejected_sizes)/sizeof(rejected_sizes[0]);++i) {
        PostReplay *bad=post_replay_create(device,packet,rejected_sizes[i],NULL,NULL);
        if (bad) { post_replay_release(bad);fprintf(stderr,"malformed packet accepted\n");return 1; }
    }
    PostReplay *replay=post_replay_create(device,packet,(size_t)length,NULL,NULL);free(packet);
    if (!replay) { fprintf(stderr,"valid packet rejected\n");return 1; }
    // RestoreContextState must preserve the native engine's cached pipeline.
    D3D11_VIEWPORT sentinel={1,2,11,13,0.25f,0.75f};
    ID3D11DeviceContext_RSSetViewports(context,1,&sentinel);
    D3D11_RASTERIZER_DESC raster={0};raster.FillMode=D3D11_FILL_SOLID;raster.CullMode=D3D11_CULL_FRONT;
    ID3D11RasterizerState *rs=NULL;check(ID3D11Device_CreateRasterizerState(device,&raster,&rs),"sentinel rasterizer");
    ID3D11DeviceContext_RSSetState(context,rs);
    D3D11_BUFFER_DESC cbd={0};cbd.ByteWidth=16;cbd.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
    ID3D11Buffer *sentinel_cb=NULL;check(ID3D11Device_CreateBuffer(device,&cbd,NULL,&sentinel_cb),"sentinel constants");
    ID3D11DeviceContext_PSSetConstantBuffers(context,0,1,&sentinel_cb);
    ID3D11ShaderResourceView *sentinel_srv=NULL;
    ID3D11Resource *sentinel_texture=replay->inputs[2]?replay->inputs[2]:replay->inputs[0];
    check(ID3D11Device_CreateShaderResourceView(device,sentinel_texture,NULL,&sentinel_srv),"sentinel texture view");
    ID3D11DeviceContext_PSSetShaderResources(context,5,1,&sentinel_srv);
    ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_LINELIST);
    D3D11_DEPTH_STENCIL_DESC dsd={0};dsd.DepthEnable=TRUE;dsd.DepthFunc=D3D11_COMPARISON_GREATER;
    ID3D11DepthStencilState *sentinel_depth=NULL;
    check(ID3D11Device_CreateDepthStencilState(device,&dsd,&sentinel_depth),"sentinel depth state");
    ID3D11DeviceContext_OMSetDepthStencilState(context,sentinel_depth,37);
    D3D11_BLEND_DESC bd={0};bd.RenderTarget[0].RenderTargetWriteMask=8;
    ID3D11BlendState *sentinel_blend=NULL;
    check(ID3D11Device_CreateBlendState(device,&bd,&sentinel_blend),"sentinel blend state");
    const float sentinel_factors[4]={0.25f,0.5f,0.75f,1};
    ID3D11DeviceContext_OMSetBlendState(context,sentinel_blend,sentinel_factors,0x12345678);
    for (unsigned repeat=0;repeat<2;++repeat) {
        ID3D11DeviceContext_ExecuteCommandList(context,replay->commands,TRUE);
        D3D11_VIEWPORT actual={0};UINT count=1;ID3D11DeviceContext_RSGetViewports(context,&count,&actual);
        ID3D11RasterizerState *actual_rs=NULL;ID3D11DeviceContext_RSGetState(context,&actual_rs);
        int same=count==1 && !memcmp(&actual,&sentinel,sizeof(actual)) && actual_rs==rs;
        if (actual_rs) ID3D11RasterizerState_Release(actual_rs);
        ID3D11Buffer *cb=NULL;ID3D11DeviceContext_PSGetConstantBuffers(context,0,1,&cb);
        ID3D11ShaderResourceView *srv=NULL;ID3D11DeviceContext_PSGetShaderResources(context,5,1,&srv);
        ID3D11VertexShader *vs=NULL;ID3D11DeviceContext_VSGetShader(context,&vs,NULL,NULL);
        ID3D11RenderTargetView *rt=NULL;ID3D11DeviceContext_OMGetRenderTargets(context,1,&rt,NULL);
        D3D11_PRIMITIVE_TOPOLOGY topology;ID3D11DeviceContext_IAGetPrimitiveTopology(context,&topology);
        same=same && cb==sentinel_cb && srv==sentinel_srv && !vs && !rt && topology==D3D11_PRIMITIVE_TOPOLOGY_LINELIST;
        ID3D11DepthStencilState *actual_depth=NULL;UINT actual_reference=0;
        ID3D11DeviceContext_OMGetDepthStencilState(context,&actual_depth,&actual_reference);
        ID3D11BlendState *actual_blend=NULL;float actual_factors[4];UINT actual_mask=0;
        ID3D11DeviceContext_OMGetBlendState(context,&actual_blend,actual_factors,&actual_mask);
        same=same && actual_depth==sentinel_depth && actual_reference==37 && actual_blend==sentinel_blend &&
            !memcmp(actual_factors,sentinel_factors,sizeof(actual_factors)) && actual_mask==0x12345678;
        if (actual_depth) ID3D11DepthStencilState_Release(actual_depth);
        if (actual_blend) ID3D11BlendState_Release(actual_blend);
        if (cb) ID3D11Buffer_Release(cb);
        if (srv) ID3D11ShaderResourceView_Release(srv);
        if (vs) ID3D11VertexShader_Release(vs);
        if (rt) ID3D11RenderTargetView_Release(rt);
        if (!same) { fprintf(stderr,"immediate-context state was not restored\n");return 1; }
    }
    ID3D11Texture2D *outputs[]={replay->output,replay->secondary_output};
    for (unsigned slot=0;slot<2 && outputs[slot];++slot) {
        UINT pixel_bytes=slot?replay->secondary_pixel_bytes:replay->pixel_bytes;
        D3D11_TEXTURE2D_DESC desc;ID3D11Texture2D_GetDesc(outputs[slot],&desc);
        desc.BindFlags=0;desc.Usage=D3D11_USAGE_STAGING;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
        ID3D11Texture2D *staging=NULL;check(ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging),"readback");
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)outputs[slot]);
        D3D11_MAPPED_SUBRESOURCE mapped;check(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped),"map");
        char path[4096];int path_size=snprintf(path,sizeof(path),slot?"%s.rt01.bin":"%s",argv[2]);
        if (path_size<0 || path_size>=(int)sizeof(path)) return 2;
        file=fopen(path,"wb");if (!file) return 2;
        for (UINT y=0;y<replay->height;++y)
            if (fwrite((char *)mapped.pData+y*mapped.RowPitch,replay->width*pixel_bytes,1,file)!=1) return 2;
        if (fclose(file)) return 2;
        ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        ID3D11Texture2D_Release(staging);
    }
    ID3D11DeviceContext_ClearState(context);ID3D11DeviceContext_Flush(context);
    ID3D11RasterizerState_Release(rs);
    ID3D11DepthStencilState_Release(sentinel_depth);ID3D11BlendState_Release(sentinel_blend);
    ID3D11Buffer_Release(sentinel_cb);ID3D11ShaderResourceView_Release(sentinel_srv);post_replay_release(replay);
    ID3D11DeviceContext_Release(context);ID3D11Device_Release(device);
    puts("original post command list: two executions, state restored, five malformed packets rejected");return 0;
}
