// Original 020 uploads (cos(angle),sin(angle),speed,radius). The pinned 026
// cloud and stratosphere branches reverse the first two values while retaining
// the same vertex-shader rotation math. Swap their trig calls, not map angles.
static int install_factory_020e_clouds(void) {
    char enabled[8], shader[MAX_PATH], capture[8];
    if (GetEnvironmentVariableA("SRP_LEGACY_020E_CLOUDS",enabled,sizeof(enabled))!=1 || enabled[0]!='1') return 1;
    DWORD length=GetEnvironmentVariableA("SRP_LEGACY_RAIN_CLOUDS",shader,sizeof(shader));
    if (strcmp(build,"026g") || !length || length>=sizeof(shader) ||
        GetFileAttributesA(shader)==INVALID_FILE_ATTRIBUTES ||
        GetEnvironmentVariableA("SRP_CAPTURE_BUFFERS",capture,sizeof(capture))!=1 || capture[0]!='1') {
        error("020e cloud coordinates require pinned 026 and paired cloud shader"); return 0;
    }
    unsigned char *base=(unsigned char *)GetModuleHandleW(NULL);
    struct Patch { unsigned rva; unsigned char before[5],after[5]; } patches[]={
        {0x194d3e,{0xe8,0xbd,0x42,0xe9,0xff},{0xe8,0xad,0x42,0xe9,0xff}},
        {0x194d4e,{0xe8,0x9d,0x42,0xe9,0xff},{0xe8,0xad,0x42,0xe9,0xff}},
        {0x194feb,{0xe8,0x10,0x40,0xe9,0xff},{0xe8,0x00,0x40,0xe9,0xff}},
        {0x194ffb,{0xe8,0xf0,0x3f,0xe9,0xff},{0xe8,0x00,0x40,0xe9,0xff}},
    };
    for (unsigned i=0;i<sizeof(patches)/sizeof(patches[0]);++i)
        if (memcmp(base+patches[i].rva,patches[i].before,5)) {
            error("020e cloud trig call bytes changed"); return 0;
        }
    for (unsigned i=0;i<sizeof(patches)/sizeof(patches[0]);++i) {
        struct Patch *p=&patches[i]; DWORD protection,ignored;
        if (!VirtualProtect(base+p->rva,5,PAGE_EXECUTE_READWRITE,&protection)) {
            error("cannot patch 020e cloud coordinates"); return 0;
        }
        memcpy(base+p->rva,p->after,5);
        FlushInstructionCache(GetCurrentProcess(),base+p->rva,5);
        if (!VirtualProtect(base+p->rva,5,protection,&ignored)) {
            error("cannot protect 020e cloud coordinates"); return 0;
        }
    }
    char path[MAX_PATH+40];snprintf(path,sizeof(path),"%s.clouds-020e.json",control);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("{\"build\":\"026g\",\"source\":\"020e\",\"rotation\":\"cos,sin\",\"authored_angles_and_speed\":true,\"rvas\":[\"194d3e\",\"194d4e\",\"194feb\",\"194ffb\"]}\n",file);
        fclose(file);
    }
    return 1;
}
