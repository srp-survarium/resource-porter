// 020e ambient lights modulate through one RGBA8 UNORM target.
// The paired exact source apply shader broadcasts red to diffuse and specular.
// Native 026 partitions intensity >1 into an additional diffuse-light pass.
// Preserve the original full-intensity modulation, including saturation, rather
// than compensating individual colors. Only the pinned renderer is supported.
static int factory_020e_ambient_enabled;
static int install_factory_020e_ambient(void) {
    char enabled[8];
    if (GetEnvironmentVariableA("SRP_LEGACY_020E_AMBIENT",enabled,sizeof(enabled))!=1 || enabled[0]!='1') return 1;
    if (strcmp(build,"026g")) { error("020e ambient adapter requires pinned 026"); return 0; }
    char apply[MAX_PATH], capture[8];
    DWORD length = GetEnvironmentVariableA("SRP_LEGACY_020E_AMBIENT_APPLY",apply,sizeof(apply));
    if (!length || length >= sizeof(apply) || GetFileAttributesA(apply)==INVALID_FILE_ATTRIBUTES ||
        GetEnvironmentVariableA("SRP_CAPTURE_BUFFERS",capture,sizeof(capture))!=1 || capture[0]!='1') {
        error("020e ambient requires exact apply shader and shader hook enabled"); return 0;
    }
    factory_020e_ambient_enabled=1;
    unsigned char *base=(unsigned char *)GetModuleHandleW(NULL);
    struct Patch { unsigned rva,size; unsigned char before[5],after[5]; } patches[]={
        // First remove_if receives EAX=begin, ECX=end, selector0 on stack.
        // Return end unchanged: retain every ambient light in modulation group.
        {0x171f28,5,{0xe8,0xb3,0x39,0,0},{0x8b,0xc1,0x90,0x90,0x90}},
        // Second remove_if receives begin in EAX. Keep it: empty additive group.
        {0x171f5a,5,{0xe8,0x81,0x39,0,0},{0x90,0x90,0x90,0x90,0x90}},
        // Worker must receive original intensity, not intensity minus one.
        {0x1716bc,4,{0xf3,0x0f,0x5c,0xc1,0},{0x90,0x90,0x90,0x90,0}},
    };
    // Adjacent argument setup confirms register/stack contracts at call sites.
    const unsigned char first_setup[]={0x89,0x4c,0x24,0x10,0x33,0xdb};
    const unsigned char second_setup[]={0x8b,0x44,0x24,0x14,0x89,0x4c,0x24,0x0c};
    if (memcmp(base+0x171f22,first_setup,sizeof(first_setup)) ||
        memcmp(base+0x171f52,second_setup,sizeof(second_setup))) {
        error("020e ambient group argument bytes changed"); return 0;
    }
    for (unsigned i=0;i<sizeof(patches)/sizeof(patches[0]);++i)
        if (memcmp(base+patches[i].rva,patches[i].before,patches[i].size)) {
            error("020e ambient stage bytes changed"); return 0;
        }
    for (unsigned i=0;i<sizeof(patches)/sizeof(patches[0]);++i) {
        struct Patch *p=&patches[i]; DWORD protection,ignored;
        if (!VirtualProtect(base+p->rva,p->size,PAGE_EXECUTE_READWRITE,&protection)) {
            error("cannot patch 020e ambient stage"); return 0;
        }
        memcpy(base+p->rva,p->after,p->size);
        FlushInstructionCache(GetCurrentProcess(),base+p->rva,p->size);
        if (!VirtualProtect(base+p->rva,p->size,protection,&ignored)) {
            error("cannot protect 020e ambient stage"); return 0;
        }
    }
    char path[MAX_PATH+40];snprintf(path,sizeof(path),"%s.ambient-020e.json",control);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("{\"build\":\"026g\",\"source\":\"020e\",\"modulation\":\"all visible ambient lights with full intensity\",\"additive_group\":\"empty\",\"rvas\":[\"171f28\",\"171f5a\",\"1716bc\"]}\n",file);
        fclose(file);
    }
    return 1;
}
