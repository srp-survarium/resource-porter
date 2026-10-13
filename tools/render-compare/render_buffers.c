// Included by camera_hook.c: v0.26 stage entry verified against 0.21 symbols.
static unsigned ambient_id, ambient_after_id;
static ID3D11ShaderResourceView *legacy_mie_view;
static ID3D11Texture2D *legacy_mie_texture;
static int legacy_fog_enabled;
static int legacy_ambient_decode_enabled;
static ID3D11ShaderResourceView *legacy_ambient_view;
static ID3D11Texture2D *legacy_ambient_copy, *legacy_ambient_source;
static int legacy_ssao_enabled, legacy_ssao_ready;
static ID3D11ShaderResourceView *legacy_ssao_view;
static void run_legacy_ssao(unsigned char *targets,const Camera *camera,int take);
void *post_trampoline, *ambient_trampoline, *ambient_after_trampoline;

static int readable(const void *pointer, size_t bytes) {
    MEMORY_BASIC_INFORMATION info;
    if (!pointer || !VirtualQuery(pointer,&info,sizeof(info)) || info.State!=MEM_COMMIT ||
        (info.Protect&(PAGE_NOACCESS|PAGE_GUARD))) return 0;
    return (uintptr_t)pointer+bytes <= (uintptr_t)info.BaseAddress+info.RegionSize;
}

static void copy_encoded_ambient(unsigned char *targets) {
    if (!readable(targets,32*0xa0)) { error("ambient target array missing"); return; }
    unsigned char *target=*(void **)(targets+31*0xa0+0x98);
    if (!readable(target,0x10)) { error("ambient target missing"); return; }
    ID3D11Texture2D *texture=*(void **)(target+0xc);
    if (!texture) { error("ambient texture missing"); return; }
    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc);
    if (desc.Format!=DXGI_FORMAT_R11G11B10_FLOAT || desc.SampleDesc.Count!=1 || desc.ArraySize!=1) {
        error("ambient texture contract changed"); return;
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext *context=NULL;
    ID3D11Texture2D_GetDevice(texture,&device); ID3D11Device_GetImmediateContext(device,&context);
    if (texture!=legacy_ambient_source) {
        if (legacy_ambient_view) ID3D11ShaderResourceView_Release(legacy_ambient_view);
        if (legacy_ambient_copy) ID3D11Texture2D_Release(legacy_ambient_copy);
        legacy_ambient_view=NULL; legacy_ambient_copy=NULL; legacy_ambient_source=NULL;
        desc.Usage=D3D11_USAGE_DEFAULT; desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
        desc.CPUAccessFlags=0; desc.MiscFlags=0; desc.MipLevels=1;
        if (SUCCEEDED(ID3D11Device_CreateTexture2D(device,&desc,NULL,&legacy_ambient_copy)) &&
            SUCCEEDED(ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)legacy_ambient_copy,NULL,&legacy_ambient_view)))
            legacy_ambient_source=texture;
        else error("cannot allocate encoded ambient copy");
    }
    if (legacy_ambient_view)
        ID3D11DeviceContext_CopySubresourceRegion(context,(ID3D11Resource *)legacy_ambient_copy,0,0,0,0,(ID3D11Resource *)texture,0,NULL);
    ID3D11DeviceContext_Release(context); ID3D11Device_Release(device);
}

static int dump_texture(ID3D11Texture2D *buffer, const char *base, FILE *index, const char *name, unsigned slot,const D3D11_SHADER_RESOURCE_VIEW_DESC *view) {
    D3D11_TEXTURE2D_DESC desc;
    ID3D11Texture2D_GetDesc(buffer,&desc);
    fprintf(index,"{\"slot\":%u,\"name\":\"%s\",\"width\":%u,\"height\":%u,\"format\":%u,\"array_size\":%u,\"mip_levels\":%u",slot,name,desc.Width,desc.Height,desc.Format,desc.ArraySize,desc.MipLevels);
    if (view) {
        fprintf(index,",\"view_format\":%u,\"view_dimension\":%u",view->Format,view->ViewDimension);
        if (view->ViewDimension==D3D11_SRV_DIMENSION_TEXTURE2D || view->ViewDimension==D3D11_SRV_DIMENSION_TEXTURE2DARRAY ||
            view->ViewDimension==D3D11_SRV_DIMENSION_TEXTURECUBE || view->ViewDimension==D3D11_SRV_DIMENSION_TEXTURECUBEARRAY)
            fprintf(index,",\"view_most_detailed_mip\":%u,\"view_mip_levels\":%u",view->Texture2D.MostDetailedMip,view->Texture2D.MipLevels);
    }
    unsigned bytes, block=1;
    switch (desc.Format) {
        case DXGI_FORMAT_R32G32B32A32_FLOAT: bytes=16; break;
        case DXGI_FORMAT_R16G16B16A16_FLOAT: bytes=8; break;
        case DXGI_FORMAT_R32G32_FLOAT: bytes=8; break;
        case DXGI_FORMAT_R16G16_FLOAT: case DXGI_FORMAT_R32_FLOAT:
        case DXGI_FORMAT_R10G10B10A2_UNORM: case DXGI_FORMAT_R11G11B10_FLOAT: case DXGI_FORMAT_R8G8B8A8_UNORM:
        case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: case DXGI_FORMAT_B8G8R8A8_UNORM:
        case DXGI_FORMAT_B8G8R8A8_TYPELESS: case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: bytes=4; break;
        case DXGI_FORMAT_BC1_TYPELESS: case DXGI_FORMAT_BC1_UNORM: case DXGI_FORMAT_BC1_UNORM_SRGB: bytes=8; block=4; break;
        case DXGI_FORMAT_BC2_TYPELESS: case DXGI_FORMAT_BC2_UNORM: case DXGI_FORMAT_BC2_UNORM_SRGB:
        case DXGI_FORMAT_BC3_TYPELESS: case DXGI_FORMAT_BC3_UNORM: case DXGI_FORMAT_BC3_UNORM_SRGB: bytes=16; block=4; break;
        case DXGI_FORMAT_R16_FLOAT: case DXGI_FORMAT_R16_TYPELESS:
        case DXGI_FORMAT_R16_UNORM: case DXGI_FORMAT_R8G8_UNORM: bytes=2; break;
        case DXGI_FORMAT_R8_UNORM: bytes=1; break;
        default: fprintf(index,",\"skipped\":\"format\"}"); return 0;
    }
    if (desc.SampleDesc.Count!=1 || desc.ArraySize!=1 || desc.Width>4096 || desc.Height>4096) {
        fprintf(index,",\"skipped\":\"dimensions\"}"); return 0;
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext *context=NULL; ID3D11Texture2D *staging=NULL;
    ID3D11Texture2D_GetDevice(buffer,&device); ID3D11Device_GetImmediateContext(device,&context);
    desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    desc.MiscFlags=0; desc.MipLevels=1;
    HRESULT hr=ID3D11Device_CreateTexture2D(device,&desc,NULL,&staging);
    int ok=0;
    if (SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopySubresourceRegion(context,(ID3D11Resource *)staging,0,0,0,0,(ID3D11Resource *)buffer,0,NULL);
        D3D11_MAPPED_SUBRESOURCE mapped;
        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
        if (SUCCEEDED(hr)) {
            char path[MAX_PATH+64]; snprintf(path,sizeof(path),"%s.rt%02u.bin",base,slot);
            FILE *file=fopen(path,"wb");
            if (file) {
                ok=1;
                unsigned rows=(desc.Height+block-1)/block, row_bytes=((desc.Width+block-1)/block)*bytes;
                for (unsigned y=0;y<rows;++y)
                    if (fwrite((char *)mapped.pData+y*mapped.RowPitch,row_bytes,1,file)!=1) ok=0;
                if (fclose(file)) ok=0;
            }
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        }
    }
    if (block==1) fprintf(index,",\"bytes_per_pixel\":%u",bytes);
    else fprintf(index,",\"block_width\":4,\"block_height\":4,\"bytes_per_block\":%u",bytes);
    fprintf(index,",\"written\":%s,\"hresult\":%ld}",ok?"true":"false",hr);
    if (staging) ID3D11Texture2D_Release(staging);
    ID3D11DeviceContext_Release(context); ID3D11Device_Release(device);
    return ok;
}

static void capture_stage_inputs(unsigned char *stage, unsigned *last_id, const char *suffix) {
    Camera camera;
    EnterCriticalSection(&lock);
    int take=applied.enabled && capture_frame_id==applied.id && applied.id!=*last_id && GetTickCount()-applied_at>=applied.settle_ms;
    if (last_id==&ambient_after_id && ambient_id!=applied.id) take=0;
    if (last_id==&buffers_id && ambient_after_id!=applied.id) take=0;
    camera=applied;
    LeaveCriticalSection(&lock);
    if (!take && !legacy_fog_enabled && !legacy_ambient_decode_enabled) return;
    if (!readable(stage,8)) return;
    unsigned char *context=*(void **)(stage+4);
    if (!readable(context,0x11a0)) return;
    unsigned char *targets=*(void **)(context+0x119c);
    if (legacy_ambient_decode_enabled && last_id==&ambient_id) {
        copy_encoded_ambient(targets);
        run_legacy_ssao(targets,&camera,take);
    }
    if (legacy_fog_enabled && readable(targets,72*0xa0)) {
        unsigned char *target=*(void **)(targets+71*0xa0+0x98);
        if (readable(target,0x10)) {
            ID3D11Texture2D *texture=*(void **)(target+0xc);
            if (texture && texture!=legacy_mie_texture) {
                ID3D11Device *device=NULL; ID3D11Texture2D_GetDevice(texture,&device);
                if (legacy_mie_view) ID3D11ShaderResourceView_Release(legacy_mie_view);
                legacy_mie_view=NULL; legacy_mie_texture=NULL;
                if (SUCCEEDED(ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)texture,NULL,&legacy_mie_view))) legacy_mie_texture=texture;
                ID3D11Device_Release(device);
            }
        }
    }
    if (!take) return;
    char base[MAX_PATH+32], path[MAX_PATH+64];
    snprintf(base,sizeof(base),"%s%s",camera.output,suffix);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *file=fopen(path,"w"); if (!file) return;
    fputs("[",file); unsigned count=0;
    for (unsigned slot=0;slot<89;++slot) {
        unsigned char *instance=targets+slot*0xa0;
        if (!readable(instance,0xa0)) break;
        const char *name=(char *)instance+0xc;
        size_t length=strnlen(name,64);
        if (!length || length>=64) continue;
        int valid=1;
        for (size_t i=0;i<length;++i) if ((unsigned char)name[i]<32 || name[i]=='"' || name[i]=='\\') valid=0;
        if (!valid) continue;
        // Names are read from the actual target, not assumed enum indices.
        if (!strstr(name,"generic") && !strstr(name,"accum") &&
            !strstr(name,"scattering") && !strstr(name,"frame_luminance") &&
            !strstr(name,"albedo") && !strstr(name,"normal")) {
            if (count++) fputs(",",file);
            fprintf(file,"{\"slot\":%u,\"name\":\"%s\",\"skipped\":\"selection\"}",slot,name);
            continue;
        }
        unsigned char *target=*(void **)(instance+0x98);
        if (!readable(target,0x30)) continue;
        ID3D11Texture2D *texture=*(void **)(target+0xc);
        if (!readable(texture,sizeof(void *))) continue;
        if (count++) fputs(",",file);
        if (*suffix && slot!=31 && slot!=32 && slot!=34) {
            fprintf(file,"{\"slot\":%u,\"name\":\"%s\",\"skipped\":\"stage selection\"}",slot,name);
        } else dump_texture(texture,base,file,name,slot,NULL);
    }
    fputs("]\n",file); fclose(file);
    EnterCriticalSection(&lock); *last_id=camera.id; LeaveCriticalSection(&lock);
}

void __cdecl capture_post_inputs(uint32_t *saved) {
    capture_stage_inputs((void *)(uintptr_t)saved[6],&buffers_id,"");
}

void __cdecl capture_ambient_inputs(uint32_t *saved) {
    // pushfd + pushad: saved ESP points to saved flags; first argument is +8.
    uint32_t *entry_stack=(void *)(uintptr_t)saved[3];
    capture_stage_inputs((void *)(uintptr_t)entry_stack[2],&ambient_id,".before-hba");
}

void __cdecl capture_ambient_outputs(uint32_t *saved) {
    capture_stage_inputs((void *)(uintptr_t)saved[1],&ambient_after_id,".after-hba");
}

__attribute__((naked)) static void ambient_after_entry(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\t"
        "pushl %ebp\n\tcall _capture_ambient_outputs\n\taddl $4, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "jmp *_ambient_after_trampoline\n\t");
}

__attribute__((naked)) static void ambient_entry(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\t"
        "pushl %ebp\n\tcall _capture_ambient_inputs\n\taddl $4, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "jmp *_ambient_trampoline\n\t");
}

__attribute__((naked)) static void post_entry(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\t"
        "pushl %ebp\n\tcall _capture_post_inputs\n\taddl $4, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "jmp *_post_trampoline\n\t");
}

static int install_stage(unsigned rva, void *hook, void **original, const unsigned char *expected) {
    unsigned char *entry=(unsigned char *)GetModuleHandleW(NULL)+rva;
    if (memcmp(entry,expected,6)) { error("render stage entry bytes mismatch"); return 0; }
    unsigned char *trampoline=VirtualAlloc(NULL,32,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE);
    if (!trampoline) return 0;
    memcpy(trampoline,expected,6); trampoline[6]=0xe9;
    *(int32_t *)(trampoline+7)=(int32_t)(entry+6-(trampoline+11)); *original=trampoline;
    DWORD protection;
    if (!VirtualProtect(entry,6,PAGE_EXECUTE_READWRITE,&protection)) return 0;
    entry[0]=0xe9; *(int32_t *)(entry+1)=(int32_t)((unsigned char *)hook-(entry+5)); entry[5]=0x90;
    FlushInstructionCache(GetCurrentProcess(),entry,6); VirtualProtect(entry,6,protection,&protection);
    return 1;
}

static int install_post(void) {
    char enabled[8];
    if (!GetEnvironmentVariableA("SRP_CAPTURE_BUFFERS",enabled,sizeof(enabled)) || strcmp(enabled,"1")) return 1;
    // Original-client capture uses D3D draw identities, without target-layout hooks.
    if (!strcmp(build,"010b") || !strcmp(build,"011a")) { buffers_enabled=1; return 1; }
    if (strcmp(build,"026g")) { error("unsupported buffer capture build"); return 0; }
    buffers_enabled=1;
    const unsigned char entry[]={0x55,0x8b,0xec,0x83,0xe4,0xf8};
    const unsigned char after[]={0x8b,0x35,0x90,0x04,0x26,0x02};
    return install_stage(0x1a2270,post_entry,&post_trampoline,entry) &&
        install_stage(0x172530,ambient_entry,&ambient_trampoline,entry) &&
        install_stage(0x1728bf,ambient_after_entry,&ambient_after_trampoline,after);
}
