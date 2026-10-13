// Optional vertex-stage evidence for the already selected material draw.
static HRESULT (STDMETHODCALLTYPE *original_create_vs)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11VertexShader **);
static HRESULT STDMETHODCALLTYPE create_material_vs(ID3D11Device *device,const void *code,SIZE_T size,ID3D11ClassLinkage *linkage,ID3D11VertexShader **shader) {
    HRESULT hr=original_create_vs(device,code,size,linkage,shader);
    if (SUCCEEDED(hr) && shader && *shader && size>=32 && size<=1024*1024)
        ID3D11VertexShader_SetPrivateData(*shader,&post_code_guid,(UINT)size,code);
    return hr;
}
static void capture_material_vertex(ID3D11DeviceContext *context,const char *base) {
    ID3D11VertexShader *shader=NULL;
    ID3D11DeviceContext_VSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    UINT size=0;
    HRESULT hr=ID3D11VertexShader_GetPrivateData(shader,&post_code_guid,&size,NULL);
    if (FAILED(hr) || size<32 || size>1024*1024) { ID3D11VertexShader_Release(shader); return; }
    void *code=malloc(size);
    if (!code) { ID3D11VertexShader_Release(shader); return; }
    hr=ID3D11VertexShader_GetPrivateData(shader,&post_code_guid,&size,code);
    ID3D11VertexShader_Release(shader);
    char path[MAX_PATH+96];
    snprintf(path,sizeof(path),"%s.vs.dxbc",base);
    if (SUCCEEDED(hr) && !write_bytes(path,code,size)) error("cannot save material vertex shader");
    free(code);
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    ID3D11Buffer *buffers[4]={0}; ID3D11DeviceContext_VSGetConstantBuffers(context,0,4,buffers);
    for (unsigned i=0;i<4;++i) if (buffers[i]) {
        D3D11_BUFFER_DESC desc; ID3D11Buffer_GetDesc(buffers[i],&desc);
        desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ; desc.MiscFlags=0;
        ID3D11Buffer *staging=NULL;
        hr=ID3D11Device_CreateBuffer(device,&desc,NULL,&staging);
        if (SUCCEEDED(hr)) {
            ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)buffers[i]);
            D3D11_MAPPED_SUBRESOURCE mapped;
            hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
            if (SUCCEEDED(hr)) {
                snprintf(path,sizeof(path),"%s.vs.cb%u.bin",base,i);
                if (!write_bytes(path,mapped.pData,desc.ByteWidth)) error("cannot save material vertex constants");
                ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
            }
            ID3D11Buffer_Release(staging);
        }
        ID3D11Buffer_Release(buffers[i]);
    }
    ID3D11Device_Release(device);
}
