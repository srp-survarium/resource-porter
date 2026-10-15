// SPDX-License-Identifier: GPL-3.0-or-later
// Opt-in SRP_STEP_LOG=1 (build 026g only): log every footstep query to
// <control>.steps.jsonl. The 0.26 step handler (VA 0x248ad0, the same code as
// 0.11e survarium::step_manager::on_step) casts 2 m down from 1 m above the
// feet, takes the hit surface's packed game material (wb << 14 | id) and
// plays the collision_sound of get_pair(actor, surface) (VA 0x306d50).
// Two kinds of lines: {"step":n,...} when the handler is entered (position,
// actor material) and {"pair":n,...} when the ray hit something (surface,
// what get_pair resolved, the pair and its sound pointers). A step with no
// pair line is a ray miss. Only the two on_step call sites and the get_pair
// call inside it are retargeted, after their bytes are checked.
static FILE *step_log; static unsigned step_count; uintptr_t step_on_step, step_get_pair;
static float step_position[3]; static unsigned step_actor;

void __cdecl step_log_begin(const float *position, unsigned actor) {
    ++step_count; step_actor=actor & 0xffff;
    if (position) memcpy(step_position,position,sizeof(step_position));
    if (step_log) { fprintf(step_log,"{\"step\":%u,\"ms\":%lu,\"position\":[%.3f,%.3f,%.3f],\"actor\":%u}\n",step_count,
        (unsigned long)GetTickCount(),step_position[0],step_position[1],step_position[2],step_actor); fflush(step_log); }
}

void __cdecl step_log_pair(const unsigned char *manager, unsigned actor, unsigned surface, const unsigned char *pair, const unsigned char *hit) {
    if (!step_log) return;
    unsigned raw=surface & 0xffff, id=raw & 0x3fff, used=id, base=0xffff, wb=raw>>14, defaults=*(const uint16_t *)(manager+0x1030c);
    const unsigned char *material=id<128 ? *(const unsigned char *const *)(manager+0x108+4*id) : NULL;
    if (material) { base=*(const uint16_t *)(material+0x66); if (base!=0xffff) used=base; } else used=defaults;
    const void *sound=pair ? *(const void *const *)(pair+0x20) : NULL, *effect=pair ? *(const void *const *)(pair+0x14) : NULL;
    fprintf(step_log,"{\"pair\":%u,\"actor\":%u,\"surface_raw\":%u,\"surface_id\":%u,\"surface_wb\":%u,\"material_known\":%d,\"base\":%u,\"lookup\":%u,"
        "\"pair_ptr\":\"%p\",\"sound\":\"%p\",\"effect\":\"%p\",\"hit\":\"%p\"}\n",
        step_count,actor & 0xffff,raw,id,wb,material!=NULL,base,used,(const void *)pair,sound,effect,(const void *)hit);
    fflush(step_log);
}

// Both stubs save flags, integer, x87 and SSE state (fxsave, like camera_entry)
// and address the game's stack through %ebp = %esp after pushfl/pushal:
// [ebp+0..31] registers, [ebp+32] flags, [ebp+36] first word above.
// Entered by `call` from an on_step call site: eax = position, then
// [ebp+36] return, [ebp+40] first argument, [ebp+44] actor material.
__attribute__((naked)) static void step_entry(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\tsubl $8, %esp\n\t"
        "pushl 44(%ebp)\n\tpushl %eax\n\tcall _step_log_begin\n\taddl $16, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "jmp *_step_on_step\n\t");
}

// Replaces `call get_pair` inside on_step: ecx = manager, dx = actor, di = surface.
// After push edx, the real call, pushfl and pushal: [ebp+36] saved edx (actor),
// [ebp+40] return into on_step, on_step's own %esp = ebp+44, and its
// [esp+0x4c] (the ray's hit object) = [ebp+120]. get_pair's results (eax, edx)
// are restored by popal.
__attribute__((naked)) static void step_pair(void) {
    __asm__ volatile(
        "pushl %edx\n\tcall *_step_get_pair\n\t"
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\tsubl $12, %esp\n\t"
        "pushl 120(%ebp)\n\tpushl %eax\n\tpushl %edi\n\tpushl 36(%ebp)\n\tpushl %ecx\n\t"
        "call _step_log_pair\n\taddl $32, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "addl $4, %esp\n\tret\n\t");
}

static int retarget_call(unsigned char *site, uintptr_t expected_target, void *hook) {
    if (site[0]!=0xe8 || (uintptr_t)(site+5+*(int32_t *)(site+1))!=expected_target) return 0;
    DWORD protection;
    if (!VirtualProtect(site,5,PAGE_EXECUTE_READWRITE,&protection)) return 0;
    *(int32_t *)(site+1)=(int32_t)((unsigned char *)hook-(site+5));
    FlushInstructionCache(GetCurrentProcess(),site,5);
    VirtualProtect(site,5,protection,&protection);
    return 1;
}

static int install_step_diagnostic(void) {
    char flag[4];
    if (GetEnvironmentVariableA("SRP_STEP_LOG",flag,sizeof(flag))!=1 || flag[0]!='1') return 1;
    if (strcmp(build,"026g")) { error("SRP_STEP_LOG is pinned to 026g"); return 0; }
    unsigned char *base=(unsigned char *)GetModuleHandleW(NULL);
    step_on_step=(uintptr_t)(base+0x238ad0); step_get_pair=(uintptr_t)(base+0x2f6d50);
    static const unsigned char on_step_head[]={0x55,0x8b,0xec,0x83,0xe4,0xf8,0x83,0xec,0x4c};
    static const unsigned char pair_head[]={0x0f,0xb7,0xc2,0x8b,0x84,0x81,0x08,0x01,0x00,0x00};
    if (memcmp((void *)step_on_step,on_step_head,sizeof(on_step_head)) || memcmp((void *)step_get_pair,pair_head,sizeof(pair_head))) {
        error("step diagnostic bytes do not match the pinned executable"); return 0;
    }
    char path[MAX_PATH+24]; snprintf(path,sizeof(path),"%s.steps.jsonl",control);
    step_log=fopen(path,"a");
    if (!step_log) { error("cannot open the step log"); return 0; }
    if (!retarget_call(base+0x22fc2c,step_on_step,(void *)step_entry) || !retarget_call(base+0x296572,step_on_step,(void *)step_entry) ||
        !retarget_call(base+0x238b89,step_get_pair,(void *)step_pair)) {
        error("step diagnostic call sites do not match the pinned executable"); return 0;
    }
    fprintf(step_log,"{\"installed\":1,\"ms\":%lu}\n",(unsigned long)GetTickCount()); fflush(step_log);
    return 1;
}
