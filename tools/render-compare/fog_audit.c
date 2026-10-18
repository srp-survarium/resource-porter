// Opt-in read-only observation of the pinned retail 0.21/0.26 fog draw.
// No shader, uniform, texture or render-state substitution is performed.
static const GUID fog_audit_guid = {0x9c057d13,0xa6c1,0x4a26,{0x89,0x23,0x40,0x41,0x75,0x9a,0x92,0x04}};
static unsigned char fog_digests[10][16] = {
    {0xf4,0x1c,0xca,0xd1,0x6c,0xa5,0xe4,0x14,0xba,0xf0,0xb2,0x79,0x5e,0x5e,0x8a,0x56},
    {0x0f,0x99,0x55,0x0b,0xd7,0x4e,0x29,0x0a,0xcf,0x08,0xab,0x46,0xdd,0xc5,0xed,0x20}
};
static unsigned fog_digest_count = 2;
static unsigned fog_audited_id;
static const GUID trace_shader_guid = {0x7d90aa12,0x2263,0x429f,{0xb0,0x24,0x88,0x64,0x2e,0x10,0x9a,0x13}};
static int fog_trace_draws;
static unsigned trace_request,trace_count;
static int fog_audit_render_target;
static int fog_audit_after_draw;
static const unsigned char fog_lut_digests[2][16] = {
    {0xbd,0x5c,0x09,0x03,0x2e,0x10,0x81,0xde,0xa9,0xcb,0xad,0xd4,0x6e,0x95,0x6f,0x8a},
    {0x18,0x28,0x29,0x05,0xd7,0x75,0xeb,0xa6,0xbb,0x7b,0x12,0xe0,0x57,0x71,0x0d,0xfe}
};
static HRESULT (STDMETHODCALLTYPE *fog_original_create)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11PixelShader **);
static HRESULT (STDMETHODCALLTYPE *trace_original_create_vs)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11VertexShader **);
static void (STDMETHODCALLTYPE *fog_original_draw)(ID3D11DeviceContext *,UINT,UINT);
static void (STDMETHODCALLTYPE *fog_original_draw_indexed)(ID3D11DeviceContext *,UINT,UINT,INT);

static HRESULT STDMETHODCALLTYPE trace_create_vs(ID3D11Device *device,const void *code,SIZE_T size,
                                                ID3D11ClassLinkage *linkage,ID3D11VertexShader **shader) {
    HRESULT result=trace_original_create_vs(device,code,size,linkage,shader);
    if (SUCCEEDED(result) && shader && *shader && size>=20 && !memcmp(code,"DXBC",4))
        ID3D11VertexShader_SetPrivateData(*shader,&trace_shader_guid,16,(const char *)code+4);
    return result;
}

static HRESULT STDMETHODCALLTYPE fog_create(ID3D11Device *device,const void *code,SIZE_T size,
                                           ID3D11ClassLinkage *linkage,ID3D11PixelShader **shader) {
    HRESULT result=fog_original_create(device,code,size,linkage,shader);
    if (SUCCEEDED(result) && shader && *shader && size>=20 && !memcmp(code,"DXBC",4)) {
        if (fog_trace_draws) ID3D11PixelShader_SetPrivateData(*shader,&trace_shader_guid,16,(const char *)code+4);
        for (unsigned i=(fog_digest_count>2?2:0);i<fog_digest_count;++i) if (!memcmp((const char *)code+4,fog_digests[i],16)) {
            unsigned kind=i+1;
            ID3D11PixelShader_SetPrivateData(*shader,&fog_audit_guid,sizeof(kind),&kind);
        }
        if (fog_audit_render_target) for (unsigned i=0;i<2;++i) if (!memcmp((const char *)code+4,fog_lut_digests[i],16)) {
            unsigned kind=100+i;
            ID3D11PixelShader_SetPrivateData(*shader,&fog_audit_guid,sizeof(kind),&kind);
        }
    }
    return result;
}

static unsigned char *fog_read_buffer(ID3D11DeviceContext *context,UINT slot,UINT *size,int vertex) {
    ID3D11Buffer *buffer=NULL,*staging=NULL;ID3D11Device *device=NULL;
    unsigned char *data=NULL;
    if (vertex) ID3D11DeviceContext_VSGetConstantBuffers(context,slot,1,&buffer);
    else ID3D11DeviceContext_PSGetConstantBuffers(context,slot,1,&buffer);
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

static void fog_observe_lookup(ID3D11DeviceContext *context) {
    if (!fog_audit_render_target || ID3D11DeviceContext_GetType(context)!=D3D11_DEVICE_CONTEXT_IMMEDIATE) return;
    ID3D11PixelShader *shader=NULL;ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned kind=0;UINT tag_size=sizeof(kind);
    HRESULT tagged=ID3D11PixelShader_GetPrivateData(shader,&fog_audit_guid,&tag_size,&kind);
    ID3D11PixelShader_Release(shader);
    if (FAILED(tagged) || kind<100 || kind>101) return;
    UINT size=0;unsigned char *data=fog_read_buffer(context,0,&size,0);
    if (!data) return;
    char path[MAX_PATH+40];snprintf(path,sizeof(path),"%s.lookup.jsonl",control);
    FILE *file=fopen(path,"ab");
    if (file) {
        fprintf(file,"{\"build\":\"%s\",\"present\":%u,\"shader_digest\":\"",build,present_calls);
        for (unsigned i=0;i<16;++i) fprintf(file,"%02x",fog_lut_digests[kind-100][i]);
        fputs("\",\"draw_constants\":",file);fog_vector(file,(const float *)data,size/sizeof(float));
        fputs("}\n",file);fclose(file);
    }
    free(data);
}

static void fog_save_texture(ID3D11DeviceContext *context,ID3D11Resource *resource,const Camera *camera,FILE *json,const char *name,const char *suffix) {
    ID3D11Texture2D *texture=NULL,*staging=NULL;ID3D11Device *device=NULL;
    if (FAILED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) goto done;
    D3D11_TEXTURE2D_DESC desc;ID3D11Texture2D_GetDesc(texture,&desc);
    fprintf(json,",\"%s_descriptor\":{\"width\":%u,\"height\":%u,\"dxgi_format\":%u,\"samples\":%u,\"mips\":%u,\"array_size\":%u}",name,desc.Width,desc.Height,(unsigned)desc.Format,desc.SampleDesc.Count,desc.MipLevels,desc.ArraySize);
    unsigned stride=desc.Format==DXGI_FORMAT_R16G16B16A16_FLOAT?8:
                    desc.Format==DXGI_FORMAT_R32G32B32A32_FLOAT?16:
                    desc.Format==DXGI_FORMAT_R11G11B10_FLOAT || desc.Format==DXGI_FORMAT_R8G8B8A8_UNORM ||
                    desc.Format==DXGI_FORMAT_B8G8R8A8_UNORM || desc.Format==DXGI_FORMAT_R10G10B10A2_UNORM?4:0;
    if (!stride || desc.SampleDesc.Count!=1 || desc.MipLevels!=1 || desc.ArraySize!=1) goto done;
    ID3D11DeviceContext_GetDevice(context,&device);
    desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.MiscFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    if (FAILED(ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging))) goto done;
    ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,resource);
    D3D11_MAPPED_SUBRESOURCE mapped;
    if (FAILED(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped))) goto done;
    char path[MAX_PATH+40];snprintf(path,sizeof(path),"%s.%s.bin",camera->output,suffix);
    FILE *file=fopen(path,"wb");int saved=file!=NULL;
    unsigned step=!strncmp(suffix,"trace-",6)?8:1;
    if (file) {
        for (UINT y=0;y<desc.Height;y+=step) {
            if (step==1) {
                if (fwrite((char *)mapped.pData+y*mapped.RowPitch,stride,desc.Width,file)!=desc.Width) saved=0;
            } else for (UINT x=0;x<desc.Width;x+=step)
                if (fwrite((char *)mapped.pData+y*mapped.RowPitch+x*stride,stride,1,file)!=1) saved=0;
        }
        fclose(file);
    }
    ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
    if (saved) fprintf(json,",\"%s\":{\"width\":%u,\"height\":%u,\"dxgi_format\":%u,\"bytes_per_pixel\":%u}",name,(desc.Width+step-1)/step,(desc.Height+step-1)/step,(unsigned)desc.Format,stride);
done:
    if (staging) ID3D11Texture2D_Release(staging);
    if (texture) ID3D11Texture2D_Release(texture);
    if (device) ID3D11Device_Release(device);
}

static void fog_save_render_target(ID3D11DeviceContext *context,const Camera *camera,FILE *json) {
    if (!fog_audit_render_target) return;
    ID3D11RenderTargetView *view=NULL;ID3D11Resource *resource=NULL;
    ID3D11DeviceContext_OMGetRenderTargets(context,1,&view,NULL);
    if (view) {
        ID3D11RenderTargetView_GetResource(view,&resource);
        fog_save_texture(context,resource,camera,json,fog_audit_after_draw?"post_draw_target":"pre_draw_target",fog_audit_after_draw?"post-draw":"pre-draw");
        ID3D11Resource_Release(resource);ID3D11RenderTargetView_Release(view);
    }
    for (UINT slot=0;slot<2;++slot) {
        ID3D11ShaderResourceView *srv=NULL;
        ID3D11DeviceContext_PSGetShaderResources(context,slot,1,&srv);
        if (!srv) continue;
        ID3D11ShaderResourceView_GetResource(srv,&resource);
        char name[16],suffix[16];snprintf(name,sizeof(name),"ps_texture_%u",slot);snprintf(suffix,sizeof(suffix),"ps-t%u",slot);
        fog_save_texture(context,resource,camera,json,name,suffix);
        ID3D11Resource_Release(resource);ID3D11ShaderResourceView_Release(srv);
    }
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
    UINT size=0;unsigned char *data=fog_read_buffer(context,1,&size,0);
    int vertex_frame=0;
    if (!data || size<400) {
        free(data);data=fog_read_buffer(context,1,&size,1);vertex_frame=1;
    }
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
    fprintf(file,"\",\"buffer_bytes\":%u,\"frame_stage\":\"%s\",\"eye\":",size,vertex_frame?"vertex":"pixel");fog_vector(file,values+80,3);
    fputs(",\"sun_color\":",file);fog_vector(file,values+96,3);
    fputs(",\"to_sun_direction\":",file);fog_vector(file,values+88,3);
    for (unsigned i=0;i<4;++i) {
        fprintf(file,",\"atmosphere_parameters%u\":",i);fog_vector(file,values+60+4*i,4);
    }
    UINT draw_size=0;unsigned char *draw=fog_read_buffer(context,0,&draw_size,0);
    if (draw) {
        fputs(",\"draw_constants\":",file);fog_vector(file,(const float *)draw,draw_size/sizeof(float));
        free(draw);
    }
    draw=fog_read_buffer(context,0,&draw_size,1);
    if (draw) {
        fputs(",\"vertex_constants\":",file);fog_vector(file,(const float *)draw,draw_size/sizeof(float));
        free(draw);
    }
    ID3D11BlendState *blend=NULL;FLOAT factors[4];UINT sample_mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factors,&sample_mask);
    if (blend) {
        D3D11_BLEND_DESC desc;ID3D11BlendState_GetDesc(blend,&desc);
        const D3D11_RENDER_TARGET_BLEND_DESC *rt=&desc.RenderTarget[0];
        fprintf(file,",\"blend\":{\"enabled\":%u,\"src\":%u,\"dst\":%u,\"op\":%u,\"src_alpha\":%u,\"dst_alpha\":%u,\"op_alpha\":%u,\"write_mask\":%u}",
                (unsigned)rt->BlendEnable,(unsigned)rt->SrcBlend,(unsigned)rt->DestBlend,(unsigned)rt->BlendOp,
                (unsigned)rt->SrcBlendAlpha,(unsigned)rt->DestBlendAlpha,(unsigned)rt->BlendOpAlpha,(unsigned)rt->RenderTargetWriteMask);
        ID3D11BlendState_Release(blend);
    }
    fog_save_render_target(context,&camera,file);
    fputs("}\n",file);fclose(file);free(data);
    EnterCriticalSection(&lock);fog_audited_id=camera.id;LeaveCriticalSection(&lock);
}

// Small readbacks after the selected draw locate later effects without
// substituting any render state. Stop at the screenshot's frame boundary.
static void fog_trace(ID3D11DeviceContext *context,UINT vertices) {
    if (!fog_trace_draws || ID3D11DeviceContext_GetType(context)!=D3D11_DEVICE_CONTEXT_IMMEDIATE) return;
    Camera camera;int ready;
    EnterCriticalSection(&lock);
    ready=applied.enabled && applied_id==fog_audited_id && applied_id!=captured_id;
    camera=applied;LeaveCriticalSection(&lock);
    if (!ready) return;
    if (trace_request!=camera.id) { trace_request=camera.id;trace_count=0; }
    if (trace_count>=1024) return;
    ID3D11PixelShader *shader=NULL;unsigned char digest[16]={0};UINT size=16;
    ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (shader) { ID3D11PixelShader_GetPrivateData(shader,&trace_shader_guid,&size,digest);ID3D11PixelShader_Release(shader); }
    char path[MAX_PATH+40],suffix[32];snprintf(path,sizeof(path),"%s.trace.jsonl",camera.output);
    FILE *file=fopen(path,"ab");if (!file) return;
    snprintf(suffix,sizeof(suffix),"trace-%04u",trace_count);
    fprintf(file,"{\"draw\":%u,\"vertices\":%u,\"present\":%u,\"shader_digest\":\"",trace_count++,vertices,present_calls);
    for (unsigned i=0;i<16;++i) fprintf(file,"%02x",digest[i]);
    fputs("\"",file);
    ID3D11VertexShader *vs=NULL;memset(digest,0,16);size=16;
    ID3D11DeviceContext_VSGetShader(context,&vs,NULL,NULL);
    if (vs) { ID3D11VertexShader_GetPrivateData(vs,&trace_shader_guid,&size,digest);ID3D11VertexShader_Release(vs); }
    fputs(",\"vertex_shader_digest\":\"",file);
    for (unsigned i=0;i<16;++i) fprintf(file,"%02x",digest[i]);
    fputs("\"",file);
    ID3D11RenderTargetView *view=NULL;ID3D11Resource *resource=NULL;
    ID3D11DeviceContext_OMGetRenderTargets(context,1,&view,NULL);
    if (view) {
        ID3D11RenderTargetView_GetResource(view,&resource);
        fog_save_texture(context,resource,&camera,file,"target",suffix);
        ID3D11Resource_Release(resource);ID3D11RenderTargetView_Release(view);
    }
    fputs("}\n",file);fclose(file);
}

static void STDMETHODCALLTYPE fog_draw(ID3D11DeviceContext *context,UINT count,UINT start) {
    fog_observe_lookup(context);
    if (!fog_audit_after_draw) fog_observe(context);
    fog_original_draw(context,count,start);
    if (fog_audit_after_draw) fog_observe(context);
    fog_trace(context,count);
}
static void STDMETHODCALLTYPE fog_draw_indexed(ID3D11DeviceContext *context,UINT count,UINT start,INT base) {
    fog_observe_lookup(context);
    if (!fog_audit_after_draw) fog_observe(context);
    fog_original_draw_indexed(context,count,start,base);
    if (fog_audit_after_draw) fog_observe(context);
    fog_trace(context,count);
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
    fog_audit_render_target=GetEnvironmentVariableA("SRP_CAPTURE_AUDIT_RT",option,sizeof(option))==1 && option[0]=='1';
    fog_audit_after_draw=GetEnvironmentVariableA("SRP_CAPTURE_AUDIT_AFTER",option,sizeof(option))==1 && option[0]=='1';
    fog_trace_draws=GetEnvironmentVariableA("SRP_CAPTURE_TRACE_DRAWS",option,sizeof(option))==1 && option[0]=='1';
    if (strcmp(build,"021d") && strcmp(build,"026g")) { error("fog audit requires 021d or 026g");return 0; }
    char digest[264];DWORD length=GetEnvironmentVariableA("SRP_CAPTURE_FOG_DIGEST",digest,sizeof(digest));
    if (length) {
        if (length>=sizeof(digest) || length%33!=32) { error("invalid experimental shader digest list");return 0; }
        unsigned count=(length+1)/33;
        for (unsigned k=0;k<count;++k) {
            if (k && digest[k*33-1]!=',') { error("shader digests must be comma-separated");return 0; }
            for (unsigned i=0;i<16;++i) {
                unsigned byte=0;
                for (unsigned j=0;j<2;++j) {
                    char ch=digest[k*33+2*i+j];
                    unsigned nibble=(ch>='0' && ch<='9')?(unsigned)(ch-'0'):
                        (ch>='a' && ch<='f')?(unsigned)(ch-'a'+10):16;
                    if (nibble>15) { error("invalid experimental shader digest");return 0; }
                    byte=16*byte+nibble;
                }
                fog_digests[2+k][i]=(unsigned char)byte;
            }
        }
        fog_digest_count=2+count;
    }
    void **dev=*(void ***)device,**ctx=*(void ***)context;
    fog_original_create=(void *)dev[15];fog_original_draw=(void *)ctx[13];fog_original_draw_indexed=(void *)ctx[12];
    if (fog_trace_draws) {
        trace_original_create_vs=(void *)dev[12];
        if (!fog_patch(dev,12,(void *)trace_create_vs)) { error("cannot install vertex shader observer");return 0; }
    }
    if (!fog_patch(dev,15,(void *)fog_create) || !fog_patch(ctx,13,(void *)fog_draw) ||
        !fog_patch(ctx,12,(void *)fog_draw_indexed)) { error("cannot install fog observer");return 0; }
    return 1;
}
