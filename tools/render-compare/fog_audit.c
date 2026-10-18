// Opt-in read-only observation of the pinned retail 0.21/0.26 fog draw.
// No shader, uniform, texture or render-state substitution is performed.
static const GUID fog_audit_guid = {0x9c057d13,0xa6c1,0x4a26,{0x89,0x23,0x40,0x41,0x75,0x9a,0x92,0x04}};
static unsigned char fog_digests[3][16] = {
    {0xf4,0x1c,0xca,0xd1,0x6c,0xa5,0xe4,0x14,0xba,0xf0,0xb2,0x79,0x5e,0x5e,0x8a,0x56},
    {0x0f,0x99,0x55,0x0b,0xd7,0x4e,0x29,0x0a,0xcf,0x08,0xab,0x46,0xdd,0xc5,0xed,0x20}
};
static unsigned fog_digest_count = 2;
static unsigned fog_audited_id;
static HRESULT (STDMETHODCALLTYPE *fog_original_create)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11PixelShader **);
static void (STDMETHODCALLTYPE *fog_original_draw)(ID3D11DeviceContext *,UINT,UINT);
static void (STDMETHODCALLTYPE *fog_original_draw_indexed)(ID3D11DeviceContext *,UINT,UINT,INT);

static HRESULT STDMETHODCALLTYPE fog_create(ID3D11Device *device,const void *code,SIZE_T size,
                                           ID3D11ClassLinkage *linkage,ID3D11PixelShader **shader) {
    HRESULT result=fog_original_create(device,code,size,linkage,shader);
    if (SUCCEEDED(result) && shader && *shader && size>=20 && !memcmp(code,"DXBC",4)) {
        for (unsigned i=(fog_digest_count==3?2:0);i<fog_digest_count;++i) if (!memcmp((const char *)code+4,fog_digests[i],16)) {
            unsigned kind=i+1;
            ID3D11PixelShader_SetPrivateData(*shader,&fog_audit_guid,sizeof(kind),&kind);
        }
    }
    return result;
}

static unsigned char *fog_read_buffer(ID3D11DeviceContext *context,UINT slot,UINT *size) {
    ID3D11Buffer *buffer=NULL,*staging=NULL;ID3D11Device *device=NULL;
    unsigned char *data=NULL;
    ID3D11DeviceContext_PSGetConstantBuffers(context,slot,1,&buffer);
    if (!buffer) return NULL;
    D3D11_BUFFER_DESC desc;ID3D11Buffer_GetDesc(buffer,&desc);
    if (!desc.ByteWidth || desc.ByteWidth>4096) goto done;
    ID3D11DeviceContext_GetDevice(context,&device);
    desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;
    desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;desc.MiscFlags=0;desc.StructureByteStride=0;
    if (FAILED(ID3D11Device_CreateBuffer(device,&desc,NULL,&staging))) goto done;
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)buffer);
    D3D11_MAPPED_SUBRESOURCE mapped;
    if (FAILED(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped))) goto done;
    data=malloc(desc.ByteWidth);
    if (data) { memcpy(data,mapped.pData,desc.ByteWidth);*size=desc.ByteWidth; }
    ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
done:
    if (staging) ID3D11Buffer_Release(staging);
    if (device) ID3D11Device_Release(device);
    ID3D11Buffer_Release(buffer);
    return data;
}

static void fog_vector(FILE *file,const float *data,unsigned size) {
    fputc('[',file);
    for (unsigned i=0;i<size;++i) {
        if (i) fputc(',',file);
        if (isfinite(data[i])) fprintf(file,"%.9g",data[i]); else fputs("null",file);
    }
    fputc(']',file);
}

static void fog_observe(ID3D11DeviceContext *context) {
    Camera camera;int ready;
    EnterCriticalSection(&lock);
    ready=applied.enabled && applied_id!=fog_audited_id && applied_id!=captured_id &&
          GetTickCount()-applied_at>=applied.settle_ms;
    camera=applied;
    LeaveCriticalSection(&lock);
    if (!ready || ID3D11DeviceContext_GetType(context)!=D3D11_DEVICE_CONTEXT_IMMEDIATE) return;
    ID3D11PixelShader *shader=NULL;
    ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned kind=0;UINT tag_size=sizeof(kind);
    HRESULT tagged=ID3D11PixelShader_GetPrivateData(shader,&fog_audit_guid,&tag_size,&kind);
    ID3D11PixelShader_Release(shader);
    if (FAILED(tagged) || kind<1 || kind>fog_digest_count) return;
    UINT size=0;unsigned char *data=fog_read_buffer(context,1,&size);
    if (!data) return;
    const float *values=(const float *)data;
    // A loading/lobby director can render in the same frame. Record only the
    // fog draw whose uploaded eye position matches the requested gameplay pose.
    int matches=size>=400;
    for (unsigned i=0;matches && i<3;++i)
        matches=isfinite(values[80+i]) && fabsf(values[80+i]-camera.position[i])<.01f;
    if (!matches) { free(data);return; }
    char path[MAX_PATH+40];snprintf(path,sizeof(path),"%s.fog-cb1.bin",camera.output);
    FILE *file=fopen(path,"wb");
    if (!file) { free(data);error("cannot write fog buffer audit");return; }
    int saved=fwrite(data,1,size,file)==size;fclose(file);
    snprintf(path,sizeof(path),"%s.fog.json",camera.output);
    file=fopen(path,"w");
    if (!file || !saved) { if (file) fclose(file);free(data);error("cannot write fog audit");return; }
    fprintf(file,"{\"request\":%u,\"build\":\"%s\",\"shader_digest\":\"",camera.id,build);
    for (unsigned i=0;i<16;++i) fprintf(file,"%02x",fog_digests[kind-1][i]);
    fprintf(file,"\",\"buffer_bytes\":%u,\"eye\":",size);fog_vector(file,values+80,3);
    fputs(",\"sun_color\":",file);fog_vector(file,values+96,3);
    fputs(",\"to_sun_direction\":",file);fog_vector(file,values+88,3);
    for (unsigned i=0;i<4;++i) {
        fprintf(file,",\"atmosphere_parameters%u\":",i);fog_vector(file,values+60+4*i,4);
    }
    fputs("}\n",file);fclose(file);free(data);
    EnterCriticalSection(&lock);fog_audited_id=camera.id;LeaveCriticalSection(&lock);
}

static void STDMETHODCALLTYPE fog_draw(ID3D11DeviceContext *context,UINT count,UINT start) {
    fog_observe(context);fog_original_draw(context,count,start);
}
static void STDMETHODCALLTYPE fog_draw_indexed(ID3D11DeviceContext *context,UINT count,UINT start,INT base) {
    fog_observe(context);fog_original_draw_indexed(context,count,start,base);
}
static int fog_patch(void **table,unsigned slot,void *replacement) {
    DWORD protection,ignored;
    if (!VirtualProtect(table+slot,sizeof(void *),PAGE_READWRITE,&protection)) return 0;
    table[slot]=replacement;
    return VirtualProtect(table+slot,sizeof(void *),protection,&ignored)!=0;
}
static int install_fog_audit(ID3D11Device *device,ID3D11DeviceContext *context) {
    char option[4];
    if (GetEnvironmentVariableA("SRP_CAPTURE_AUDIT_FOG",option,sizeof(option))!=1 || option[0]!='1') return 1;
    if (strcmp(build,"021d") && strcmp(build,"026g")) { error("fog audit requires 021d or 026g");return 0; }
    char digest[34];DWORD length=GetEnvironmentVariableA("SRP_CAPTURE_FOG_DIGEST",digest,sizeof(digest));
    if (length) {
        if (length!=32) { error("experimental fog digest must contain 32 hexadecimal characters");return 0; }
        for (unsigned i=0;i<16;++i) {
            unsigned byte=0;
            for (unsigned j=0;j<2;++j) {
                char ch=digest[2*i+j];
                unsigned nibble=(ch>='0' && ch<='9')?(unsigned)(ch-'0'):
                    (ch>='a' && ch<='f')?(unsigned)(ch-'a'+10):16;
                if (nibble>15) { error("invalid experimental fog digest");return 0; }
                byte=16*byte+nibble;
            }
            fog_digests[2][i]=(unsigned char)byte;
        }
        fog_digest_count=3;
    }
    void **dev=*(void ***)device,**ctx=*(void ***)context;
    fog_original_create=(void *)dev[15];fog_original_draw=(void *)ctx[13];fog_original_draw_indexed=(void *)ctx[12];
    if (!fog_patch(dev,15,(void *)fog_create) || !fog_patch(ctx,13,(void *)fog_draw) ||
        !fog_patch(ctx,12,(void *)fog_draw_indexed)) { error("cannot install fog observer");return 0; }
    return 1;
}
