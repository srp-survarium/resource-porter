/* Optional 0.11a box-probe order adaptation. Only the visible pointer list is
   reordered; probe properties, influence geometry and shader constants stay
   untouched. Original 0.11a set_properties derives radius as .75*max(scale),
   whereas 0.26 retains the authored radius for boxes. */
static void * __attribute__((used)) probe_native_final_sort=(void *)(uintptr_t)0x1b9460;
static unsigned probe_sort_attempts,probe_sort_calls,probe_sort_rejections;
static DWORD probe_sort_begin,probe_sort_end;
static unsigned probe_sort_line,probe_sort_index;
static DWORD probe_sort_last_hash;

typedef struct {
    DWORD pointer;
    float key,native_key,position[3];
    DWORD geometry;
} LegacyProbeOrder;

static int probe_order_read(DWORD address,void *out,SIZE_T size) {
    SIZE_T got=0;
    return ReadProcessMemory(GetCurrentProcess(),(const void *)(uintptr_t)address,out,size,&got)&&got==size;
}

static void __attribute__((used,noinline)) legacy_probe_sort(DWORD *begin,DWORD *end) {
    ++probe_sort_attempts; probe_sort_begin=(DWORD)(uintptr_t)begin; probe_sort_end=(DWORD)(uintptr_t)end;
    uintptr_t bytes=(uintptr_t)end-(uintptr_t)begin;
    if ((uintptr_t)end<(uintptr_t)begin || bytes%4 || bytes>1024*4) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
    unsigned count=(unsigned)(bytes/4);
    if (!count) return;
    DWORD pointers[1024]; LegacyProbeOrder rows[1024];
    if (!probe_order_read((DWORD)(uintptr_t)begin,pointers,bytes)) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
    for (unsigned i=0;i<count;++i) {
        probe_sort_index=i;
        unsigned char object[0x340];
        if (!probe_order_read(pointers[i],object,sizeof(object))) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
        if (probe_sort_attempts==1) {
            char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.probe-first-%u.bin",control,i);
            FILE *file=fopen(path,"wb"); if(file) { fwrite(object,1,sizeof(object),file); fclose(file); }
        }
        LegacyProbeOrder *row=&rows[i]; row->pointer=pointers[i];
        memcpy(&row->native_key,object+0x320,4);
        memcpy(&row->geometry,object+0x33c,4);
        memcpy(row->position,object+0x314,12);
        row->key=row->native_key;
        if (row->geometry>1 || !isfinite(row->key) || row->key<=0) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
        if (row->geometry) {
            float matrix[16],maximum=0;
            memcpy(matrix,object+0x234,sizeof(matrix));
            for (unsigned j=0;j<3;++j) {
                float sum=0;
                for (unsigned k=0;k<3;++k) sum+=matrix[j*4+k]*matrix[j*4+k];
                if (!isfinite(sum) || sum<=0) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
                maximum=fmaxf(maximum,sqrtf(sum));
            }
            row->key=.75f*maximum;
        }
        for (unsigned k=0;k<3;++k) if (!isfinite(row->position[k])) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
    }
    /* Stable insertion sort retains native order for genuinely equal source
       keys. No pointer/address-derived tie-breaker is introduced. */
    for (unsigned i=1;i<count;++i) {
        LegacyProbeOrder value=rows[i]; unsigned j=i;
        while (j && value.key>rows[j-1].key) { rows[j]=rows[j-1]; --j; }
        rows[j]=value;
    }
    MEMORY_BASIC_INFORMATION memory;
    if (VirtualQuery(begin,&memory,sizeof(memory))!=sizeof(memory) || memory.State!=MEM_COMMIT ||
        (memory.Protect!=PAGE_READWRITE && memory.Protect!=PAGE_WRITECOPY) ||
        (uintptr_t)end>(uintptr_t)memory.BaseAddress+memory.RegionSize) { probe_sort_line=__LINE__; ++probe_sort_rejections; return; }
    for (unsigned i=0;i<count;++i) pointers[i]=rows[i].pointer;
    memcpy(begin,pointers,bytes);
    ++probe_sort_calls;
    DWORD hash=2166136261u;
    for (unsigned i=0;i<count;++i) { hash^=rows[i].pointer; hash*=16777619u; }
    if (hash==probe_sort_last_hash) return;
    probe_sort_last_hash=hash;
    char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.probe-order.jsonl",control);
    FILE *file=fopen(path,"a"); if (!file) return;
    fprintf(file,"{\"calls\":%u,\"rejections\":%u,\"count\":%u,\"order\":[",probe_sort_calls,probe_sort_rejections,count);
    for (unsigned i=0;i<count;++i) {
        LegacyProbeOrder *r=&rows[i];
        fprintf(file,"%s{\"pointer\":%lu,\"geometry\":%lu,\"native_key\":%.9g,\"source_key\":%.9g,\"position\":[%.9g,%.9g,%.9g]}",i?",":"",r->pointer,r->geometry,r->native_key,r->key,r->position[0],r->position[1],r->position[2]);
    }
    fprintf(file,"]}\n"); fclose(file);
}

/* Native usercall: EAX=begin, EDI=end, one caller-cleaned stack argument.
   Preserve its result general registers/flags after running the unchanged native sort. */
static void __attribute__((naked,used)) legacy_probe_sort_bridge(void) {
    __asm__ volatile(
        "push %edi\n\tpush %eax\n\tpushl 12(%esp)\n\t"
        "call *_probe_native_final_sort\n\tlea 4(%esp),%esp\n\t"
        "pushfl\n\tpushal\n\t"
        "mov 36(%esp),%eax\n\tmov 40(%esp),%edx\n\t"
        "push %edx\n\tpush %eax\n\tcall _legacy_probe_sort\n\tadd $8,%esp\n\t"
        "popal\n\tpopfl\n\tlea 8(%esp),%esp\n\tret\n\t");
}

static int probe_sort_enabled;
static void probe_sort_status(void) {
    if (!probe_sort_enabled) return;
    char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.probe-status.json",control);
    FILE *file=fopen(path,"w"); if (!file) return;
    fprintf(file,"{\"attempts\":%u,\"calls\":%u,\"rejections\":%u,\"begin\":%lu,\"end\":%lu,\"line\":%u,\"index\":%u}\n",probe_sort_attempts,probe_sort_calls,probe_sort_rejections,probe_sort_begin,probe_sort_end,probe_sort_line,probe_sort_index);
    fclose(file);
}

static int install_legacy_probe_sort(void) {
    char option[8];
    if (GetEnvironmentVariableA("SRP_LEGACY_PROBE_SORT_011A",option,sizeof(option))!=1 || option[0]!='1') return 1;
    probe_sort_enabled=1;
    static const unsigned char call[]={0xe8,0xf8,0x28,0x00,0x00};
    static const unsigned char compare[]={0xf3,0x0f,0x10,0x83,0x20,0x03,0x00,0x00,0x0f,0x2f,0x80,0x20,0x03,0x00,0x00};
    static const unsigned char box[]={0x8d,0xb3,0x34,0x02,0x00,0x00,0xb9,0x10,0x00,0x00,0x00};
    unsigned char actual[sizeof(compare)];
    if (strcmp(build,"026g") || (uintptr_t)GetModuleHandleA(NULL)!=0x10000 ||
        !probe_order_read(0x1b6b63,actual,sizeof(call)) || memcmp(actual,call,sizeof(call)) ||
        !probe_order_read(0x1b9e85,actual,sizeof(compare)) || memcmp(actual,compare,sizeof(compare)) ||
        !probe_order_read(0x1c3b52,actual,sizeof(box)) || memcmp(actual,box,sizeof(box))) {
        error("legacy probe order requires verified 0.26 sort/layout"); return 0;
    }
    unsigned char patch[5]={0xe8};
    int32_t relative=(int32_t)((uintptr_t)legacy_probe_sort_bridge-(0x1b6b63+5));
    memcpy(patch+1,&relative,4);
    DWORD protection=0,unused=0;
    void *at=(void *)(uintptr_t)0x1b6b63;
    if (!VirtualProtect(at,sizeof(patch),PAGE_EXECUTE_READWRITE,&protection)) { error("probe sort protection failed"); return 0; }
    memcpy(at,patch,sizeof(patch));
    FlushInstructionCache(GetCurrentProcess(),at,sizeof(patch));
    VirtualProtect(at,sizeof(patch),protection,&unused);
    return 1;
}
