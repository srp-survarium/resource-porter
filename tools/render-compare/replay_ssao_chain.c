// Execute unchanged original SSAO accumulation -> temporal mask -> resolve ->
// skylight, passing produced GPU resources between draws. History remains a
// captured input: this harness does not claim a moving temporal sequence.
#define COBJMACROS
#include <windows.h>
#include <d3d11.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "post_replay.h"

static void check(HRESULT hr,const char *action) {
    if (FAILED(hr)) { fprintf(stderr,"%s: %08lx\n",action,(unsigned long)hr);exit(1); }
}
static PostReplay *load(ID3D11Device *device,const char *path) {
    FILE *file=fopen(path,"rb");if (!file || fseek(file,0,SEEK_END)) exit(2);
    long size=ftell(file);rewind(file);if (size<8 || size>128*1024*1024) exit(2);
    void *data=malloc((size_t)size);if (!data || fread(data,1,(size_t)size,file)!=(size_t)size) exit(2);
    fclose(file);PostReplay *pass=post_replay_create(device,data,(size_t)size,NULL,NULL);free(data);
    if (!pass) { fprintf(stderr,"invalid packet: %s\n",path);exit(1); }
    return pass;
}
static void connect_pass(ID3D11DeviceContext *context,PostReplay *source,PostReplay *destination,unsigned slot) {
    ID3D11Texture2D *target=NULL;
    if (slot>=16 || !destination->inputs[slot]) exit(1);
    check(ID3D11Resource_QueryInterface(destination->inputs[slot],&IID_ID3D11Texture2D,(void **)&target),"consumer texture");
    D3D11_TEXTURE2D_DESC a,b;ID3D11Texture2D_GetDesc(source->output,&a);ID3D11Texture2D_GetDesc(target,&b);
    if (a.Width!=b.Width || a.Height!=b.Height || a.Format!=b.Format || a.MipLevels!=b.MipLevels ||
        a.ArraySize!=b.ArraySize || a.SampleDesc.Count!=b.SampleDesc.Count || a.SampleDesc.Quality!=b.SampleDesc.Quality) {
        fprintf(stderr,"producer/consumer contract mismatch\n");exit(1);
    }
    ID3D11DeviceContext_CopyResource(context,destination->inputs[slot],(ID3D11Resource *)source->output);
    ID3D11Texture2D_Release(target);
}
static void save(ID3D11Device *device,ID3D11DeviceContext *context,ID3D11Texture2D *texture,UINT bytes,const char *base,const char *suffix) {
    D3D11_TEXTURE2D_DESC desc;ID3D11Texture2D_GetDesc(texture,&desc);
    desc.BindFlags=0;desc.Usage=D3D11_USAGE_STAGING;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    ID3D11Texture2D *staging=NULL;check(ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging),"readback texture");
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)texture);
    D3D11_MAPPED_SUBRESOURCE mapped;check(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped),"readback map");
    char path[4096];int length=snprintf(path,sizeof(path),"%s%s",base,suffix);if (length<0 || length>=(int)sizeof(path)) exit(2);
    FILE *file=fopen(path,"wb");if (!file) exit(2);
    for (UINT y=0;y<desc.Height;++y)
        if (fwrite((char *)mapped.pData+y*mapped.RowPitch,desc.Width*bytes,1,file)!=1) exit(2);
    if (fclose(file)) exit(2);
    ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);ID3D11Texture2D_Release(staging);
}
int main(int argc,char **argv) {
    int reset=argc==7 && !strcmp(argv[6],"--reset-history");
    if (argc!=6 && !reset) { fprintf(stderr,"usage: replay_ssao_chain ACCUM_PACKET MASK_PACKET TEMPORAL_PACKET SKY_PACKET OUTPUT_BASE [--reset-history]\n");return 2; }
    ID3D11Device *device=NULL;ID3D11DeviceContext *context=NULL;D3D_FEATURE_LEVEL feature;
    check(D3D11CreateDevice(NULL,D3D_DRIVER_TYPE_HARDWARE,NULL,0,NULL,0,D3D11_SDK_VERSION,&device,&feature,&context),"device");
    PostReplay *passes[4];for (unsigned i=0;i<4;++i) passes[i]=load(device,argv[i+1]);
    if (!passes[2]->secondary_output) { fprintf(stderr,"temporal depth output absent\n");return 1; }
    for (unsigned repeat=0;repeat<2;++repeat) {
        ID3D11DeviceContext_ExecuteCommandList(context,passes[0]->commands,TRUE);
        ID3D11DeviceContext_ExecuteCommandList(context,passes[1]->commands,TRUE);
        if (reset) {
            const float zero[4]={0};
            ID3D11DeviceContext_ClearRenderTargetView(context,passes[1]->target_view,zero);
        }
        connect_pass(context,passes[0],passes[2],1);connect_pass(context,passes[1],passes[2],3);
        ID3D11DeviceContext_ExecuteCommandList(context,passes[2]->commands,TRUE);
        connect_pass(context,passes[2],passes[3],2);
        ID3D11DeviceContext_ExecuteCommandList(context,passes[3]->commands,TRUE);
    }
    save(device,context,passes[0]->output,passes[0]->pixel_bytes,argv[5],".accum.bin");
    save(device,context,passes[1]->output,passes[1]->pixel_bytes,argv[5],".mask.bin");
    save(device,context,passes[3]->output,passes[3]->pixel_bytes,argv[5],"");
    save(device,context,passes[2]->output,passes[2]->pixel_bytes,argv[5],".ao.bin");
    save(device,context,passes[2]->secondary_output,passes[2]->secondary_pixel_bytes,argv[5],".depth.bin");
    ID3D11DeviceContext_ClearState(context);ID3D11DeviceContext_Flush(context);
    for (unsigned i=0;i<4;++i) post_replay_release(passes[i]);
    ID3D11DeviceContext_Release(context);ID3D11Device_Release(device);
    puts("original SSAO -> skylight chain executed twice; history is captured input");return 0;
}
