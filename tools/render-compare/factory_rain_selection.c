// SPDX-License-Identifier: GPL-3.0-or-later
// Source 0.10b selects rain geometry LODs from the installed shadow camera.
// Native 0.26 instead passes its saved main camera to this one selection call.
static void * __attribute__((used)) rain_selection_resume=(void *)0x17c6f2;
static unsigned rain_selection_logged_id;

static void __attribute__((used)) rain_selection_record(uintptr_t context,const float *main_eye) {
    Camera camera;
    EnterCriticalSection(&lock);
    camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id;
    LeaveCriticalSection(&lock);
    if (!take || rain_selection_logged_id==camera.id) return;
    float main[3],shadow[3]; SIZE_T count;
    if (!ReadProcessMemory(GetCurrentProcess(),main_eye,main,sizeof(main),&count) || count!=sizeof(main) ||
        !ReadProcessMemory(GetCurrentProcess(),(void *)(context+0x54a4),shadow,sizeof(shadow),&count) || count!=sizeof(shadow)) return;
    for (unsigned i=0;i<3;++i) if (!isfinite(main[i]) || !isfinite(shadow[i])) return;
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.rain-selection.json",camera.output);
    FILE *file=fopen(path,"w");
    if (!file) return;
    fprintf(file,"{\"mode\":\"source_shadow_camera_selection\",\"camera_calls\":%u,\"present_calls\":%u,\"native_saved_main_camera\":[%.9g,%.9g,%.9g],\"selected_shadow_camera\":[%.9g,%.9g,%.9g],\"cached_factors_unchanged\":true}\n",
        camera_calls,present_calls,main[0],main[1],main[2],shadow[0],shadow[1],shadow[2]);
    fclose(file); rain_selection_logged_id=camera.id;
}

static void __attribute__((naked,used)) rain_selection_bridge(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmov %esp,%ebp\n\t"
        "sub $528,%esp\n\tand $-16,%esp\n\tfxsave (%esp)\n\t"
        // Original ESP=saved base+36; native main eye is original ESP+0x64.
        "lea 136(%ebp),%eax\n\tpush %eax\n\tpushl 28(%ebp)\n\tcall _rain_selection_record\n\tadd $8,%esp\n\t"
        "fxrstor (%esp)\n\tmov %ebp,%esp\n\tpopal\n\tpopfl\n\t"
        "lea 0x54a4(%eax),%edx\n\tlea 0x41ac(%esp),%ecx\n\tjmp *_rain_selection_resume\n\t");
}

static int install_factory_rain_selection(void) {
    char option[8];
    if (GetEnvironmentVariableA("SRP_DIAGNOSTIC_FACTORY_RAIN_SELECTION",option,sizeof(option))!=1 || option[0]!='1') return 1;
    static const unsigned char selection[]={0x8b,0x45,0x04,0x8d,0x54,0x24,0x64,0x8d,0x8c,0x24,0xac,0x41,0,0,0x52};
    static const unsigned char set_view[]={0xe8,0x80,0xdc,0xfc,0xff};
    static const unsigned char position_write[]={0x66,0x0f,0xd6,0x83,0xa4,0x54,0,0};
    static const unsigned char select_call[]={0xe8,0x4b,0x3e,0xfd,0xff};
    unsigned char *site=(unsigned char *)0x17c6e7;
    if (strcmp(build,"026g") || (uintptr_t)GetModuleHandleA(NULL)!=0x10000 ||
        memcmp((void *)0x17c6e4,selection,sizeof(selection)) ||
        memcmp((void *)0x17c5bb,set_view,sizeof(set_view)) ||
        memcmp((void *)0x14a59d,position_write,sizeof(position_write)) ||
        memcmp((void *)0x17c710,select_call,sizeof(select_call))) {
        error("Factory rain selection diagnostic requires pinned 0.26 layout"); return 0;
    }
    DWORD old,unused;
    if (!VirtualProtect(site,11,PAGE_EXECUTE_READWRITE,&old)) { error("cannot write Factory rain selection diagnostic"); return 0; }
    site[0]=0xe9;
    int32_t jump=(int32_t)((intptr_t)rain_selection_bridge-(intptr_t)(site+5));
    memcpy(site+1,&jump,4); memset(site+5,0x90,6);
    FlushInstructionCache(GetCurrentProcess(),site,11);
    if (!VirtualProtect(site,11,old,&unused)) { error("cannot protect Factory rain selection diagnostic"); return 0; }
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.factory-rain-selection.json",control);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("{\"mode\":\"diagnostic_source_shadow_camera_selection\",\"site\":\"0x17c6e7\",\"shadow_position_offset\":21668,\"cached_factors_unchanged\":true,\"size_predicate_unchanged\":true,\"occlusion_unchanged\":true}\n",file);
        fclose(file);
    }
    return 1;
}
