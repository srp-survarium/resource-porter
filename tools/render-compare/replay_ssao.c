// Run the archived 0.10b SSAO and spatial upsample on a saved R16_FLOAT depth.
// No client hooks or window are needed. Temporal history is deliberately absent.
#define COBJMACROS
#include <windows.h>
#include <d3d11.h>
#include <stdio.h>
#include <stdlib.h>
#include <float.h>

static ID3D11Device *device;
static ID3D11DeviceContext *context;

static void check(HRESULT hr,const char *action) {
    if (FAILED(hr)) { fprintf(stderr,"%s failed: %08lx\n",action,(unsigned long)hr); exit(1); }
}
static void *read_file(const char *path,size_t *size) {
    FILE *file=fopen(path,"rb");
    if (!file) { perror(path); exit(1); }
    if (fseek(file,0,SEEK_END)) exit(1);
    long length=ftell(file);
    if (length<=0 || length>256*1024*1024) exit(1);
    rewind(file); *size=(size_t)length;
    void *data=malloc(*size);
    if (!data || fread(data,1,*size,file)!=*size) exit(1);
    fclose(file); return data;
}
static ID3D11Texture2D *texture(UINT width,UINT height,DXGI_FORMAT format,UINT binds,const void *data,UINT pitch) {
    D3D11_TEXTURE2D_DESC desc={0};
    desc.Width=width; desc.Height=height; desc.MipLevels=1; desc.ArraySize=1;
    desc.Format=format; desc.SampleDesc.Count=1; desc.BindFlags=binds;
    D3D11_SUBRESOURCE_DATA initial={data,pitch,0};
    ID3D11Texture2D *result=NULL;
    check(ID3D11Device_CreateTexture2D(device,&desc,data?&initial:NULL,&result),"CreateTexture2D");
    return result;
}
static ID3D11ShaderResourceView *view(ID3D11Texture2D *tex) {
    ID3D11ShaderResourceView *result=NULL;
    check(ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)tex,NULL,&result),"CreateShaderResourceView");
    return result;
}
static ID3D11Buffer *constants(const void *data,UINT size) {
    D3D11_BUFFER_DESC desc={0}; desc.ByteWidth=size; desc.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
    D3D11_SUBRESOURCE_DATA initial={data,0,0};
    ID3D11Buffer *result=NULL;
    check(ID3D11Device_CreateBuffer(device,&desc,&initial,&result),"CreateBuffer");
    return result;
}
static ID3D11PixelShader *pixel_shader(const char *path) {
    size_t size; void *data=read_file(path,&size); ID3D11PixelShader *result=NULL;
    check(ID3D11Device_CreatePixelShader(device,data,size,NULL,&result),"CreatePixelShader");
    free(data); return result;
}
static void render(ID3D11Texture2D *target,ID3D11PixelShader *shader,UINT width,UINT height) {
    ID3D11RenderTargetView *rt=NULL;
    check(ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)target,NULL,&rt),"CreateRenderTargetView");
    ID3D11DeviceContext_OMSetRenderTargets(context,1,&rt,NULL);
    const float clear[4]={0,0,0,0}; ID3D11DeviceContext_ClearRenderTargetView(context,rt,clear);
    D3D11_VIEWPORT vp={0,0,(float)width,(float)height,0,1};
    ID3D11DeviceContext_RSSetViewports(context,1,&vp);
    ID3D11DeviceContext_PSSetShader(context,shader,NULL,0);
    ID3D11DeviceContext_Draw(context,3,0);
    ID3D11DeviceContext_OMSetRenderTargets(context,0,NULL,NULL);
    ID3D11RenderTargetView_Release(rt);
}
static void save(ID3D11Texture2D *tex,const char *prefix,const char *suffix,UINT pixel_bytes) {
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(tex,&desc);
    desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    ID3D11Texture2D *staging=NULL;
    check(ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging),"Create staging texture");
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)tex);
    D3D11_MAPPED_SUBRESOURCE map;
    check(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&map),"Map");
    char path[1024];
    if (snprintf(path,sizeof(path),"%s%s",prefix,suffix)>=(int)sizeof(path)) exit(1);
    FILE *file=fopen(path,"wb"); if (!file) { perror(path); exit(1); }
    for (UINT y=0;y<desc.Height;++y)
        if (fwrite((char *)map.pData+y*map.RowPitch,pixel_bytes,desc.Width,file)!=desc.Width) exit(1);
    if (fclose(file)) exit(1);
    ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
    ID3D11Texture2D_Release(staging);
}
int main(int argc,char **argv) {
    if (argc!=9) {
        fprintf(stderr,"usage: replay_ssao WIDTH HEIGHT VERTEX_DXBC ACCUM_DXBC UPSAMPLE_DXBC DEPTH_R16F NOISE_BGRA OUTPUT_PREFIX\n");
        return 2;
    }
    UINT width=(UINT)strtoul(argv[1],NULL,10),height=(UINT)strtoul(argv[2],NULL,10);
    if (!width || !height || width>8192 || height>8192 || width%2 || height%2) return 2;
    D3D_FEATURE_LEVEL feature;
    check(D3D11CreateDevice(NULL,D3D_DRIVER_TYPE_HARDWARE,NULL,0,NULL,0,D3D11_SDK_VERSION,&device,&feature,&context),"D3D11CreateDevice");
    size_t size; void *data=read_file(argv[3],&size); ID3D11VertexShader *vs=NULL;
    check(ID3D11Device_CreateVertexShader(device,data,size,NULL,&vs),"CreateVertexShader"); free(data);
    ID3D11DeviceContext_VSSetShader(context,vs,NULL,0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context,D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    D3D11_RASTERIZER_DESC raster={0}; raster.FillMode=D3D11_FILL_SOLID; raster.CullMode=D3D11_CULL_NONE; raster.DepthClipEnable=TRUE;
    ID3D11RasterizerState *rs=NULL;
    check(ID3D11Device_CreateRasterizerState(device,&raster,&rs),"CreateRasterizerState");
    ID3D11DeviceContext_RSSetState(context,rs);
    ID3D11SamplerState *samplers[2]={0};
    D3D11_SAMPLER_DESC sampler={0}; sampler.Filter=D3D11_FILTER_MIN_MAG_MIP_LINEAR;
    sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D11_TEXTURE_ADDRESS_WRAP;
    sampler.MaxLOD=FLT_MAX; sampler.ComparisonFunc=D3D11_COMPARISON_NEVER; sampler.MaxAnisotropy=1;
    check(ID3D11Device_CreateSamplerState(device,&sampler,&samplers[0]),"Create linear sampler");
    sampler.Filter=D3D11_FILTER_MIN_MAG_MIP_POINT;
    sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D11_TEXTURE_ADDRESS_CLAMP;
    check(ID3D11Device_CreateSamplerState(device,&sampler,&samplers[1]),"Create depth sampler");
    ID3D11DeviceContext_PSSetSamplers(context,0,2,samplers);
    data=read_file(argv[6],&size); if (size!=(size_t)width*height*2) return 2;
    ID3D11Texture2D *depth=texture(width,height,DXGI_FORMAT_R16_FLOAT,D3D11_BIND_SHADER_RESOURCE,data,width*2); free(data);
    data=read_file(argv[7],&size); if (size!=64) return 2;
    ID3D11Texture2D *noise=texture(4,4,DXGI_FORMAT_B8G8R8A8_UNORM,D3D11_BIND_SHADER_RESOURCE,data,16); free(data);
    ID3D11ShaderResourceView *inputs[2]={view(depth),view(noise)};
    ID3D11DeviceContext_PSSetShaderResources(context,0,2,inputs);
    float params[12]={0}; params[4]=2; params[5]=1.5f;
    float globals[76]={0}; globals[72]=(float)width; globals[73]=(float)height; globals[74]=1.f/width; globals[75]=1.f/height;
    ID3D11Buffer *cb[2]={constants(params,sizeof(params)),constants(globals,sizeof(globals))};
    ID3D11DeviceContext_PSSetConstantBuffers(context,0,2,cb);
    ID3D11PixelShader *accum=pixel_shader(argv[4]),*upsample=pixel_shader(argv[5]);
    ID3D11Texture2D *half=texture(width/2,height/2,DXGI_FORMAT_R16G16_FLOAT,D3D11_BIND_RENDER_TARGET|D3D11_BIND_SHADER_RESOURCE,NULL,0);
    render(half,accum,width/2,height/2);
    save(half,argv[8],".half.rg16f",4);
    ID3D11ShaderResourceView *half_view=view(half);
    ID3D11DeviceContext_PSSetShaderResources(context,1,1,&half_view);
    ID3D11DeviceContext_PSSetConstantBuffers(context,0,1,&cb[1]);
    ID3D11Texture2D *full=texture(width,height,DXGI_FORMAT_R8G8_UNORM,D3D11_BIND_RENDER_TARGET,NULL,0);
    render(full,upsample,width,height);
    save(full,argv[8],".full.rg8",2);
    printf("%ux%u: archived SSAO and spatial upsample complete; temporal filtering omitted\n",width,height);
    ID3D11DeviceContext_ClearState(context); ID3D11DeviceContext_Flush(context);
    ID3D11Texture2D_Release(full); ID3D11ShaderResourceView_Release(half_view); ID3D11Texture2D_Release(half);
    ID3D11PixelShader_Release(accum); ID3D11PixelShader_Release(upsample);
    for (int i=0;i<2;++i) { ID3D11Buffer_Release(cb[i]); ID3D11ShaderResourceView_Release(inputs[i]); ID3D11SamplerState_Release(samplers[i]); }
    ID3D11Texture2D_Release(depth); ID3D11Texture2D_Release(noise);
    ID3D11RasterizerState_Release(rs); ID3D11VertexShader_Release(vs);
    ID3D11DeviceContext_Release(context); ID3D11Device_Release(device);
    return 0;
}
