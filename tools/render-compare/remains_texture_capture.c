/* Opt-in, bounded read-only capture of the two known remains textures.
 * Compare a64x64 compressed mip; never alter the live SRV/resource/state. */
static void capture_remains_texture(ID3D11DeviceContext *context) {
    static int configured,enabled;
    static unsigned request,seen_count,matched;
    static unsigned char expected[4][4096];
    static void *seen[512];
    static const char *names[]={"skeleton-bc2","rag-bc2","skeleton-bc3","rag-bc3"};
    if (!configured) {
        configured=1; char prefix[MAX_PATH],path[MAX_PATH+32];
        if (GetEnvironmentVariableA("SRP_REMAINS_PREFIX",prefix,sizeof(prefix))) {
            enabled=1;
            for (unsigned i=0;i<4;++i) {
                snprintf(path,sizeof(path),"%s-%s.bin",prefix,names[i]);
                FILE *file=fopen(path,"rb");
                if (!file) { enabled=0; break; }
                int ok=fread(expected[i],1,4096,file)==4096 && fgetc(file)==EOF;
                fclose(file); if (!ok) { enabled=0; break; }
            }
            if (!enabled) error("remains diagnostic fingerprint missing");
        }
    }
    if (!enabled) return;
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    /* Start just before the screenshot deadline so a deadline landing in
     * postprocessing cannot miss all geometry draws in that frame. */
    DWORD threshold=camera.settle_ms>250?camera.settle_ms-250:0;
    int take=camera.enabled && captured_id!=camera.id && GetTickCount()-applied_at>=threshold;
    LeaveCriticalSection(&lock); if (!take) return;
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.remains-textures.jsonl",camera.output);
    if (request!=camera.id) {
        request=camera.id; seen_count=matched=0;
        FILE *file=fopen(path,"w"); if (file) { fprintf(file,"{\"request\":%u,\"present\":%lu,\"cap\":512}\n",request,(unsigned long)present_calls); fclose(file); }
    }
    if (seen_count>=512 || matched==3) return;
    ID3D11ShaderResourceView *srv=NULL; ID3D11DeviceContext_PSGetShaderResources(context,0,1,&srv);
    if (!srv) return;
    ID3D11Resource *resource=NULL; ID3D11ShaderResourceView_GetResource(srv,&resource);
    D3D11_SHADER_RESOURCE_VIEW_DESC view; ID3D11ShaderResourceView_GetDesc(srv,&view);
    ID3D11ShaderResourceView_Release(srv);
    for (unsigned i=0;i<seen_count;++i) if (seen[i]==resource) { ID3D11Resource_Release(resource); return; }
    ID3D11Texture2D *texture=NULL;
    HRESULT hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource); if (FAILED(hr)) return;
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc);
    if (desc.ArraySize!=1 || desc.SampleDesc.Count!=1 || view.ViewDimension!=D3D11_SRV_DIMENSION_TEXTURE2D ||
        !((desc.Format>=DXGI_FORMAT_BC2_TYPELESS && desc.Format<=DXGI_FORMAT_BC2_UNORM_SRGB) ||
          (desc.Format>=DXGI_FORMAT_BC3_TYPELESS && desc.Format<=DXGI_FORMAT_BC3_UNORM_SRGB))) {
        ID3D11Texture2D_Release(texture); return;
    }
    unsigned mip=0;
    while (mip+1<desc.MipLevels && (desc.Width>>mip)>64) ++mip;
    if ((desc.Width>>mip)!=64 || (desc.Height>>mip)!=64) { ID3D11Texture2D_Release(texture); return; }
    seen[seen_count++]=texture;
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    D3D11_TEXTURE2D_DESC small=desc; small.Width=small.Height=64; small.MipLevels=1;
    small.Usage=D3D11_USAGE_STAGING; small.BindFlags=0; small.CPUAccessFlags=D3D11_CPU_ACCESS_READ; small.MiscFlags=0;
    ID3D11Texture2D *staging=NULL; hr=ID3D11Device_CreateTexture2D(device,&small,NULL,&staging);
    if (SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopySubresourceRegion(context,(ID3D11Resource *)staging,0,0,0,0,(ID3D11Resource *)texture,mip,NULL);
        D3D11_MAPPED_SUBRESOURCE mapped;
        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
        if (SUCCEEDED(hr)) {
            unsigned char pixels[4096];
            for (unsigned y=0;y<16;++y) memcpy(pixels+y*256,(char *)mapped.pData+y*mapped.RowPitch,256);
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
            for (unsigned i=0;i<4;++i) if (!memcmp(pixels,expected[i],4096)) {
                FILE *file=fopen(path,"a");
                if (file) { fprintf(file,"{\"name\":\"%s\",\"present\":%lu,\"width\":%u,\"height\":%u,\"mips\":%u,\"format\":%u,\"srv_format\":%u,\"srv_most_detailed_mip\":%u,\"srv_mips\":%u,\"fingerprint_mip\":%u,\"checked_resources\":%u}\n",names[i],(unsigned long)present_calls,desc.Width,desc.Height,desc.MipLevels,desc.Format,view.Format,view.Texture2D.MostDetailedMip,view.Texture2D.MipLevels,mip,seen_count); fclose(file); }
                matched|=1u<<(i%2);
            }
        }
        ID3D11Texture2D_Release(staging);
    }
    ID3D11Device_Release(device); ID3D11Texture2D_Release(texture);
}
