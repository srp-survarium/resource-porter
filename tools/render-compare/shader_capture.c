// Capture the actual post-process draw, including its shader and bound inputs.
static const GUID post_code_guid={0x9205d461,0x758c,0x44fe,{0xad,0x61,0xa1,0x13,0x71,0x31,0x22,0xc3}};
static HRESULT (STDMETHODCALLTYPE *original_create_ps)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11PixelShader **);
static void (STDMETHODCALLTYPE *original_draw)(ID3D11DeviceContext *,UINT,UINT);
static void (STDMETHODCALLTYPE *original_draw_indexed)(ID3D11DeviceContext *,UINT,UINT,INT);
static unsigned draw_ids[9];
static unsigned probe_request_id, probe_shader_count;
static unsigned char probe_shader_digests[16][16];
static int capture_materials_enabled;
static unsigned material_request_id, material_shader_count;
static unsigned char material_shader_digests[128][16];
static void *legacy_lut_code;
static size_t legacy_lut_size;
static unsigned legacy_lut_uses;
static const GUID legacy_fog_guid={0x33b451e1,0x8051,0x4e0b,{0x81,0x62,0x27,0x6a,0xd5,0x21,0x50,0x31}};
static void *legacy_fog_code;
static size_t legacy_fog_size;
static unsigned legacy_fog_uses;
static void *legacy_sky_code, *legacy_cloud_code;
static size_t legacy_sky_size, legacy_cloud_size;
static unsigned legacy_sky_uses, legacy_cloud_uses;
static void *legacy_post_code;
static size_t legacy_post_size;
static unsigned legacy_post_uses;
static void *legacy_probe_code;
static size_t legacy_probe_size;
static unsigned legacy_probe_uses;
static void *legacy_probe_box_code;
static size_t legacy_probe_box_size;
static void *legacy_composition_code;
static size_t legacy_composition_size;
static unsigned legacy_composition_uses;
static ID3D11PixelShader *native_composition_shader;
static const GUID composition_guid={0x94bd30e1,0x2601,0x49c4,{0xa4,0x37,0x72,0x15,0x6b,0x1d,0x4c,0xe2}};
static unsigned composition_output_id;
static void *legacy_ambient_decode_code;
static size_t legacy_ambient_decode_size;
static unsigned legacy_ambient_decode_uses;
static ID3D11BlendState *legacy_decode_blend;

static int contains(const void *data, size_t length, const char *text) {
    size_t size=strlen(text)+1;
    if (length<size) return 0;
    for (size_t i=0;i<=length-size;++i) if (!memcmp((const char *)data+i,text,size)) return 1;
    return 0;
}

static HRESULT STDMETHODCALLTYPE create_ps(ID3D11Device *device,const void *code,SIZE_T size,ID3D11ClassLinkage *linkage,ID3D11PixelShader **shader) {
    const unsigned char expected[]={0x18,0x28,0x29,0x05,0xd7,0x75,0xeb,0xa6,0xbb,0x7b,0x12,0xe0,0x57,0x71,0x0d,0xfe};
    int replaced=legacy_lut_code && size==3508 && !memcmp((const char *)code+4,expected,16);
    const unsigned char fog_expected[]={0x0f,0x99,0x55,0x0b,0xd7,0x4e,0x29,0x0a,0xcf,0x08,0xab,0x46,0xdd,0xc5,0xed,0x20};
    int fog_replaced=legacy_fog_code && size==3608 && !memcmp((const char *)code+4,fog_expected,16);
    const unsigned char sky_expected[]={0xe7,0x17,0xa1,0x72,0x3c,0x00,0x86,0x3f,0x9d,0x5f,0x7b,0x57,0x62,0x15,0x5e,0x57};
    const unsigned char cloud_expected[]={0xd2,0xef,0x52,0xb2,0x15,0x6d,0xeb,0x31,0xd3,0x83,0x18,0xc3,0x8e,0x13,0x02,0xb5};
    int sky_replaced=legacy_sky_code && size==2588 && !memcmp((const char *)code+4,sky_expected,16);
    int cloud_replaced=legacy_cloud_code && size==4132 && !memcmp((const char *)code+4,cloud_expected,16);
    const unsigned char post_expected[]={0x22,0xa1,0xd1,0x39,0x98,0x9d,0x32,0x3c,0x71,0xb8,0x68,0x2b,0x30,0xbe,0xeb,0x48};
    int post_replaced=legacy_post_code && size==3972 && !memcmp((const char *)code+4,post_expected,16);
    const unsigned char probe_expected[]={0xa7,0x0e,0x27,0x2e,0x38,0xe4,0x26,0x66,0xe1,0x39,0xc6,0xc9,0xdb,0x33,0x51,0x66};
    int probe_replaced=legacy_probe_code && size==6104 && !memcmp((const char *)code+4,probe_expected,16);
    const unsigned char probe_box_expected[]={0x4f,0x5e,0x7d,0xc1,0xcf,0x82,0xff,0xa2,0x51,0x2c,0x12,0x5a,0x42,0xfe,0x1b,0xe5};
    int probe_box_replaced=legacy_probe_box_code && size==6116 && !memcmp((const char *)code+4,probe_box_expected,16);
    const unsigned char composition_expected[]={0xde,0x3e,0x7a,0x63,0x18,0x34,0xe1,0x5d,0xb4,0xae,0x34,0x73,0x59,0x86,0x7c,0x30};
    int composition_replaced=legacy_composition_code && size==8416 && !memcmp((const char *)code+4,composition_expected,16);
    const unsigned char ssao_expected[]={0xe8,0x52,0xc8,0x1d,0x2c,0xf9,0xc9,0x55,0x4a,0x60,0x46,0xc5,0x31,0x39,0x0e,0xc8};
    int ssao_reference=size==660 && !memcmp((const char *)code+4,ssao_expected,16);
    int ambient_decode_replaced=legacy_ambient_decode_code && ssao_reference;
    if (composition_replaced && !native_composition_shader &&
        FAILED(original_create_ps(device,code,size,linkage,&native_composition_shader))) {
        error("cannot create native composition reference");
        composition_replaced=0;
    }
    if (replaced) { code=legacy_lut_code; size=legacy_lut_size; }
    if (fog_replaced) { code=legacy_fog_code; size=legacy_fog_size; }
    if (sky_replaced) { code=legacy_sky_code; size=legacy_sky_size; }
    if (cloud_replaced) { code=legacy_cloud_code; size=legacy_cloud_size; }
    if (post_replaced) { code=legacy_post_code; size=legacy_post_size; }
    if (probe_replaced) { code=legacy_probe_code; size=legacy_probe_size; }
    if (probe_box_replaced) { code=legacy_probe_box_code; size=legacy_probe_box_size; }
    if (composition_replaced) { code=legacy_composition_code; size=legacy_composition_size; }
    if (ambient_decode_replaced) { code=legacy_ambient_decode_code; size=legacy_ambient_decode_size; }
    HRESULT hr=original_create_ps(device,code,size,linkage,shader);
    if (composition_replaced && SUCCEEDED(hr) && shader && *shader) {
        unsigned flag=1; ID3D11PixelShader_SetPrivateData(*shader,&composition_guid,sizeof(flag),&flag);
    }
    if (fog_replaced && SUCCEEDED(hr) && shader && *shader) {
        unsigned flag=1; ID3D11PixelShader_SetPrivateData(*shader,&legacy_fog_guid,sizeof(flag),&flag);
    }
    if (ambient_decode_replaced && SUCCEEDED(hr) && shader && *shader) {
        unsigned flag=2; ID3D11PixelShader_SetPrivateData(*shader,&legacy_fog_guid,sizeof(flag),&flag);
    }
    if (replaced || fog_replaced || sky_replaced || cloud_replaced || post_replaced || probe_replaced || probe_box_replaced || composition_replaced || ambient_decode_replaced) {
        if (FAILED(hr)) error("legacy atmosphere shader rejected");
        else if (replaced) ++legacy_lut_uses;
        else if (fog_replaced) ++legacy_fog_uses;
        else if (sky_replaced) ++legacy_sky_uses;
        else if (cloud_replaced) ++legacy_cloud_uses;
        else if (post_replaced) ++legacy_post_uses;
        else if (composition_replaced) ++legacy_composition_uses;
        else if (ambient_decode_replaced) ++legacy_ambient_decode_uses;
        else ++legacy_probe_uses;
        char path[MAX_PATH+32]; snprintf(path,sizeof(path),"%s.shader-overrides.json",control);
        FILE *file=fopen(path,"w");
        if (file) { fprintf(file,"{\"legacy_lut_uses\":%u,\"legacy_fog_uses\":%u,\"legacy_sky_uses\":%u,\"legacy_cloud_uses\":%u,\"legacy_post_uses\":%u,\"legacy_probe_uses\":%u,\"legacy_composition_uses\":%u,\"legacy_ambient_decode_uses\":%u,\"hresult\":%ld}\n",legacy_lut_uses,legacy_fog_uses,legacy_sky_uses,legacy_cloud_uses,legacy_post_uses,legacy_probe_uses,legacy_composition_uses,legacy_ambient_decode_uses,hr); fclose(file); }
    }
    if (SUCCEEDED(hr) && shader && *shader && size<=1024*1024 &&
        (ssao_reference || (contains(code,size,"t_color_grading_lut") && contains(code,size,"t_frame_luminance")) ||
        contains(code,size,"sky_clouds_parameters0") || contains(code,size,"t_probe_cubemap_diffuse") ||
        contains(code,size,"t_rayleigh_scattering") || contains(code,size,"hba_parameters") ||
        contains(code,size,"skylight_parameters0") ||
        (capture_materials_enabled && contains(code,size,"deferred_material_parameters")) ||
        (contains(code,size,"t_diffuse_accumulation") && contains(code,size,"t_specular_accumulation"))))
        ID3D11PixelShader_SetPrivateData(*shader,&post_code_guid,(UINT)size,code);
    return hr;
}

static int write_bytes(const char *path,const void *data,size_t size) {
    FILE *file=fopen(path,"wb"); if (!file) return 0;
    int ok=fwrite(data,1,size,file)==size;
    return fclose(file)==0 && ok;
}

static void capture_probe_initial(ID3D11DeviceContext *context,const char *output) {
    ID3D11RenderTargetView *views[2]={0};
    ID3D11DeviceContext_OMGetRenderTargets(context,2,views,NULL);
    char base[MAX_PATH+48],path[MAX_PATH+80];
    snprintf(base,sizeof(base),"%s.probe-initial",output);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *file=fopen(path,"w"); unsigned count=0;
    if (file) fputs("[",file);
    for (unsigned i=0;i<2;++i) if (views[i]) {
        ID3D11Resource *resource=NULL; ID3D11RenderTargetView_GetResource(views[i],&resource);
        ID3D11Texture2D *texture=NULL;
        if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) {
            if (file) { if (count++) fputs(",",file); dump_texture(texture,base,file,i?"specular_before_first_probe":"diffuse_before_first_probe",i,NULL); }
            ID3D11Texture2D_Release(texture);
        }
        ID3D11Resource_Release(resource); ID3D11RenderTargetView_Release(views[i]);
    }
    if (file) { fputs("]\n",file); fclose(file); }
}

static void capture_post_draw(ID3D11DeviceContext *context) {
    Camera camera;
    EnterCriticalSection(&lock);
    int take=applied.enabled && capture_frame_id==applied.id && captured_id!=applied.id && GetTickCount()-applied_at>=applied.settle_ms;
    camera=applied;
    LeaveCriticalSection(&lock);
    if (!take) return;
    ID3D11PixelShader *shader=NULL;
    ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    UINT size=0;
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,NULL);
    if (FAILED(hr) || !size || size>1024*1024) { ID3D11PixelShader_Release(shader); return; }
    void *code=malloc(size);
    if (!code) { ID3D11PixelShader_Release(shader); return; }
    hr=ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,code);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr)) { free(code); return; }
    unsigned kind=contains(code,size,"t_diffuse_accumulation") && contains(code,size,"t_specular_accumulation")?7:
        capture_materials_enabled && contains(code,size,"deferred_material_parameters")?8:
        contains(code,size,"hba_parameters") || contains(code,size,"t_encoded_ambient") || (contains(code,size,"t_ssao_accumulator") && !contains(code,size,"t_probe_cubemap_diffuse"))?5:contains(code,size,"skylight_parameters0")?6:
        contains(code,size,"t_frame_luminance")?0:contains(code,size,"sky_clouds_parameters0")?1:
        contains(code,size,"t_probe_cubemap_diffuse")?2:contains(code,size,"t_position")?3:4;
    unsigned probe_variant=0;
    unsigned material_variant=0;
    if (kind==2) {
        if (probe_request_id!=camera.id) { probe_request_id=camera.id; probe_shader_count=0; }
        if (size<20 || probe_shader_count>=16) { free(code); return; }
        for (unsigned i=0;i<probe_shader_count;++i)
            if (!memcmp(probe_shader_digests[i],(char *)code+4,16)) { free(code); return; }
        probe_variant=probe_shader_count;
    } else if (kind==8) {
        if (material_request_id!=camera.id) { material_request_id=camera.id; material_shader_count=0; }
        if (size<20 || material_shader_count>=128) { free(code); return; }
        for (unsigned i=0;i<material_shader_count;++i)
            if (!memcmp(material_shader_digests[i],(char *)code+4,16)) { free(code); return; }
        material_variant=material_shader_count;
    } else if (draw_ids[kind]==camera.id || (kind==0 && buffers_id!=camera.id) ||
        (kind==5 && ambient_id!=camera.id)) { free(code); return; }
    if (kind==2 && !probe_variant) capture_probe_initial(context,camera.output);
    char base[MAX_PATH+48],path[MAX_PATH+80];
    const char *labels[]={"post-draw","cloud-draw","probe-draw","fog-draw","sky-draw","hba-draw","skylight-draw","composition-draw","material-draw"};
    if (kind==8)
        snprintf(base,sizeof(base),"%s.material-draw-%u",camera.output,material_variant);
    else if (kind==2 && probe_variant)
        snprintf(base,sizeof(base),"%s.probe-draw-%u",camera.output,probe_variant);
    else snprintf(base,sizeof(base),"%s.%s",camera.output,labels[kind]);
    if (kind==2) memcpy(probe_shader_digests[probe_shader_count++],(char *)code+4,16);
    if (kind==8) memcpy(material_shader_digests[material_shader_count++],(char *)code+4,16);
    snprintf(path,sizeof(path),"%s.dxbc",base);
    int ok=SUCCEEDED(hr) && write_bytes(path,code,size); free(code);
    if (!ok) { error("cannot save active post-process shader"); return; }
    ID3D11BlendState *blend=NULL; float factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    if (blend) {
        D3D11_BLEND_DESC desc; ID3D11BlendState_GetDesc(blend,&desc); ID3D11BlendState_Release(blend);
        snprintf(path,sizeof(path),"%s.blend.json",base); FILE *file=fopen(path,"w");
        if (file) { fprintf(file,"{\"enabled\":%d,\"source\":%u,\"destination\":%u,\"operation\":%u}\n",desc.RenderTarget[0].BlendEnable,desc.RenderTarget[0].SrcBlend,desc.RenderTarget[0].DestBlend,desc.RenderTarget[0].BlendOp); fclose(file); }
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    ID3D11Buffer *buffers[4]={0};
    ID3D11DeviceContext_PSGetConstantBuffers(context,0,4,buffers);
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
                snprintf(path,sizeof(path),"%s.cb%u.bin",base,i);
                if (!write_bytes(path,mapped.pData,desc.ByteWidth)) error("cannot save post-process constants");
                ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
            }
            ID3D11Buffer_Release(staging);
        }
        ID3D11Buffer_Release(buffers[i]);
    }
    ID3D11Device_Release(device);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *index=fopen(path,"w");
    if (index) {
        ID3D11ShaderResourceView *views[12]={0};
        ID3D11DeviceContext_PSGetShaderResources(context,0,12,views);
        fputs("[",index); unsigned count=0;
        for (unsigned i=0;i<12;++i) if (views[i]) {
            ID3D11Resource *resource=NULL; ID3D11ShaderResourceView_GetResource(views[i],&resource);
            ID3D11Texture2D *texture=NULL;
            hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
            if (SUCCEEDED(hr)) {
                D3D11_SHADER_RESOURCE_VIEW_DESC view_desc;
                ID3D11ShaderResourceView_GetDesc(views[i],&view_desc);
                char name[48]; snprintf(name,sizeof(name),"shader_input_%u_view_format_%u",i,view_desc.Format);
                if (count++) fputs(",",index);
                if (kind==8) {
                    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc);
                    fprintf(index,"{\"slot\":%u,\"name\":\"%s\",\"width\":%u,\"height\":%u,\"format\":%u,\"array_size\":%u,\"most_detailed_mip\":%u,\"skipped\":\"material metadata only\"}",i,name,desc.Width,desc.Height,desc.Format,desc.ArraySize,view_desc.Texture2D.MostDetailedMip);
                } else dump_texture(texture,base,index,name,i,&view_desc);
                ID3D11Texture2D_Release(texture);
            }
            ID3D11Resource_Release(resource); ID3D11ShaderResourceView_Release(views[i]);
        }
        fputs("]\n",index); fclose(index);
    }
    EnterCriticalSection(&lock); draw_ids[kind]=camera.id; LeaveCriticalSection(&lock);
}

typedef struct {
    ID3D11ShaderResourceView *view;
    ID3D11BlendState *blend;
    float factor[4];
    UINT mask;
    int bound,blend_bound;
} LegacyExtraState;
static void bind_legacy_extra(ID3D11DeviceContext *context,LegacyExtraState *state) {
    memset(state,0,sizeof(*state));
    if (!legacy_fog_code && !legacy_ambient_decode_code) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&legacy_fog_guid,&size,&flag);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || !flag) return;
    ID3D11ShaderResourceView *view=flag==1?legacy_mie_view:flag==2?legacy_ambient_view:NULL;
    if (!view) { error("legacy draw extra resource missing"); return; }
    if (flag==2) {
        if (!legacy_decode_blend) {
            ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
            D3D11_BLEND_DESC desc={0};
            desc.RenderTarget[0].SrcBlend=D3D11_BLEND_ONE;
            desc.RenderTarget[0].DestBlend=D3D11_BLEND_ZERO;
            desc.RenderTarget[0].BlendOp=D3D11_BLEND_OP_ADD;
            desc.RenderTarget[0].SrcBlendAlpha=D3D11_BLEND_ONE;
            desc.RenderTarget[0].DestBlendAlpha=D3D11_BLEND_ZERO;
            desc.RenderTarget[0].BlendOpAlpha=D3D11_BLEND_OP_ADD;
            desc.RenderTarget[0].RenderTargetWriteMask=D3D11_COLOR_WRITE_ENABLE_ALL;
            hr=ID3D11Device_CreateBlendState(device,&desc,&legacy_decode_blend);
            ID3D11Device_Release(device);
            if (FAILED(hr)) { error("cannot create ambient decode blend state"); return; }
        }
        ID3D11DeviceContext_OMGetBlendState(context,&state->blend,state->factor,&state->mask);
        ID3D11DeviceContext_OMSetBlendState(context,legacy_decode_blend,state->factor,state->mask);
        state->blend_bound=1;
    }
    ID3D11DeviceContext_PSGetShaderResources(context,2,1,&state->view);
    ID3D11DeviceContext_PSSetShaderResources(context,2,1,&view); state->bound=1;
}
static void restore_extra(ID3D11DeviceContext *context,LegacyExtraState *state) {
    if (state->bound) {
        ID3D11DeviceContext_PSSetShaderResources(context,2,1,&state->view);
        if (state->view) ID3D11ShaderResourceView_Release(state->view);
    }
    if (state->blend_bound) {
        ID3D11DeviceContext_OMSetBlendState(context,state->blend,state->factor,state->mask);
        if (state->blend) ID3D11BlendState_Release(state->blend);
    }
}
static void capture_composition_outputs(ID3D11DeviceContext *context,UINT count,UINT start,INT base_vertex,int indexed) {
    if (!native_composition_shader) return;
    Camera camera;
    EnterCriticalSection(&lock);
    camera=applied;
    int take=camera.enabled && captured_id!=camera.id && composition_output_id!=camera.id && draw_ids[7]==camera.id;
    LeaveCriticalSection(&lock);
    if (!take) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&composition_guid,&size,&flag);
    if (FAILED(hr) || !flag) { ID3D11PixelShader_Release(shader); return; }
    // Composition overwrites RT0. Refuse to replay if another variant blends.
    ID3D11BlendState *blend=NULL; float factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    if (blend) {
        D3D11_BLEND_DESC desc; ID3D11BlendState_GetDesc(blend,&desc); ID3D11BlendState_Release(blend);
        if (desc.RenderTarget[0].BlendEnable) { ID3D11PixelShader_Release(shader); error("composition reference requires overwrite blend"); return; }
    }
    ID3D11RenderTargetView *view=NULL; ID3D11DeviceContext_OMGetRenderTargets(context,1,&view,NULL);
    if (!view) { ID3D11PixelShader_Release(shader); return; }
    ID3D11Resource *resource=NULL; ID3D11RenderTargetView_GetResource(view,&resource);
    ID3D11RenderTargetView_Release(view);
    ID3D11Texture2D *texture=NULL;
    hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource);
    if (FAILED(hr)) { ID3D11PixelShader_Release(shader); return; }
    char base[MAX_PATH+48],path[MAX_PATH+80];
    snprintf(base,sizeof(base),"%s.composition-output",camera.output);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("[",file); dump_texture(texture,base,file,"override",0,NULL); fputs(",",file);
        ID3D11DeviceContext_PSSetShader(context,native_composition_shader,NULL,0);
        if (indexed) original_draw_indexed(context,count,start,base_vertex); else original_draw(context,count,start);
        dump_texture(texture,base,file,"native_same_inputs",1,NULL); fputs("]\n",file); fclose(file);
        ID3D11DeviceContext_PSSetShader(context,shader,NULL,0);
        if (indexed) original_draw_indexed(context,count,start,base_vertex); else original_draw(context,count,start);
        composition_output_id=camera.id;
    }
    ID3D11Texture2D_Release(texture); ID3D11PixelShader_Release(shader);
}
static void STDMETHODCALLTYPE draw(ID3D11DeviceContext *context,UINT count,UINT start) {
    LegacyExtraState state; bind_legacy_extra(context,&state);
    capture_post_draw(context); original_draw(context,count,start);
    capture_composition_outputs(context,count,start,0,0); restore_extra(context,&state);
}
static void STDMETHODCALLTYPE draw_indexed(ID3D11DeviceContext *context,UINT count,UINT start,INT base) {
    LegacyExtraState state; bind_legacy_extra(context,&state);
    capture_post_draw(context); original_draw_indexed(context,count,start,base);
    capture_composition_outputs(context,count,start,base,1); restore_extra(context,&state);
}
static int replace_method(void *object,size_t offset,void *hook,void **original) {
    void **method=(void **)((char *)*(void **)object+offset); DWORD protection;
    if (!VirtualProtect(method,sizeof(void *),PAGE_READWRITE,&protection)) return 0;
    *original=*method; *method=hook; VirtualProtect(method,sizeof(void *),protection,&protection);
    return 1;
}
static int load_shader_override(const char *variable,void **code,size_t *size) {
    char path[MAX_PATH];
    if (GetEnvironmentVariableA(variable,path,sizeof(path))) {
        FILE *file=fopen(path,"rb");
        if (!file) { error("legacy atmosphere shader missing"); return 0; }
        fseek(file,0,SEEK_END); long length=ftell(file); rewind(file);
        if (length<32 || length>1024*1024) { fclose(file); return 0; }
        *code=malloc((size_t)length); *size=(size_t)length;
        int ok=*code && fread(*code,1,*size,file)==*size;
        fclose(file);
        if (!ok || memcmp(*code,"DXBC",4) || *(uint32_t *)((char *)*code+24)!=*size) {
            error("invalid legacy atmosphere shader"); return 0;
        }
    }
    return 1;
}
static int install_shader_capture(ID3D11Device *device,ID3D11DeviceContext *context) {
    char enabled[8];
    if (!GetEnvironmentVariableA("SRP_CAPTURE_BUFFERS",enabled,sizeof(enabled)) || strcmp(enabled,"1")) return 1;
    capture_materials_enabled=GetEnvironmentVariableA("SRP_CAPTURE_MATERIALS",enabled,sizeof(enabled)) && !strcmp(enabled,"1");
    if (!load_shader_override("SRP_LEGACY_ATMOSPHERE_LUT",&legacy_lut_code,&legacy_lut_size) ||
        !load_shader_override("SRP_LEGACY_GEOMETRY_FOG",&legacy_fog_code,&legacy_fog_size) ||
        !load_shader_override("SRP_LEGACY_SKY",&legacy_sky_code,&legacy_sky_size) ||
        !load_shader_override("SRP_LEGACY_RAIN_CLOUDS",&legacy_cloud_code,&legacy_cloud_size) ||
        !load_shader_override("SRP_LEGACY_RAIN_TONEMAP",&legacy_post_code,&legacy_post_size) ||
        !load_shader_override("SRP_LEGACY_PROBE_DIFFUSE",&legacy_probe_code,&legacy_probe_size) ||
        !load_shader_override("SRP_LEGACY_PROBE_DIFFUSE_BOX",&legacy_probe_box_code,&legacy_probe_box_size) ||
        !load_shader_override("SRP_LEGACY_COMPOSITION",&legacy_composition_code,&legacy_composition_size) ||
        !load_shader_override("SRP_LEGACY_AMBIENT_DECODE",&legacy_ambient_decode_code,&legacy_ambient_decode_size)) return 0;
    int encoded_sphere=legacy_probe_code && contains(legacy_probe_code,legacy_probe_size,"encoded_color_parameters");
    int encoded_box=legacy_probe_box_code && contains(legacy_probe_box_code,legacy_probe_box_size,"encoded_color_parameters");
    if ((legacy_ambient_decode_code && (!encoded_sphere || !encoded_box)) ||
        ((encoded_sphere || encoded_box) && !legacy_ambient_decode_code)) {
        error("encoded probes require paired sphere, box and ambient decoder"); return 0;
    }
    legacy_ambient_decode_enabled=legacy_ambient_decode_code!=NULL;
    legacy_fog_enabled=legacy_fog_code!=NULL;
    return replace_method(device,offsetof(ID3D11DeviceVtbl,CreatePixelShader),create_ps,(void **)&original_create_ps) &&
        replace_method(context,offsetof(ID3D11DeviceContextVtbl,Draw),draw,(void **)&original_draw) &&
        replace_method(context,offsetof(ID3D11DeviceContextVtbl,DrawIndexed),draw_indexed,(void **)&original_draw_indexed);
}
