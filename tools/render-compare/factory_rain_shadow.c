// Restore the 0.10b Factory rain camera before native caster rasterization.
// Source stage_pre_rain uses position + full_view_direction * 32, a 64-unit
// orthographic span and the same 128-square depth target/far-plane formula.
static const float factory_shadow_one=1.0f, factory_shadow_half=.5f;
static int install_factory_rain_shadow(void) {
    char option[8];
    if (GetEnvironmentVariableA("SRP_LEGACY_FACTORY_RAIN_SHADOW",option,sizeof(option))!=1 || option[0]!='1') return 1;
    unsigned char *base=(unsigned char *)GetModuleHandleW(NULL);
    static const unsigned char forward[]={0xf3,0x0f,0x10,0x05,0xd4,0x6f,0xac,0x00};
    static const unsigned char vertical[]={0xf3,0x0f,0x59,0xd5,0xf3,0x0f,0x59,0xd0};
    static const unsigned char projection[]={0xd9,0x44,0x24,0x14,0xf3,0x0f,0x10,0x44,0x24,0x14,0xd9,0x14,0x24};
    static const unsigned char call[]={0xe8,0x86,0x2a,0xf7,0xff};
    unsigned char *sites[]={base+0x16c390,base+0x16c3c9,base+0x16c485};
    const unsigned char *expected[]={forward,vertical,projection};
    const SIZE_T sizes[]={sizeof(forward),sizeof(vertical),sizeof(projection)};
    if (strcmp(build,"026g") || (uintptr_t)base!=0x10000 || memcmp(base+0x16c4b5,call,sizeof(call))) {
        error("Factory rain shadow adapter requires pinned 0.26 projection"); return 0;
    }
    for (unsigned i=0;i<3;++i) if (memcmp(sites[i],expected[i],sizes[i])) {
        error("Factory rain shadow instructions differ from pinned build"); return 0;
    }
    unsigned char *code=VirtualAlloc(NULL,64,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE);
    if (!code) { error("cannot allocate Factory rain shadow adapter"); return 0; }
    // At this point [esp+2c] is the saved full camera forward Y; native uses
    // xmm5=zero instead. Preserve xmm5 because later code needs that zero.
    const unsigned char ycode[]={0xf3,0x0f,0x59,0x54,0x24,0x2c,0xf3,0x0f,0x59,0xd0,0xe9};
    memcpy(code,ycode,sizeof(ycode));
    int32_t back=(int32_t)((intptr_t)(sites[1]+sizes[1])-(intptr_t)(code+15));
    memcpy(code+11,&back,4);
    unsigned char *width=code+32;
    memcpy(width,projection,4); // fld [esp+14], shadow target size.
    width[4]=0xd8; width[5]=0x0d; // fmul [half], only the projection dimensions.
    uint32_t half=(uint32_t)(uintptr_t)&factory_shadow_half;
    memcpy(width+6,&half,4);
    memcpy(width+10,projection+4,sizeof(projection)-4); // Original far-plane input stays unchanged.
    width[19]=0xe9;
    back=(int32_t)((intptr_t)(sites[2]+sizes[2])-(intptr_t)(width+24));
    memcpy(width+20,&back,4);
    DWORD protection[3],unused;
    for (unsigned i=0;i<3;++i) if (!VirtualProtect(sites[i],sizes[i],PAGE_EXECUTE_READWRITE,&protection[i])) {
        for (unsigned j=0;j<i;++j) VirtualProtect(sites[j],sizes[j],protection[j],&unused);
        VirtualFree(code,0,MEM_RELEASE);
        error("cannot write Factory rain shadow adapter"); return 0;
    }
    uint32_t one=(uint32_t)(uintptr_t)&factory_shadow_one;
    memcpy(sites[0]+4,&one,4); // Remove native's extra .5 camera-offset multiplier.
    for (unsigned i=1;i<3;++i) {
        unsigned char *destination=i==1?code:width;
        sites[i][0]=0xe9;
        int32_t jump=(int32_t)((intptr_t)destination-(intptr_t)(sites[i]+5));
        memcpy(sites[i]+1,&jump,4); memset(sites[i]+5,0x90,sizes[i]-5);
    }
    FlushInstructionCache(GetCurrentProcess(),code,64);
    int protected=1;
    for (unsigned i=0;i<3;++i) {
        FlushInstructionCache(GetCurrentProcess(),sites[i],sizes[i]);
        if (!VirtualProtect(sites[i],sizes[i],protection[i],&unused)) protected=0;
    }
    if (!protected) { error("cannot protect Factory rain shadow adapter"); return 0; }
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.factory-rain-shadow.json",control);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("{\"build\":\"026g\",\"scope\":\"Factory010 rain\",\"shadow_texels\":128,\"world_span\":64,\"camera_forward_scale\":32,\"full_vertical_direction\":true,\"far_plane_unchanged\":true,\"sites\":[\"0x17c390\",\"0x17c3c9\",\"0x17c485\"]}\n",file);
        fclose(file);
    }
    return 1;
}
