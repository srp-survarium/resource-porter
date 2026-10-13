// Private capture-only workaround for the verified 0.26 inactive-window gate.
// This changes neither activity flags nor shader/render settings. Default off.
static int capture_background_enabled, capture_background_owned;
static unsigned char capture_background_original;
static unsigned capture_background_writes;
static DWORD capture_background_vtable,capture_background_state,capture_background_protect,capture_background_type;
static SIZE_T capture_background_query;
static unsigned capture_background_rejection;
static char capture_background_switch[MAX_PATH+32];

static int capture_background_read(uintptr_t address,void *out,SIZE_T size) {
    SIZE_T count=0;
    return ReadProcessMemory(GetCurrentProcess(),(const void *)address,out,size,&count) && count==size;
}

static int capture_background_guard(unsigned char *gate) {
    MEMORY_BASIC_INFORMATION memory;memset(&memory,0,sizeof(memory));
    int table_ok=capture_background_read(0xa7b6b8,&capture_background_vtable,sizeof(capture_background_vtable));
    capture_background_query=VirtualQuery((const void *)0xa7b9cb,&memory,sizeof(memory));
    capture_background_state=memory.State;capture_background_protect=memory.Protect;capture_background_type=memory.Type;
    int gate_ok=capture_background_read(0xa7b9cb,gate,1);
    capture_background_rejection=!table_ok?1:capture_background_vtable!=0x9db60c?2:
        capture_background_query!=sizeof(memory)?3:memory.State!=MEM_COMMIT?4:
        !(memory.Protect==PAGE_READWRITE || (memory.Protect==PAGE_WRITECOPY &&
            memory.Type==MEM_IMAGE && (uintptr_t)memory.AllocationBase==0x10000))?5:!gate_ok?6:*gate>1?7:0;
    return !capture_background_rejection;
}

static void capture_background_record(const char *event,unsigned char gate) {
    char path[MAX_PATH+40];snprintf(path,sizeof(path),"%s.background-render.json",control);
    FILE *file=fopen(path,"w");if (!file) return;
    fprintf(file,"{\"event\":\"%s\",\"enabled\":%s,\"owned\":%s,\"original_gate\":%u,\"current_gate\":%u,\"writes\":%u,\"guard_rejection\":%u,\"vtable\":%lu,\"query_bytes\":%lu,\"page_state\":%lu,\"page_protect\":%lu,\"page_type\":%lu}\n",
        event,capture_background_enabled?"true":"false",capture_background_owned?"true":"false",
        capture_background_original,gate,capture_background_writes,capture_background_rejection,
        capture_background_vtable,(unsigned long)capture_background_query,capture_background_state,capture_background_protect,capture_background_type);
    fclose(file);
}

static void capture_background_restore(void) {
    unsigned char gate=0;
    if (!capture_background_owned || !capture_background_guard(&gate)) return;
    if (!gate) {
        *(volatile unsigned char *)(uintptr_t)0xa7b9cb=capture_background_original;
        ++capture_background_writes;
    }
    capture_background_owned=0;
}

static int configure_capture_background(void) {
    char option[8];
    DWORD length=GetEnvironmentVariableA("SRP_CAPTURE_BACKGROUND_RENDER",option,sizeof(option));
    capture_background_enabled=length==1 && option[0]=='1';
    if (!capture_background_enabled) return 1;
    const unsigned char expected_gate[]={0x80,0xbb,0x13,0x03,0x00,0x00,0x00,0x74,0x0f};
    const unsigned char expected_activity[]={0x80,0xb9,0x0a,0x03,0x00,0x00,0x00,0xb0,0x01,0x75,0x06,0x8a,0x81,0x09,0x03,0x00,0x00,0xc3};
    unsigned char gate_code[sizeof(expected_gate)],activity_code[sizeof(expected_activity)];
    if (strcmp(build,"026g") || (uintptr_t)GetModuleHandleA(NULL)!=0x10000 ||
        !capture_background_read(0x1a0d8,gate_code,sizeof(gate_code)) || memcmp(gate_code,expected_gate,sizeof(gate_code)) ||
        !capture_background_read(0x14000,activity_code,sizeof(activity_code)) || memcmp(activity_code,expected_activity,sizeof(activity_code))) {
        error("background capture requires the verified 0.26 activity gate"); return 0;
    }
    snprintf(capture_background_switch,sizeof(capture_background_switch),"%s.background-render",control);
    capture_background_record("configured",0);
    return 1;
}

static void capture_background_step(void) {
    if (!capture_background_enabled) return;
    int enabled=1;
    FILE *file=fopen(capture_background_switch,"r");
    if (file) { int option=fgetc(file); fclose(file); if (option=='0') enabled=0; }
    unsigned char gate=0;
    if (!capture_background_guard(&gate)) {
        static unsigned last_rejection;static DWORD last_vtable,last_protect;static unsigned char last_gate;
        if (last_rejection!=capture_background_rejection || last_vtable!=capture_background_vtable || last_protect!=capture_background_protect || last_gate!=gate)
            capture_background_record("guard-rejected",gate);
        last_rejection=capture_background_rejection;last_vtable=capture_background_vtable;last_protect=capture_background_protect;last_gate=gate;
        return; // Engine may not be constructed yet; rejected pages are never changed.
    }
    if (!enabled) {
        if (capture_background_owned) {
            capture_background_restore();
            capture_background_read(0xa7b9cb,&gate,1);
            capture_background_record("restored",gate);
        }
        return;
    }
    // Wait for the native show-window path to enable its gate before taking ownership.
    if (gate) {
        if (!capture_background_owned) capture_background_original=gate;
        *(volatile unsigned char *)(uintptr_t)0xa7b9cb=0;
        capture_background_owned=1; ++capture_background_writes;
        capture_background_record("applied",0);
    }
}
