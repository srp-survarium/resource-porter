// SPDX-License-Identifier: GPL-3.0-or-later
// Private source-like rain factor cache. Native factors and occlusion are untouched.
#define RAIN_CACHE_CAPACITY 65536u
struct RainFactorEntry {
    uintptr_t instance,parent,surface;
    float factor,eye[3];
    unsigned updates,generation,last_frame;
};
static struct RainFactorEntry rain_factor_cache[RAIN_CACHE_CAPACITY];
static CRITICAL_SECTION rain_cache_lock;
static unsigned rain_cache_generation,rain_cache_overflow,rain_cache_update_frames;
static unsigned rain_cache_candidates,rain_cache_updates,rain_cache_rejected,rain_cache_option_off;
static void * __attribute__((used)) rain_cache_constructor_resume=(void *)0x158ca0;
static void * __attribute__((used)) rain_cache_visible_resume=(void *)0x1b7366;
static unsigned rain_cache_log_id,rain_cache_log_count;
static uintptr_t rain_cache_log_seen[2048];
static FILE *rain_cache_log;

// No eviction: a missing/overflowed entry retains the source constructor default.
static struct RainFactorEntry *rain_cache_find(uintptr_t instance,int create) {
    unsigned slot=(unsigned)(((instance>>4)*2654435761u)&(RAIN_CACHE_CAPACITY-1));
    for (unsigned i=0;i<RAIN_CACHE_CAPACITY;++i) {
        struct RainFactorEntry *e=&rain_factor_cache[(slot+i)&(RAIN_CACHE_CAPACITY-1)];
        if (e->instance==instance) return e;
        if (!e->instance) {
            if (!create) return NULL;
            e->instance=instance; e->factor=1; return e;
        }
    }
    ++rain_cache_overflow; return NULL;
}
static void __attribute__((used)) rain_cache_construct(uintptr_t instance) {
    EnterCriticalSection(&rain_cache_lock);
    struct RainFactorEntry *e=rain_cache_find(instance,1);
    if (e) { memset(e,0,sizeof(*e)); e->instance=instance; e->factor=1; e->generation=++rain_cache_generation; }
    LeaveCriticalSection(&rain_cache_lock);
}

// Exact pure lookup from 16b070, for actual materials. Unknown/default materials
// are excluded and counted rather than inventing their G-buffer eligibility.
static int rain_cache_gbuffer(uintptr_t surface) {
    uintptr_t material=0,entries=0,effect=0; unsigned count=0,type=0;
    unsigned char default_material=0;
    if (!rain_caster_read(0x1f1d991,&default_material,1) || default_material ||
        !rain_caster_read(surface+0x1c,&material,4) || !material ||
        !rain_caster_read(surface+0x48,&type,4) ||
        !rain_caster_read(material+0x21c,&entries,4) || !entries ||
        !rain_caster_read(material+0x220,&count,4) || count>64) return -1;
    uintptr_t selected=entries;
    for (unsigned i=0;i<count;++i) {
        unsigned candidate;
        if (!rain_caster_read(entries+i*0x94+0x24,&candidate,4)) return -1;
        if (candidate==type) { selected=entries+i*0x94; break; }
    }
    if (!rain_caster_read(selected+0x2c,&effect,4)) return -1;
    return effect!=0;
}
static void __attribute__((used)) rain_cache_visible(uintptr_t stage,unsigned probe_pass) {
    if (probe_pass) return;
    uintptr_t options=0,context=0,view=0,begin=0,end=0;
    unsigned char shader_lods=0; float eye[3];
    if (!rain_caster_read(0x2260498,&options,4) || !rain_caster_read(options+0x131,&shader_lods,1)) return;
    if (!shader_lods) { ++rain_cache_option_off; return; }
    if (!rain_caster_read(stage+4,&context,4) || !rain_caster_read(context+0x4170,&view,4) ||
        !rain_caster_read(view+0x44a0,&begin,4) || !rain_caster_read(view+0x44a4,&end,4) ||
        end<begin || (end-begin)%4 || (end-begin)/4>32768 ||
        !rain_caster_read(context+0x54a4,eye,sizeof(eye))) { ++rain_cache_rejected; return; }
    for (unsigned j=0;j<3;++j) if (!isfinite(eye[j])) { ++rain_cache_rejected; return; }
    ++rain_cache_update_frames;
    for (uintptr_t p=begin;p<end;p+=4) {
        uintptr_t instance=0,parent=0,surface=0,table=0,original=0,transform=0;
        float box[6],matrix[16],world[6]; ++rain_cache_candidates;
        if (!rain_caster_read(p,&instance,4) || !rain_caster_read(instance+0x38,&surface,4) ||
            !rain_caster_read(instance+0x3c,&parent,4) || !rain_caster_read(parent,&table,4) || table!=0x9f2b24) { ++rain_cache_rejected; continue; }
        int eligible=rain_cache_gbuffer(surface);
        if (eligible<0) { ++rain_cache_rejected; continue; }
        if (!eligible) continue;
        if (!rain_caster_read(parent+0x290,&original,4) || !rain_caster_read(original+0x108,box,sizeof(box)) ||
            !rain_caster_read(instance+0x50,&transform,4) || !rain_caster_read(transform,matrix,sizeof(matrix))) { ++rain_cache_rejected; continue; }
        int valid=1;
        for (unsigned j=0;j<6;++j) valid &= isfinite(box[j]);
        for (unsigned j=0;j<16;++j) valid &= isfinite(matrix[j]);
        for (unsigned j=0;j<3;++j) valid &= box[j]<=box[j+3];
        valid &= fabsf(matrix[3])+fabsf(matrix[7])+fabsf(matrix[11])<=1e-5f && fabsf(matrix[15]-1)<=1e-5f;
        if (!valid) { ++rain_cache_rejected; continue; }
        float factor=rain_source_factor(box,matrix,eye,world);
        if (!isfinite(factor)) { ++rain_cache_rejected; continue; }
        EnterCriticalSection(&rain_cache_lock);
        struct RainFactorEntry *e=rain_cache_find(instance,1);
        if (e) {
            e->parent=parent; e->surface=surface; e->factor=factor;
            memcpy(e->eye,eye,sizeof(eye)); ++e->updates; e->last_frame=rain_cache_update_frames; ++rain_cache_updates;
        }
        LeaveCriticalSection(&rain_cache_lock);
    }
}
static float rain_source_cached_factor(uintptr_t instance,uintptr_t parent) {
    uintptr_t surface=0; rain_caster_read(instance+0x38,&surface,4);
    struct RainFactorEntry value={0}; value.factor=1;
    EnterCriticalSection(&rain_cache_lock);
    struct RainFactorEntry *e=rain_cache_find(instance,0);
    if (e && (!e->parent || (e->parent==parent && e->surface==surface))) value=*e;
    LeaveCriticalSection(&rain_cache_lock);
    Camera camera;
    EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id;
    LeaveCriticalSection(&lock);
    if (take) {
        if (rain_cache_log_id!=camera.id) {
            if (rain_cache_log) fclose(rain_cache_log);
            char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.rain-cache.jsonl",camera.output);
            rain_cache_log=fopen(path,"w"); rain_cache_log_id=camera.id; rain_cache_log_count=0;
        }
        int seen=0;
        for (unsigned i=0;i<rain_cache_log_count;++i) if (rain_cache_log_seen[i]==instance) seen=1;
        if (!seen && rain_cache_log && rain_cache_log_count<2048) {
            rain_cache_log_seen[rain_cache_log_count++]=instance;
            fprintf(rain_cache_log,"{\"instance\":%lu,\"parent\":%lu,\"surface\":%lu,\"factor\":%.9g,\"default\":%s,\"generation\":%u,\"updates\":%u,\"update_frame\":%u,\"current_frame\":%u,\"update_eye\":[%.9g,%.9g,%.9g],\"total_updates\":%u,\"rejected_candidates\":%u,\"overflow\":%u,\"shader_lods_off\":%u}\n",
                (unsigned long)instance,(unsigned long)parent,(unsigned long)surface,value.factor,value.updates?"false":"true",value.generation,value.updates,value.last_frame,rain_cache_update_frames,value.eye[0],value.eye[1],value.eye[2],rain_cache_updates,rain_cache_rejected,rain_cache_overflow,rain_cache_option_off);
            fflush(rain_cache_log);
        }
    }
    return value.factor;
}
static void __attribute__((naked,used)) rain_cache_constructor_bridge(void) {
    __asm__ volatile("pushfl\n\tpushal\n\tmov %esp,%ebp\n\tsub $528,%esp\n\tand $-16,%esp\n\tfxsave (%esp)\n\t"
        "pushl 28(%ebp)\n\tcall _rain_cache_construct\n\tadd $4,%esp\n\tfxrstor (%esp)\n\tmov %ebp,%esp\n\tpopal\n\tpopfl\n\t"
        "mov %cl,0x60(%eax)\n\tmov %cl,0x61(%eax)\n\tmov %cl,0x63(%eax)\n\tjmp *_rain_cache_constructor_resume\n\t");
}
static void __attribute__((naked,used)) rain_cache_visible_bridge(void) {
    __asm__ volatile("pushfl\n\tpushal\n\tmov %esp,%ebp\n\tsub $528,%esp\n\tand $-16,%esp\n\tfxsave (%esp)\n\t"
        "movzbl 44(%ebp),%eax\n\tpush %eax\n\tpushl 40(%ebp)\n\tcall _rain_cache_visible\n\tadd $8,%esp\n\t"
        "fxrstor (%esp)\n\tmov %ebp,%esp\n\tpopal\n\tpopfl\n\t"
        "push %ebp\n\tmov %esp,%ebp\n\tmov 8(%ebp),%eax\n\tjmp *_rain_cache_visible_resume\n\t");
}
static int install_factory_rain_cache(void) {
    char option[8];
    if (GetEnvironmentVariableA("SRP_DIAGNOSTIC_FACTORY_RAIN_CACHE",option,sizeof(option))!=1 || option[0]!='1') return 1;
    if (GetEnvironmentVariableA("SRP_DIAGNOSTIC_FACTORY_RAIN_CASTERS",option,sizeof(option))==1 && option[0]=='1') {
        error("choose rain cache or recomputed factor, not both"); return 0;
    }
    static const unsigned char constructor[]={0x88,0x48,0x60,0x88,0x48,0x61,0x88,0x48,0x63};
    static const unsigned char visible[]={0x55,0x8b,0xec,0x8b,0x45,0x08,0x8b,0x48,0x04};
    static const unsigned char lookup[]={0x8b,0x96,0x20,0x02,0,0,0x53,0x8b,0x5c,0x24,0x08};
    static const unsigned char shader_lods[]={0x8d,0x87,0x31,0x01,0,0,0x50,0xba,0x02,0,0,0,0x68,0x38,0x84,0xa2,0};
    if (strcmp(build,"026g") || (uintptr_t)GetModuleHandleA(NULL)!=0x10000 ||
        memcmp((void *)0x158c97,constructor,sizeof(constructor)) || memcmp((void *)0x1b7360,visible,sizeof(visible)) ||
        memcmp((void *)0x16b070,lookup,sizeof(lookup)) || memcmp((void *)0x3ce27b,shader_lods,sizeof(shader_lods))) {
        error("Factory rain cache requires pinned constructor/visibility/material/options layout"); return 0;
    }
    InitializeCriticalSection(&rain_cache_lock);
    unsigned char *sites[]={(unsigned char *)0x158c97,(unsigned char *)0x1b7360};
    void *bridges[]={rain_cache_constructor_bridge,rain_cache_visible_bridge}; SIZE_T sizes[]={9,6}; DWORD previous[2],unused;
    for (unsigned i=0;i<2;++i) if (!VirtualProtect(sites[i],sizes[i],PAGE_EXECUTE_READWRITE,&previous[i])) {
        for (unsigned j=0;j<i;++j) VirtualProtect(sites[j],sizes[j],previous[j],&unused);
        error("cannot write rain cache hooks"); return 0;
    }
    for (unsigned i=0;i<2;++i) {
        sites[i][0]=0xe9; int32_t jump=(int32_t)((intptr_t)bridges[i]-(intptr_t)(sites[i]+5));
        memcpy(sites[i]+1,&jump,4); memset(sites[i]+5,0x90,sizes[i]-5); FlushInstructionCache(GetCurrentProcess(),sites[i],sizes[i]);
        if (!VirtualProtect(sites[i],sizes[i],previous[i],&unused)) { error("cannot protect rain cache hook"); return 0; }
    }
    rain_source_cache_enabled=1;
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.factory-rain-cache.json",control); FILE *f=fopen(path,"w");
    if (f) { fputs("{\"mode\":\"diagnostic_source_factor_cache\",\"default\":1,\"constructor_reset\":true,\"updates\":\"pre_occlusion_removal_main_view_gbuffer_static_surfaces\",\"shader_lods_option_offset\":305,\"native_factor_unchanged\":true,\"unknown_material_skipped\":true,\"source_visibility_identity_proven\":false}\n",f); fclose(f); }
    return 1;
}
