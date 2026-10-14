/* Include in the private observer DLL. Observation only; native callback always runs.
 * Call collision_observer_init(control), collision_observer_step() from its worker,
 * collision_observer_restore() at shutdown. Default off. Never unload DLL while
 * physics threads are running: restoring the slot does not cancel in-flight calls.
 */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef float (__attribute__((thiscall)) *collision_result_fn)(void *,void *,int);
#define COLLISION_SLOT ((void *volatile *)(uintptr_t)0x9e3fd8)
#define COLLISION_CAP 512u
#define COLLISION_LIMIT 8192u
struct collision_record {
    uint32_t tick,thread,callback,result,normal_world,valid;
    uint32_t ghost,controller,object,wrapper,resource,resource_root;
    uint32_t shape_part,triangle_index;
    float returned;
    unsigned char before[0xa0],after[0xa0],input[0x40],object_state[0x100];
    unsigned char controller_dims[0x10],capsule[0x44],resource_state[0x118],ghost_transform[0x40],controller_filters[4],hit_shape_state[0x44],root_shape_state[0x44];
};
_Static_assert(sizeof(struct collision_record)==1268,"collision record ABI");
static struct collision_record collision_ring[COLLISION_CAP];
static volatile LONG collision_lock,collision_seen,collision_dropped;
static unsigned collision_read,collision_write,collision_written;
static FILE *collision_file;
static int collision_owned;
static DWORD collision_last_flush;
static char collision_status[MAX_PATH+64];
static collision_result_fn collision_original=(collision_result_fn)(uintptr_t)0x895f0;
static int collision_read_memory(uintptr_t address,void *out,SIZE_T size) {
    SIZE_T count=0;
    return address && ReadProcessMemory(GetCurrentProcess(),(const void *)address,out,size,&count) && count==size;
}
static uint32_t collision_u32(const void *data,unsigned offset) {
    uint32_t value;memcpy(&value,(const unsigned char *)data+offset,4);return value;
}
static void collision_status_write(const char *state) {
    FILE *file=fopen(collision_status,"w");if (!file) return;
    fprintf(file,"{\"state\":\"%s\",\"version\":1,\"record_bytes\":%u,\"seen\":%ld,\"dropped\":%ld,\"written\":%u,\"limit\":%u}\n",
        state,(unsigned)sizeof(struct collision_record),collision_seen,collision_dropped,collision_written,COLLISION_LIMIT);
    fclose(file);
}
static float __attribute__((thiscall,noinline)) collision_observe_result(void *callback,void *input,int normal_world) {
    struct collision_record r;memset(&r,0,sizeof(r));
    r.tick=GetTickCount();r.thread=GetCurrentThreadId();r.callback=(uintptr_t)callback;
    r.result=(uintptr_t)input;r.normal_world=(unsigned)normal_world;
    LONG sequence=InterlockedIncrement(&collision_seen);
    int take=sequence<=(LONG)COLLISION_LIMIT;
    if (take && collision_read_memory((uintptr_t)callback,r.before,sizeof(r.before)) &&
        collision_read_memory((uintptr_t)input,r.input,sizeof(r.input))) {
        r.valid|=1;r.ghost=collision_u32(r.before,0x90);r.object=collision_u32(r.input,0);
        uintptr_t info=collision_u32(r.input,4);uint32_t pair[2];
        if (collision_read_memory(info,pair,sizeof(pair))) {r.valid|=2;r.shape_part=pair[0];r.triangle_index=pair[1];}
        uint32_t table=0;
        if (r.ghost>=0x70 && collision_read_memory(r.ghost-0x70,&table,4) && table==0x9e3b9c) {
            r.controller=r.ghost-0x70;
            if (collision_read_memory(r.controller+0x60,r.controller_dims,sizeof(r.controller_dims)) &&
                collision_read_memory(r.controller+0x1b0,r.capsule,sizeof(r.capsule)) &&
                collision_read_memory(r.ghost+0x10,r.ghost_transform,sizeof(r.ghost_transform)) &&
                collision_read_memory(r.controller+0x2c700,r.controller_filters,sizeof(r.controller_filters))) r.valid|=4;
        }
        if (collision_read_memory(r.object,r.object_state,sizeof(r.object_state))) {
            r.valid|=8;r.wrapper=collision_u32(r.object_state,0xf8);
            if (collision_read_memory(collision_u32(r.object_state,0xcc),r.hit_shape_state,sizeof(r.hit_shape_state))) r.valid|=128;
            if (collision_read_memory(collision_u32(r.object_state,0xd4),r.root_shape_state,sizeof(r.root_shape_state))) r.valid|=256;
            unsigned char body[0x18];
            if (collision_read_memory(r.wrapper,body,sizeof(body)) && collision_u32(body,0)==0xa1f258 && collision_u32(body,0x14)==r.object) {
                r.valid|=16;r.resource=collision_u32(body,0x10);
                if (collision_read_memory(r.resource,r.resource_state,sizeof(r.resource_state)) && collision_u32(r.resource_state,0)==0x9e31ec) {
                    r.valid|=32;r.resource_root=collision_u32(r.resource_state,0x108);
                }
            }
        }
    }
    r.returned=collision_original(callback,input,normal_world);
    if (take) {
        if (collision_read_memory((uintptr_t)callback,r.after,sizeof(r.after))) r.valid|=64;
        if (!InterlockedCompareExchange(&collision_lock,1,0)) {
            if (collision_write-collision_read<COLLISION_CAP) collision_ring[collision_write++%COLLISION_CAP]=r;
            else InterlockedIncrement(&collision_dropped);
            InterlockedExchange(&collision_lock,0);
        } else InterlockedIncrement(&collision_dropped);
    }
    return r.returned;
}
static int collision_observer_init(const char *control_path) {
    char option[8];DWORD length=GetEnvironmentVariableA("SRP_CAPTURE_COLLISION",option,sizeof(option));
    if (length!=1 || option[0]!='1') return 1;
    snprintf(collision_status,sizeof(collision_status),"%s.collision-status.json",control_path);
    const unsigned char prologue[]={0x55,0x8b,0xec,0x83,0xe4,0xf0,0x83,0xec,0x44,0x53,0x8b,0xd9};
    const uint32_t expected_table[]={0x73ef0,0x73d70,0x895f0};
    unsigned char actual[sizeof(prologue)];uint32_t table[3];
    if ((uintptr_t)GetModuleHandleA(NULL)!=0x10000 ||
        !collision_read_memory(0x895f0,actual,sizeof(actual)) || memcmp(actual,prologue,sizeof(actual)) ||
        !collision_read_memory(0x9e3fd0,table,sizeof(table)) || memcmp(table,expected_table,sizeof(table))) {
        collision_status_write("guard-rejected");return 0;
    }
    char path[MAX_PATH+64];snprintf(path,sizeof(path),"%s.collision.bin",control_path);
    collision_file=fopen(path,"wb");if (!collision_file) {collision_status_write("open-failed");return 0;}
    const uint32_t header[]={0x434f4c31,1,sizeof(struct collision_record),0x10000};
    if (fwrite(header,sizeof(header),1,collision_file)!=1) {fclose(collision_file);collision_file=NULL;return 0;}
    DWORD old;
    if (!VirtualProtect((void *)COLLISION_SLOT,4,PAGE_READWRITE,&old)) {
        fclose(collision_file);collision_file=NULL;collision_status_write("protect-failed");return 0;
    }
    void *previous=InterlockedCompareExchangePointer(COLLISION_SLOT,(void *)collision_observe_result,(void *)collision_original);
    DWORD ignored;VirtualProtect((void *)COLLISION_SLOT,4,old,&ignored);
    collision_owned=previous==(void *)collision_original;
    if (!collision_owned) {fclose(collision_file);collision_file=NULL;collision_status_write("slot-changed");return 0;}
    collision_status_write("installed");return 1;
}
static void collision_observer_step(void) {
    if (!collision_file) return;
    struct collision_record batch[16];unsigned count=0;
    if (!InterlockedCompareExchange(&collision_lock,1,0)) {
        while (collision_read!=collision_write && count<16) batch[count++]=collision_ring[collision_read++%COLLISION_CAP];
        InterlockedExchange(&collision_lock,0);
    }
    if (count) {
        collision_written+=(unsigned)fwrite(batch,sizeof(batch[0]),count,collision_file);
        if (ferror(collision_file)) {collision_status_write("write-failed");fclose(collision_file);collision_file=NULL;return;}
    }
    DWORD now=GetTickCount();
    if (now-collision_last_flush>=1000) {fflush(collision_file);collision_status_write("recording");collision_last_flush=now;}
}
static void collision_observer_restore(void) {
    if (collision_owned) {
        DWORD old;
        if (VirtualProtect((void *)COLLISION_SLOT,4,PAGE_READWRITE,&old)) {
            InterlockedCompareExchangePointer(COLLISION_SLOT,(void *)collision_original,(void *)collision_observe_result);
            DWORD ignored;VirtualProtect((void *)COLLISION_SLOT,4,old,&ignored);collision_owned=0;
        }
    }
    /* Invoke after the physics loop stops; this final flush does not wait in DllMain. */
    if (collision_file) {
        for (unsigned i=0;i<COLLISION_CAP/16;i++) collision_observer_step();
        fclose(collision_file);collision_file=NULL;collision_status_write("stopped");
    }
}
