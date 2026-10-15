// SPDX-License-Identifier: GPL-3.0-or-later
// Opt-in SRP_STEP_LOG=1 (build 026g only): log every footstep query to
// step.log next to the control file (<runtime>/step.log).
//
// The 0.26 step handler on_step (VA 0x248ad0; the same code as 0.11e
// survarium::step_manager::on_step) casts a ray 2 m down from 1 m above the
// feet. On a hit it asks the hit object for its 16-bit surface material and
// calls get_pair(manager, actor, surface) (VA 0x306d50). get_pair indexes
// the 128-slot material table at manager+0x108 with all 16 bits, takes the
// material's base (+0x66) unless it is 0xffff, falls back to the default
// (+0x1030c) for an empty slot, and returns pairs[actor*128+surface] from
// +0x308 (the default surface column when that slot is empty). on_step plays
// the pair's sound (+0x20) and effect (+0x14) when they are not null.
//
// Two call sites of on_step and the get_pair call inside it are retargeted
// to the stubs below, and only after every byte they rely on is checked. On
// any mismatch or error nothing is patched, the reason goes to step.log and
// the game runs as without the log; this never stops the game.
//
// On the game thread the stubs save flags, integer, x87 and SSE state and
// queue a small record: arguments, the position (read with
// ReadProcessMemory) and on_step's stack frame. A writer thread formats the
// records, reads the material table, the pair and its sound through
// ReadProcessMemory (an unreadable address gives null, never a fault), and
// writes each line with its own WriteFile (no stdio buffer, nothing held
// back in the process). A step with no pair line is a ray miss.
//
// Lines: {"installed":1|0,...} once per start; {"step":n,...} per call of
// on_step; {"pair":n,...} per ray hit (n = the step on the same thread);
// {"material":...} and {"sound_object":...} once per distinct pointer, with
// the strings their words point to; {"dropped":n} when the queue overflowed.

#define STEP_RING 512
#define STEP_FRAME 0x60

typedef struct {
    unsigned kind; // 1 step, 2 pair
    DWORD ms, thread;
    int position_ok, frame_ok;
    float position[3];
    uint32_t arg1, actor, arg3, manager, surface, pair;
    unsigned char frame[STEP_FRAME];
} StepRecord;

static CRITICAL_SECTION step_lock;
static StepRecord step_ring[STEP_RING];
static unsigned step_first, step_queued, step_dropped;
static HANDLE step_file=INVALID_HANDLE_VALUE;
uintptr_t step_on_step, step_get_pair; // jump targets of the stubs

static int step_read(uintptr_t address, void *out, size_t size) {
    SIZE_T done=0;
    if (address<0x10000 || address+size<address) return 0;
    return ReadProcessMemory(GetCurrentProcess(),(const void *)address,out,size,&done) && done==size;
}

static uint32_t step_u32(uintptr_t address, int *ok) {
    uint32_t value=0;
    if (!step_read(address,&value,sizeof(value))) { value=0; if (ok) *ok=0; }
    return value;
}

static unsigned step_u16(uintptr_t address, int *ok) {
    uint16_t value=0;
    if (!step_read(address,&value,sizeof(value))) { value=0; if (ok) *ok=0; }
    return value;
}

static void step_write(const char *line) {
    if (step_file==INVALID_HANDLE_VALUE) return;
    DWORD length=(DWORD)strlen(line), written=0;
    // One unbuffered WriteFile per line: once written, a line survives a game crash.
    WriteFile(step_file,line,length,&written,NULL);
}

static void step_push(const StepRecord *record) {
    EnterCriticalSection(&step_lock);
    if (step_queued<STEP_RING) { step_ring[(step_first+step_queued)%STEP_RING]=*record; ++step_queued; }
    else ++step_dropped;
    LeaveCriticalSection(&step_lock);
}

// Called by step_entry on the game thread.
void __cdecl step_log_begin(uintptr_t position, uint32_t arg1, uint32_t actor, uint32_t arg3) {
    StepRecord record;
    memset(&record,0,sizeof(record));
    record.kind=1; record.ms=GetTickCount(); record.thread=GetCurrentThreadId();
    record.position_ok=step_read(position,record.position,sizeof(record.position));
    record.arg1=arg1; record.actor=actor; record.arg3=arg3;
    step_push(&record);
}

// Called by step_pair on the game thread, after the real get_pair.
void __cdecl step_log_pair(uint32_t manager, uint32_t actor, uint32_t surface, uint32_t pair, uintptr_t frame) {
    StepRecord record;
    memset(&record,0,sizeof(record));
    record.kind=2; record.ms=GetTickCount(); record.thread=GetCurrentThreadId();
    record.manager=manager; record.actor=actor; record.surface=surface; record.pair=pair;
    record.frame_ok=step_read(frame,record.frame,sizeof(record.frame));
    step_push(&record);
}

// The words of an object that point to a short printable string, as JSON
// members "0xOFFSET":"text" (no quote or backslash is accepted).
static void step_strings(uintptr_t object, size_t size, char *out, size_t out_size) {
    size_t used=0;
    out[0]=0;
    for (size_t offset=0; offset+4<=size; offset+=4) {
        uint32_t target=step_u32(object+offset,NULL);
        char text[49];
        if (!target || !step_read(target,text,sizeof(text)-1)) continue;
        text[sizeof(text)-1]=0;
        size_t length=0;
        while (length<sizeof(text)-1 && text[length]>=0x20 && text[length]<0x7f && text[length]!='"' && text[length]!='\\') ++length;
        if (length<3 || (length<sizeof(text)-1 && text[length]!=0)) continue;
        text[length]=0;
        int n=snprintf(out+used,out_size-used,"%s\"0x%x\":\"%s\"",used ? "," : "",(unsigned)offset,text);
        if (n<0 || (size_t)n>=out_size-used) { out[used]=0; return; }
        used+=(size_t)n;
    }
}

static void step_dump(const char *key, uintptr_t object, size_t size) {
    unsigned char bytes[0x80];
    char hex[2*sizeof(bytes)+1], strings[1024], line[2*sizeof(bytes)+1200];
    if (size>sizeof(bytes)) size=sizeof(bytes);
    if (!step_read(object,bytes,size)) { snprintf(line,sizeof(line),"{\"%s\":\"0x%08x\",\"readable\":0}\n",key,(unsigned)object); step_write(line); return; }
    for (size_t i=0; i<size; ++i) snprintf(hex+2*i,3,"%02x",bytes[i]);
    step_strings(object,size,strings,sizeof(strings));
    snprintf(line,sizeof(line),"{\"%s\":\"0x%08x\",\"bytes\":\"%s\",\"strings\":{%s}}\n",key,(unsigned)object,hex,strings);
    step_write(line);
}

// Remember up to 128 dumped pointers per kind.
static int step_first_seen(uint32_t *seen, unsigned *count, uint32_t pointer) {
    for (unsigned i=0; i<*count; ++i) if (seen[i]==pointer) return 0;
    if (*count>=128) return 0;
    seen[(*count)++]=pointer;
    return 1;
}

// get_pair's resolution of one material value, read the way the game reads it.
static unsigned step_resolve(uint32_t manager, unsigned value, uint32_t *entry, unsigned *base, int *ok) {
    unsigned index=value & 0xffff;
    *entry=step_u32(manager+0x108+4u*index,ok);
    *base=0xffff;
    if (!*entry) return step_u16(manager+0x1030c,ok);
    *base=step_u16(*entry+0x66,ok);
    return *base==0xffff ? index : *base;
}

static void step_format(const StepRecord *r) {
    static unsigned steps, threads;
    static DWORD step_thread[16];
    static unsigned step_number[16];
    static uint32_t materials_seen[128], sounds_seen[128];
    static unsigned materials_count, sounds_count;
    char line[1024];
    if (r->kind==1) {
        ++steps;
        unsigned slot=0;
        while (slot<threads && step_thread[slot]!=r->thread) ++slot;
        if (slot==threads) { if (threads<16) ++threads; else slot=steps%16; step_thread[slot]=r->thread; }
        step_number[slot]=steps;
        char position[96]="null";
        if (r->position_ok) snprintf(position,sizeof(position),"[%.3f,%.3f,%.3f]",r->position[0],r->position[1],r->position[2]);
        snprintf(line,sizeof(line),"{\"step\":%u,\"ms\":%lu,\"thread\":%lu,\"position\":%s,\"arg1\":\"0x%08x\",\"actor\":%u,\"arg3\":%u}\n",
            steps,(unsigned long)r->ms,(unsigned long)r->thread,position,(unsigned)r->arg1,(unsigned)(r->actor & 0xffff),(unsigned)(r->arg3 & 0xff));
        step_write(line);
        return;
    }
    unsigned step=0;
    for (unsigned slot=0; slot<threads; ++slot) if (step_thread[slot]==r->thread) step=step_number[slot];
    int ok=1;
    unsigned raw=r->surface & 0xffff, actor_base, surface_base;
    uint32_t actor_entry, surface_entry;
    unsigned a=step_resolve(r->manager,r->actor,&actor_entry,&actor_base,&ok);
    unsigned b=step_resolve(r->manager,raw,&surface_entry,&surface_base,&ok);
    unsigned column=b, column_default=0;
    if (!step_u32(r->manager+0x308+4u*(a*128+b),&ok)) { column=step_u16(r->manager+0x1030c,&ok); column_default=1; }
    uint32_t expected=step_u32(r->manager+0x308+4u*(a*128+column),&ok);
    uint32_t sound=r->pair ? step_u32(r->pair+0x20,&ok) : 0, effect=r->pair ? step_u32(r->pair+0x14,&ok) : 0;
    uint32_t hit=0; float distance=0;
    if (r->frame_ok) { memcpy(&hit,r->frame+0x4c,4); memcpy(&distance,r->frame+0x50,4); }
    char frame[2*STEP_FRAME+1]="";
    if (r->frame_ok) for (unsigned i=0; i<STEP_FRAME; ++i) snprintf(frame+2*i,3,"%02x",r->frame[i]);
    snprintf(line,sizeof(line),
        "{\"pair\":%u,\"ms\":%lu,\"thread\":%lu,\"actor\":%u,\"actor_entry\":\"0x%08x\",\"actor_base\":%u,\"actor_lookup\":%u,"
        "\"surface_raw\":%u,\"surface_id\":%u,\"surface_wb\":%u,\"surface_in_table\":%d,\"surface_entry\":\"0x%08x\",\"surface_base\":%u,"
        "\"surface_lookup\":%u,\"column\":%u,\"column_default\":%d,\"pair_ptr\":\"0x%08x\",\"pair_expected\":\"0x%08x\",\"reads_ok\":%d,"
        "\"sound\":\"0x%08x\",\"effect\":\"0x%08x\",\"hit\":\"0x%08x\",\"frame_0x50_float\":%.4f,\"frame\":\"%s\"}\n",
        step,(unsigned long)r->ms,(unsigned long)r->thread,(unsigned)(r->actor & 0xffff),(unsigned)actor_entry,actor_base,a,
        raw,raw & 0x3fff,raw>>14,raw<128,(unsigned)surface_entry,surface_base,b,column,column_default,(unsigned)r->pair,(unsigned)expected,ok,
        (unsigned)sound,(unsigned)effect,(unsigned)hit,(double)distance,frame);
    step_write(line);
    if (surface_entry && step_first_seen(materials_seen,&materials_count,surface_entry)) step_dump("material",surface_entry,0x6c);
    if (actor_entry && step_first_seen(materials_seen,&materials_count,actor_entry)) step_dump("material",actor_entry,0x6c);
    if (sound && step_first_seen(sounds_seen,&sounds_count,sound)) step_dump("sound_object",sound,0x80);
}

static DWORD WINAPI step_writer(void *unused) {
    (void)unused;
    static StepRecord batch[64];
    unsigned reported=0;
    for (;;) {
        unsigned count, dropped;
        EnterCriticalSection(&step_lock);
        count=step_queued<64 ? step_queued : 64;
        for (unsigned i=0; i<count; ++i) batch[i]=step_ring[(step_first+i)%STEP_RING];
        step_first=(step_first+count)%STEP_RING; step_queued-=count; dropped=step_dropped;
        LeaveCriticalSection(&step_lock);
        for (unsigned i=0; i<count; ++i) step_format(&batch[i]);
        if (dropped!=reported) {
            char line[64]; snprintf(line,sizeof(line),"{\"dropped\":%u}\n",dropped); step_write(line); reported=dropped;
        }
        if (!count) Sleep(20);
    }
    return 0;
}

// Entered by `call` from an on_step call site in place of on_step:
// eax = position, ecx = this. After pushfl/pushal and %ebp = %esp:
// [ebp+0..31] registers, [ebp+32] flags, [ebp+36] return address,
// [ebp+40] [ebp+44] [ebp+48] on_step's three stack arguments (the second is
// the actor material). The stub then jumps to on_step with the caller's
// registers, flags and stack restored.
__attribute__((naked)) static void step_entry(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\tcld\n\t"
        "pushl 48(%ebp)\n\tpushl 44(%ebp)\n\tpushl 40(%ebp)\n\tpushl %eax\n\tcall _step_log_begin\n\taddl $16, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "jmp *_step_on_step\n\t");
}

// Replaces `call get_pair` inside on_step: ecx = manager, dx = actor,
// di = surface; get_pair takes no stack arguments, returns in eax and ends
// in a plain ret. edx is pushed to keep the actor, then the real get_pair
// runs. After pushfl/pushal: [ebp+36] the saved edx, [ebp+40] the return
// into on_step, on_step's own %esp = ebp+44 (its frame: the hit object at
// +0x4c). popal restores get_pair's eax and edx for on_step.
__attribute__((naked)) static void step_pair(void) {
    __asm__ volatile(
        "pushl %edx\n\tcall *_step_get_pair\n\t"
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\tcld\n\tsubl $12, %esp\n\t"
        "leal 44(%ebp), %edx\n\tpushl %edx\n\tpushl %eax\n\tpushl %edi\n\tpushl 36(%ebp)\n\tpushl %ecx\n\t"
        "call _step_log_pair\n\taddl $32, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "addl $4, %esp\n\tret\n\t");
}

typedef struct { unsigned offset; unsigned char bytes[16]; unsigned length; } StepPin;

// Every byte the stubs rely on, at the pinned 026g executable (base 0x10000).
static const StepPin step_pins[]={
    // on_step: prologue (frame layout), the hit object at [esp+0x4c], the
    // get_pair call (ecx manager, edx actor = [ebp+0xc], di surface) and ret 0xc.
    {0x238ad0,{0x55,0x8b,0xec,0x83,0xe4,0xf8,0x83,0xec,0x4c},9},
    {0x238aea,{0x53,0x56,0x57},3},
    {0x238b5a,{0x8b,0x4c,0x24,0x4c,0x85,0xc9},6},
    {0x238b7a,{0x0f,0xb7,0xf8},3},
    {0x238b84,{0x8b,0x55,0x0c,0x8b,0xc8,0xe8,0xc2,0xe1,0x0b,0x00,0x8b,0xf8},12},
    {0x238cdd,{0xc2,0x0c,0x00},3},
    // get_pair: register arguments, table at +0x108, plain ret.
    {0x2f6d50,{0x0f,0xb7,0xc2,0x8b,0x84,0x81,0x08,0x01,0x00,0x00,0x56},11},
    {0x2f6df1,{0x8b,0x84,0x91,0x08,0x03,0x00,0x00,0xc3},8},
    // The two on_step call sites: eax = position, three pushed arguments.
    {0x22fc22,{0x50,0x8b,0x44,0x24,0x18,0x51,0x8b,0x4c,0x24,0x18,0xe8,0x9f,0x8e,0x00,0x00},15},
    {0x29656b,{0x52,0x57,0x50,0x8b,0x44,0x24,0x1c,0xe8,0x59,0x25,0xfa,0xff},12},
};

static const unsigned step_sites[3]={0x22fc2c,0x296572,0x238b89};

static void step_install_line(int installed, const char *reason) {
    char line[320];
    snprintf(line,sizeof(line),"{\"installed\":%d,\"ms\":%lu,\"pid\":%lu,\"reason\":\"%s\"}\n",installed,
        (unsigned long)GetTickCount(),(unsigned long)GetCurrentProcessId(),reason);
    step_write(line);
}

static void install_step_diagnostic(void) {
    char flag[4];
    if (GetEnvironmentVariableA("SRP_STEP_LOG",flag,sizeof(flag))!=1 || flag[0]!='1') return;
    char path[MAX_PATH+16];
    snprintf(path,sizeof(path),"%s",control);
    char *slash=strrchr(path,'\\');
    if (!slash) slash=strrchr(path,'/');
    if (!slash) return;
    snprintf(slash+1,sizeof(path)-(size_t)(slash+1-path),"step.log");
    step_file=CreateFileA(path,GENERIC_WRITE,FILE_SHARE_READ,NULL,OPEN_ALWAYS,FILE_ATTRIBUTE_NORMAL,NULL);
    if (step_file==INVALID_HANDLE_VALUE) return;
    LARGE_INTEGER zero={0};
    if (!SetFilePointerEx(step_file,zero,NULL,FILE_END)) { CloseHandle(step_file); step_file=INVALID_HANDLE_VALUE; return; }
    if (strcmp(build,"026g")) { step_install_line(0,"SRP_STEP_LOG is pinned to 026g"); return; }
    uintptr_t base=(uintptr_t)GetModuleHandleW(NULL);
    IMAGE_DOS_HEADER dos; IMAGE_NT_HEADERS32 nt;
    if (!step_read(base,&dos,sizeof(dos)) || dos.e_magic!=IMAGE_DOS_SIGNATURE || !step_read(base+(uintptr_t)dos.e_lfanew,&nt,sizeof(nt)) ||
        nt.Signature!=IMAGE_NT_SIGNATURE || nt.OptionalHeader.SizeOfImage<0x300000) {
        step_install_line(0,"unexpected executable headers"); return;
    }
    for (unsigned i=0; i<sizeof(step_pins)/sizeof(step_pins[0]); ++i) {
        unsigned char bytes[16];
        if (!step_read(base+step_pins[i].offset,bytes,step_pins[i].length) || memcmp(bytes,step_pins[i].bytes,step_pins[i].length)) {
            char reason[96]; snprintf(reason,sizeof(reason),"bytes at RVA 0x%x do not match the pinned executable",step_pins[i].offset);
            step_install_line(0,reason); return;
        }
    }
    step_on_step=base+0x238ad0; step_get_pair=base+0x2f6d50;
    const uintptr_t targets[3]={step_on_step,step_on_step,step_get_pair};
    void *const hooks[3]={(void *)step_entry,(void *)step_entry,(void *)step_pair};
    for (unsigned i=0; i<3; ++i) {
        unsigned char *site=(unsigned char *)(base+step_sites[i]);
        if (site[0]!=0xe8 || (uintptr_t)(site+5+*(int32_t *)(site+1))!=targets[i]) { step_install_line(0,"call site target mismatch"); return; }
    }
    InitializeCriticalSection(&step_lock);
    // Patch all three or none: a failed VirtualProtect restores the sites already patched.
    int32_t original[3];
    unsigned patched=0;
    for (; patched<3; ++patched) {
        unsigned char *site=(unsigned char *)(base+step_sites[patched]);
        DWORD protection;
        if (!VirtualProtect(site,5,PAGE_EXECUTE_READWRITE,&protection)) break;
        original[patched]=*(int32_t *)(site+1);
        *(int32_t *)(site+1)=(int32_t)((unsigned char *)hooks[patched]-(site+5));
        FlushInstructionCache(GetCurrentProcess(),site,5);
        VirtualProtect(site,5,protection,&protection);
    }
    if (patched<3) {
        while (patched--) {
            unsigned char *site=(unsigned char *)(base+step_sites[patched]);
            DWORD protection;
            if (VirtualProtect(site,5,PAGE_EXECUTE_READWRITE,&protection)) {
                *(int32_t *)(site+1)=original[patched];
                FlushInstructionCache(GetCurrentProcess(),site,5);
                VirtualProtect(site,5,protection,&protection);
            }
        }
        step_install_line(0,"VirtualProtect failed; call sites restored");
        return;
    }
    // Only this thread writes until the writer starts; records queue meanwhile.
    step_install_line(1,"on_step and get_pair call sites retargeted");
    HANDLE thread=CreateThread(NULL,0,step_writer,NULL,0,NULL);
    if (!thread) { step_install_line(0,"cannot start the writer thread; records are dropped"); return; }
    CloseHandle(thread);
}
