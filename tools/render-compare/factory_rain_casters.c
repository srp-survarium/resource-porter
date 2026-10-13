// SPDX-License-Identifier: GPL-3.0-or-later
// Diagnostic 0.10b rain predicate: recomputed factor or private source-like cache.
// Native surface factors, occlusion, geometry and bounds remain unchanged.
static int rain_source_cache_enabled;
static float rain_source_cached_factor(uintptr_t instance,uintptr_t parent);
static unsigned rain_caster_calls, rain_caster_skipped, rain_caster_fallbacks;
static unsigned rain_caster_disagreements, rain_caster_log_id, rain_caster_seen_count;
static uintptr_t rain_caster_seen[2048];
static FILE *rain_caster_log;
static void * __attribute__((used)) rain_caster_continue=(void *)0x17c7ff, * __attribute__((used)) rain_caster_skip=(void *)0x17c84b;

static int rain_caster_read(uintptr_t address,void *out,SIZE_T size) {
    SIZE_T count=0;
    return address && ReadProcessMemory(GetCurrentProcess(),(void *)address,out,size,&count) && count==size;
}

// Matches the old aabb::modify accumulation order and renderer::screen_factor.
__attribute__((target("sse2,fpmath=sse")))
static float rain_source_factor(const float box[6],const float matrix[16],const float eye[3],float world[6]) {
    for (unsigned j=0;j<3;++j) {
        world[j]=box[0]*matrix[j]+box[1]*matrix[4+j]+box[2]*matrix[8+j]+matrix[12+j];
        world[3+j]=world[j];
    }
    for (unsigned i=0;i<3;++i) for (unsigned j=0;j<3;++j) {
        float value=matrix[i*4+j]*(box[3+i]-box[i]);
        if (value<0) world[j]+=value; else world[3+j]+=value;
    }
    float half[3],delta[3];
    for (unsigned j=0;j<3;++j) {
        half[j]=(world[3+j]-world[j])*.5f;
        delta[j]=eye[j]-(world[3+j]+world[j])*.5f;
    }
    // The inlined original caller sums X² + Z² + Y² (647cd6..647cec).
    float distance=delta[0]*delta[0]+delta[2]*delta[2]+delta[1]*delta[1];
    float maximum=half[1]>half[2]?half[1]:half[2];
    maximum=half[0]>maximum?half[0]:maximum;
    float result=maximum/(distance>1e-6f?distance:1e-6f);
    return result<0?0:(result>1?1:result);
}

static void rain_caster_record(uintptr_t instance,uintptr_t parent,uintptr_t original,
                               int result,int native_keep,float factor,float projected,
                               unsigned clip,const float *box,const float *matrix,
                               const float *eye,const float *world) {
    Camera camera;
    EnterCriticalSection(&lock);
    camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id;
    LeaveCriticalSection(&lock);
    if (!take) return;
    if (rain_caster_log_id!=camera.id) {
        if (rain_caster_log) fclose(rain_caster_log);
        char path[MAX_PATH+40];
        snprintf(path,sizeof(path),"%s.rain-casters.jsonl",camera.output);
        rain_caster_log=fopen(path,"w"); rain_caster_log_id=camera.id; rain_caster_seen_count=0;
    }
    if (!rain_caster_log || rain_caster_seen_count>=2048) return;
    for (unsigned i=0;i<rain_caster_seen_count;++i) if (rain_caster_seen[i]==instance) return;
    rain_caster_seen[rain_caster_seen_count++]=instance;
    fprintf(rain_caster_log,"{\"instance\":%lu,\"parent\":%lu,\"original_model\":%lu,\"decision\":%d,\"native_keep\":%d,\"source_factor\":%.9g,\"native_projected_factor\":%.9g,\"native_clip_enabled\":%u,\"camera_calls\":%u,\"present_calls\":%u,\"totals\":[%u,%u,%u,%u]",
        (unsigned long)instance,(unsigned long)parent,(unsigned long)original,result,native_keep,
        isfinite(factor)?factor:-1,isfinite(projected)?projected:-1,clip,camera_calls,present_calls,
        rain_caster_calls,rain_caster_skipped,rain_caster_fallbacks,rain_caster_disagreements);
    const char *names[]={"local_parent_aabb","instance_transform","main_camera","world_parent_aabb"};
    const float *values[]={box,matrix,eye,world}; const unsigned counts[]={6,16,3,6};
    for (unsigned j=0;j<4;++j) if (values[j]) {
        fprintf(rain_caster_log,",\"%s\":[",names[j]);
        for (unsigned i=0;i<counts[j];++i) fprintf(rain_caster_log,"%s%.9g",i?",":"",isfinite(values[j][i])?values[j][i]:-1);
        fputc(']',rain_caster_log);
    }
    fputs("}\n",rain_caster_log); fflush(rain_caster_log);
}

// 0=skip, 1=keep, 2=run the unchanged native predicate.
static int __attribute__((used)) rain_source_caster(uintptr_t instance,const float *saved_eye) {
    uintptr_t parent=0,table=0,original=0,transform=0;
    float box[6]={0},matrix[16]={0},eye[3]={0},world[6]={0},projected=0,factor=-1;
    unsigned char clip=0; int native_keep=1;
    ++rain_caster_calls;
    if (!rain_caster_read(instance+0x3c,&parent,4) ||
        !rain_caster_read(instance+0x58,&projected,4) ||
        !rain_caster_read(parent+0x116,&clip,1)) goto fallback;
    native_keep=!clip || !(projected<10.0f);
    if (!rain_caster_read(parent,&table,4) || table!=0x9f2b24 ||
        !rain_caster_read(parent+0x290,&original,4) ||
        !rain_caster_read(original+0x108,box,sizeof(box)) ||
        !rain_caster_read(instance+0x50,&transform,4) ||
        !rain_caster_read(transform,matrix,sizeof(matrix)) ||
        !rain_caster_read((uintptr_t)saved_eye,eye,sizeof(eye))) goto fallback;
    for (unsigned i=0;i<6;++i) if (!isfinite(box[i])) goto fallback;
    for (unsigned i=0;i<16;++i) if (!isfinite(matrix[i])) goto fallback;
    for (unsigned i=0;i<3;++i) if (!isfinite(eye[i]) || box[i]>box[i+3]) goto fallback;
    if (fabsf(matrix[3])+fabsf(matrix[7])+fabsf(matrix[11])>1e-5f || fabsf(matrix[15]-1)>1e-5f) goto fallback;
    factor=rain_source_factor(box,matrix,eye,world);
    if (rain_source_cache_enabled) factor=rain_source_cached_factor(instance,parent);
    if (!isfinite(factor)) goto fallback;
    int result=!(factor<.004f);
    if (!result) ++rain_caster_skipped;
    if (result!=native_keep) ++rain_caster_disagreements;
    rain_caster_record(instance,parent,original,result,native_keep,factor,projected,clip,box,matrix,eye,world);
    return result;
fallback:
    ++rain_caster_fallbacks;
    rain_caster_record(instance,parent,original,2,native_keep,factor,projected,clip,NULL,NULL,NULL,NULL);
    return 2;
}

static void __attribute__((naked,used)) rain_caster_native(void) {
    __asm__ volatile(
        "mov 0x3c(%ebx),%ecx\n\tcmpb $0,0x116(%ecx)\n\tje 1f\n\t"
        "movss 0x58(%ebx),%xmm0\n\tmovss 0xa2d75c,%xmm1\n\tcomiss %xmm0,%xmm1\n\tja 2f\n\t"
        "1: jmp *_rain_caster_continue\n\t2: jmp *_rain_caster_skip\n\t");
}

static void __attribute__((naked,used)) rain_caster_bridge(void) {
    __asm__ volatile(
        "pushl $_rain_caster_native\n\tpushfl\n\tpushal\n\tmov %esp,%ebp\n\t"
        "sub $528,%esp\n\tand $-16,%esp\n\tfxsave (%esp)\n\t"
        // Original ESP = saved-register base + 40; saved camera is ESP+0x5c.
        "lea 132(%ebp),%eax\n\tpush %eax\n\tpushl 16(%ebp)\n\tcall _rain_source_caster\n\tadd $8,%esp\n\t"
        "cmp $2,%eax\n\tje 3f\n\ttest %eax,%eax\n\tje 1f\n\t"
        "mov _rain_caster_continue,%eax\n\tjmp 2f\n\t"
        "1: mov _rain_caster_skip,%eax\n\t2: mov %eax,36(%ebp)\n\t"
        "3: fxrstor (%esp)\n\tmov %ebp,%esp\n\tpopal\n\tpopfl\n\tret\n\t");
}

static int install_factory_rain_casters(void) {
    char option[8];
    if (!rain_source_cache_enabled && (GetEnvironmentVariableA("SRP_DIAGNOSTIC_FACTORY_RAIN_CASTERS",option,sizeof(option))!=1 || option[0]!='1')) return 1;
    static const unsigned char predicate[]={0x8b,0x4b,0x3c,0x80,0xb9,0x16,0x01,0,0,0,0x74,0x12,0xf3,0x0f,0x10,0x43,0x58,0xf3,0x0f,0x10,0x0d,0x5c,0xd7,0xa2,0,0x0f,0x2f,0xc8,0x77,0x4c};
    static const unsigned char getter[]={0x8b,0x89,0x90,0x02,0,0,0x8b,0x44,0x24,0x04,0xf3,0x0f,0x7e,0x81,0x08,0x01,0,0};
    static const unsigned char save_camera[]={0xf3,0x0f,0x7e,0x80,0xa4,0x54,0,0,0x66,0x0f,0xd6,0x44,0x24,0x54};
    static const unsigned char conditional_occlusion[]={0xa1,0x98,0x04,0x26,0x02,0x80,0xb8,0x2d,0x01,0,0,0,0x74,0x06,0x80,0x7b,0x61,0,0x75,0x6a};
    unsigned char *site=(unsigned char *)0x17c7e1;
    if (strcmp(build,"026g") || (uintptr_t)GetModuleHandleA(NULL)!=0x10000 ||
        memcmp(site,predicate,sizeof(predicate)) || memcmp((void *)0x16e420,getter,sizeof(getter)) ||
        memcmp((void *)0x17c310,save_camera,sizeof(save_camera)) ||
        memcmp((void *)0x17c7cd,conditional_occlusion,sizeof(conditional_occlusion)) ||
        *(uintptr_t *)0x9f2b78!=0x16e420 || *(float *)0xa2d75c!=10.0f) {
        error("Factory rain caster diagnostic requires pinned 0.26 layout"); return 0;
    }
    DWORD old,unused;
    if (!VirtualProtect(site,sizeof(predicate),PAGE_EXECUTE_READWRITE,&old)) { error("cannot write Factory caster diagnostic"); return 0; }
    site[0]=0xe9;
    int32_t jump=(int32_t)((intptr_t)rain_caster_bridge-(intptr_t)(site+5));
    memcpy(site+1,&jump,4); memset(site+5,0x90,sizeof(predicate)-5);
    FlushInstructionCache(GetCurrentProcess(),site,sizeof(predicate));
    if (!VirtualProtect(site,sizeof(predicate),old,&unused)) { error("cannot protect Factory caster diagnostic"); return 0; }
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.factory-rain-casters.json",control);
    FILE *file=fopen(path,"w");
    if (file) {
        fprintf(file,"{\"mode\":\"%s\",\"source_shader_lods\":true,\"source_threshold\":0.004,\"native_threshold\":10,\"conditional_occlusion_unchanged\":true,\"original_visibility_identity_proven\":false,\"unknown_parent_native_fallback\":true,\"row_decisions\":{\"0\":\"skip\",\"1\":\"keep\",\"2\":\"native_fallback\"}}\n",rain_source_cache_enabled?"diagnostic_source_factor_cache":"diagnostic_recomputed_source_factor");
        fclose(file);
    }
    return 1;
}
