// Optional read-only selection of a native cone by its authored world origin.
// The verified 026g cone VS stores row-major float3x4 m_W at cb0 byte 112.
static int volume_filter_enabled;
static float volume_filter_origin[3], volume_filter_radius;
static const GUID volume_filter_vs_guid={0x26cfd290,0xebd1,0x4729,{0x91,0x36,0xdb,0xd1,0xf3,0x22,0xc4,0x27}};
static HRESULT (STDMETHODCALLTYPE *volume_filter_original_create_vs)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11VertexShader **);
static HRESULT STDMETHODCALLTYPE volume_filter_create_vs(ID3D11Device *device,const void *code,SIZE_T size,ID3D11ClassLinkage *linkage,ID3D11VertexShader **shader) {
    static const unsigned char digest[]={0x4a,0x17,0x20,0xd5,0x00,0x46,0x13,0x54,0xff,0xd2,0xfa,0xdd,0xf8,0x95,0x83,0x07};
    HRESULT hr=volume_filter_original_create_vs(device,code,size,linkage,shader);
    if (SUCCEEDED(hr) && shader && *shader && size==3584 && !memcmp((const char *)code+4,digest,16)) {
        unsigned marker=1;
        ID3D11VertexShader_SetPrivateData(*shader,&volume_filter_vs_guid,sizeof(marker),&marker);
    }
    return hr;
}
static int configure_volume_filter(void) {
    char value[160],extra; DWORD length=GetEnvironmentVariableA("SRP_CAPTURE_VOLUME_ORIGIN",value,sizeof(value));
    if (!length) return 1;
    if (length>=sizeof(value) || strcmp(build,"026g") ||
        sscanf(value,"%f,%f,%f,%f %c",&volume_filter_origin[0],&volume_filter_origin[1],&volume_filter_origin[2],&volume_filter_radius,&extra)!=4 ||
        !isfinite(volume_filter_origin[0]) || !isfinite(volume_filter_origin[1]) || !isfinite(volume_filter_origin[2]) ||
        !isfinite(volume_filter_radius) || volume_filter_radius<=0 || volume_filter_radius>10) {
        error("volume observation filter requires 026g and x,y,z,radius (0 < radius <= 10)"); return 0;
    }
    volume_filter_enabled=1; return 1;
}
static void record_volume_candidate(const Camera *camera,const float *origin,int selected,const char *reason) {
    static unsigned request_id,count; static float seen[128][3]; static int missing_recorded;
    if (request_id!=camera->id) { request_id=camera->id; count=0; missing_recorded=0; }
    if (origin) {
        for (unsigned i=0;i<count;++i) if (!memcmp(seen[i],origin,3*sizeof(float))) return;
        if (count>=128) return;
        memcpy(seen[count++],origin,3*sizeof(float));
    } else { if (missing_recorded) return; missing_recorded=1; }
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.volume-selection.jsonl",camera->output);
    FILE *file=fopen(path,"a"); if (!file) return;
    fprintf(file,"{\"camera_id\":%u,\"target\":[%.9g,%.9g,%.9g],\"radius\":%.9g,\"origin\":",camera->id,
        volume_filter_origin[0],volume_filter_origin[1],volume_filter_origin[2],volume_filter_radius);
    if (origin) fprintf(file,"[%.9g,%.9g,%.9g]",origin[0],origin[1],origin[2]); else fputs("null",file);
    fprintf(file,",\"selected\":%s,\"reason\":\"%s\"}\n",selected?"true":"false",reason); fclose(file);
}
static int select_volume_capture(ID3D11DeviceContext *context,const Camera *camera) {
    if (!volume_filter_enabled) return 1;
    ID3D11VertexShader *shader=NULL; UINT size=sizeof(unsigned); unsigned marker=0;
    ID3D11DeviceContext_VSGetShader(context,&shader,NULL,NULL);
    HRESULT hr=shader?ID3D11VertexShader_GetPrivateData(shader,&volume_filter_vs_guid,&size,&marker):E_FAIL;
    if (shader) ID3D11VertexShader_Release(shader);
    if (FAILED(hr) || marker!=1 || size!=sizeof(marker)) {
        record_volume_candidate(camera,NULL,0,"unverified vertex shader"); return 0;
    }
    ID3D11Buffer *buffer=NULL,*staging=NULL; ID3D11Device *device=NULL;
    ID3D11DeviceContext_VSGetConstantBuffers(context,0,1,&buffer);
    if (!buffer) { record_volume_candidate(camera,NULL,0,"missing world constants"); return 0; }
    D3D11_BUFFER_DESC desc; ID3D11Buffer_GetDesc(buffer,&desc);
    if (desc.ByteWidth!=160) {
        ID3D11Buffer_Release(buffer); record_volume_candidate(camera,NULL,0,"unexpected world buffer size"); return 0;
    }
    ID3D11DeviceContext_GetDevice(context,&device);
    desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ; desc.MiscFlags=0;
    hr=ID3D11Device_CreateBuffer(device,&desc,NULL,&staging);
    float origin[3]={0}; int valid=0;
    if (SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)buffer);
        D3D11_MAPPED_SUBRESOURCE mapped;
        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
        if (SUCCEEDED(hr)) {
            const float *values=mapped.pData;
            origin[0]=values[31]; origin[1]=values[35]; origin[2]=values[39];
            valid=isfinite(origin[0]) && isfinite(origin[1]) && isfinite(origin[2]);
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        }
        ID3D11Buffer_Release(staging);
    }
    ID3D11Buffer_Release(buffer); ID3D11Device_Release(device);
    if (!valid) { record_volume_candidate(camera,NULL,0,"world readback failed"); return 0; }
    float distance2=0;
    for (unsigned i=0;i<3;++i) { float delta=origin[i]-volume_filter_origin[i]; distance2+=delta*delta; }
    int selected=distance2<=volume_filter_radius*volume_filter_radius;
    record_volume_candidate(camera,origin,selected,selected?"matched":"outside radius");
    return selected;
}
