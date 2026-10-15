// SPDX-License-Identifier: GPL-3.0-or-later
// Console test of step_diagnostic.c under Wine (no game is started):
//   test_step_diagnostic.exe <survarium-capture.exe> <log directory>
// - install_step_diagnostic() on this test executable fails closed: it logs
//   installed 0 and patches nothing;
// - the stubs run between a fake on_step with the real frame layout and
//   calling convention and the real get_pair, copied from the 026g
//   executable (it is register-relative code): registers, the stack, xmm1
//   and get_pair's result must come back unchanged, and the writer's own
//   reading of the table must equal what get_pair returned, also for
//   surfaces with walker-behaviour bits;
// - unreadable pointers, a bogus manager and queue overflow do not fault.
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char control[MAX_PATH], build[16];
#include "step_diagnostic.c"

void *test_entry_target, *test_pair_target;
uint32_t test_this=0x7e570000, test_manager, test_surface, test_pair_result, test_bad;
uint32_t test_seen_position, test_seen_this, test_seen_a1, test_seen_actor, test_seen_a3;
uint32_t test_saved_esp, test_onstep_esp;

int __cdecl test_call_step(const float *position, uint32_t a1, uint32_t actor, uint32_t a3);
void test_on_step(void);

__asm__(
    ".text\n"
    // Caller like the two retail call sites: eax = position, ecx = this, three pushed arguments.
    "_test_call_step:\n\t"
    "pushl %ebp\n\tmovl %esp, %ebp\n\tpushl %ebx\n\tpushl %esi\n\tpushl %edi\n\t"
    "movl $0x11111111, %ebx\n\tmovl $0x22222222, %esi\n\tmovl $0x33333333, %edi\n\t"
    "movl %esp, _test_saved_esp\n\t"
    "pushl 20(%ebp)\n\tpushl 16(%ebp)\n\tpushl 12(%ebp)\n\tmovl 8(%ebp), %eax\n\tmovl _test_this, %ecx\n\t"
    "call *_test_entry_target\n\t"
    "xorl %eax, %eax\n\t"
    "cmpl _test_saved_esp, %esp\n\tjne 1f\n\t"
    "cmpl $0x11111111, %ebx\n\tjne 1f\n\tcmpl $0x22222222, %esi\n\tjne 1f\n\tcmpl $0x33333333, %edi\n\tjne 1f\n\t"
    "movl $1, %eax\n"
    "1:\n\tleal -12(%ebp), %esp\n\tpopl %edi\n\tpopl %esi\n\tpopl %ebx\n\tpopl %ebp\n\tret\n"
    // on_step's frame: push ebp; mov ebp,esp; and esp,-8; sub esp,0x4c; push ebx/esi/edi;
    // the hit object at [esp+0x4c]; get_pair(ecx manager, edx actor = [ebp+0xc], di surface); ret 0xc.
    "_test_on_step:\n\t"
    "pushl %ebp\n\tmovl %esp, %ebp\n\tandl $-8, %esp\n\tsubl $0x4c, %esp\n\tpushl %ebx\n\tpushl %esi\n\tpushl %edi\n\t"
    "movl %eax, _test_seen_position\n\tmovl %ecx, _test_seen_this\n\t"
    "movl 8(%ebp), %eax\n\tmovl %eax, _test_seen_a1\n\tmovl 12(%ebp), %eax\n\tmovl %eax, _test_seen_actor\n\t"
    "movl 16(%ebp), %eax\n\tmovl %eax, _test_seen_a3\n\t"
    "movl $0x5555aaa0, 0x4c(%esp)\n\tmovl $0x3f800000, 0x50(%esp)\n\t"
    "movl $0x40490fdb, %eax\n\tmovd %eax, %xmm1\n\t"
    "movl _test_surface, %edi\n\tmovl 0xc(%ebp), %edx\n\tmovl _test_manager, %ecx\n\t"
    "movl $0x66666666, %ebx\n\tmovl $0x77777777, %esi\n\t"
    "movl %esp, _test_onstep_esp\n\t"
    "call *_test_pair_target\n\t"
    "movl %eax, _test_pair_result\n\t"
    "cmpl _test_onstep_esp, %esp\n\tje 2f\n\torl $1, _test_bad\n"
    "2:\n\tcmpl $0x66666666, %ebx\n\tje 3f\n\torl $2, _test_bad\n"
    "3:\n\tcmpl $0x77777777, %esi\n\tje 4f\n\torl $4, _test_bad\n"
    "4:\n\tcmpl _test_surface, %edi\n\tje 5f\n\torl $8, _test_bad\n"
    "5:\n\tcmpl _test_manager, %ecx\n\tje 6f\n\torl $16, _test_bad\n"
    "6:\n\tmovd %xmm1, %eax\n\tcmpl $0x40490fdb, %eax\n\tje 7f\n\torl $32, _test_bad\n"
    "7:\n\tcmpl $0x5555aaa0, 0x4c(%esp)\n\tje 8f\n\torl $64, _test_bad\n"
    "8:\n\tpopl %edi\n\tpopl %esi\n\tpopl %ebx\n\tmovl %ebp, %esp\n\tpopl %ebp\n\tret $0xc\n");

#define CHECK(c) do { if (!(c)) { printf("FAIL line %d: %s\n", __LINE__, #c); return 1; } } while (0)

static unsigned char *manager_memory;
static uint32_t pointer(const void *p) { return (uint32_t)(uintptr_t)p; }

// The same resolution as get_pair, written separately for the test.
static uint32_t reference_pair(unsigned actor, unsigned surface) {
    unsigned v[2]={actor & 0xffff, surface & 0xffff}, r[2];
    for (int i=0; i<2; ++i) {
        uint32_t entry; memcpy(&entry,manager_memory+0x108+4*v[i],4);
        uint16_t base=0xffff, fallback; memcpy(&fallback,manager_memory+0x1030c,2);
        if (entry) memcpy(&base,(void *)(uintptr_t)(entry+0x66),2);
        r[i]=entry ? (base==0xffff ? v[i] : base) : fallback;
    }
    uint32_t slot; memcpy(&slot,manager_memory+0x308+4*(r[0]*128+r[1]),4);
    if (!slot) { uint16_t d; memcpy(&d,manager_memory+0x1030c,2); r[1]=d; }
    memcpy(&slot,manager_memory+0x308+4*(r[0]*128+r[1]),4);
    return slot;
}

int main(int argc, char **argv) {
    if (argc!=3) { printf("usage: test_step_diagnostic <survarium-capture.exe> <log directory>\n"); return 2; }
    // The real get_pair: VA 0x306d50, .text at VA 0x11000 / file offset 0x400, 0xa9 bytes.
    unsigned char code[0xa9];
    FILE *exe=fopen(argv[1],"rb");
    CHECK(exe && !fseek(exe,0x306d50-0x11000+0x400,SEEK_SET) && fread(code,1,sizeof(code),exe)==sizeof(code));
    fclose(exe);
    CHECK(!memcmp(code,step_pins[6].bytes,step_pins[6].length) && code[0xa8]==0xc3);
    unsigned char *get_pair=VirtualAlloc(NULL,4096,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE);
    CHECK(get_pair); memcpy(get_pair,code,sizeof(code));

    snprintf(control,sizeof(control),"%s\\camera.txt",argv[2]);
    snprintf(build,sizeof(build),"026g");
    char log_path[MAX_PATH+16]; snprintf(log_path,sizeof(log_path),"%s\\step.log",argv[2]);
    DeleteFileA(log_path);

    // Disabled unless SRP_STEP_LOG=1: no file, nothing changes.
    SetEnvironmentVariableA("SRP_STEP_LOG","0");
    install_step_diagnostic();
    CHECK(step_file==INVALID_HANDLE_VALUE && GetFileAttributesA(log_path)==INVALID_FILE_ATTRIBUTES);
    // Enabled on an executable that is not the pinned one: fails closed, logs why.
    SetEnvironmentVariableA("SRP_STEP_LOG","1");
    install_step_diagnostic();
    CHECK(step_file!=INVALID_HANDLE_VALUE && !step_on_step && !step_get_pair);

    // Guarded reads.
    uint32_t value;
    CHECK(!step_read(0,&value,4) && !step_read(0x10,&value,4) && !step_read(0xfffffffe,&value,4));
    void *freed=VirtualAlloc(NULL,4096,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    VirtualFree(freed,0,MEM_RELEASE);
    CHECK(!step_read((uintptr_t)freed,&value,4));
    void *guard=VirtualAlloc(NULL,4096,MEM_COMMIT|MEM_RESERVE,PAGE_NOACCESS);
    CHECK(!step_read((uintptr_t)guard,&value,4));

    // A manager laid out like the game's: table at +0x108, pairs at +0x308, default at +0x1030c.
    manager_memory=calloc(1,0x50000);
    static unsigned char materials[16][0x6c];
    static unsigned char pairs[8][0x40];
    static const char *names[16]={"metall_thick","ground","default"};
    for (int i=0; i<16; ++i) { uint16_t base=0xffff; memcpy(materials[i]+0x66,&base,2); }
    uint16_t base3=3; memcpy(materials[7]+0x66,&base3,2);          // material 7 uses base 3
    const char *name=names[0]; uint32_t name_pointer=pointer(name); memcpy(materials[5]+0x10,&name_pointer,4);
    int ids[]={2,3,5,7,11};
    for (unsigned i=0; i<sizeof(ids)/sizeof(ids[0]); ++i) { uint32_t p=pointer(materials[ids[i]]); memcpy(manager_memory+0x108+4*ids[i],&p,4); }
    uint16_t fallback=2; memcpy(manager_memory+0x1030c,&fallback,2);
    uint32_t sound=pointer(pairs[7]);
    for (int i=0; i<4; ++i) memcpy(pairs[i]+0x20,&sound,4);
    uint32_t p0=pointer(pairs[0]), p1=pointer(pairs[1]), p2=pointer(pairs[2]);
    memcpy(manager_memory+0x308+4*(11*128+5),&p0,4);   // actor 11 on 5
    memcpy(manager_memory+0x308+4*(11*128+3),&p1,4);   // actor 11 on base 3
    memcpy(manager_memory+0x308+4*(11*128+2),&p2,4);   // actor 11 on the default
    test_manager=pointer(manager_memory);

    InitializeCriticalSection(&step_lock);
    step_on_step=(uintptr_t)test_on_step; step_get_pair=(uintptr_t)get_pair;
    test_entry_target=(void *)step_entry; test_pair_target=(void *)step_pair;
    HANDLE writer=CreateThread(NULL,0,step_writer,NULL,0,NULL);
    CHECK(writer);

    const unsigned surfaces[]={5,7,9,3,0x4005,0x8007,0xc000|2,0xffff,0x7f};
    const unsigned count=sizeof(surfaces)/sizeof(surfaces[0]);
    for (unsigned i=0; i<count; ++i) {
        float position[3]={1.5f+i,10.25f,-3.0f};
        test_surface=surfaces[i]; test_bad=0; test_pair_result=0xdeadbeef;
        CHECK(test_call_step(position,0xa1a1a1a1,11,1));
        CHECK(!test_bad);
        CHECK(test_seen_position==pointer(position) && test_seen_this==test_this && test_seen_a1==0xa1a1a1a1 && test_seen_actor==11 && test_seen_a3==1);
        CHECK(test_pair_result==reference_pair(11,surfaces[i]));
    }
    // A bogus manager and position: logged with reads_ok 0, no fault.
    step_log_pair(0x12345678,11,5,0x9abcdef0,0x10);
    step_log_begin(0x20,0,11,0);
    // Overflow: records are dropped and counted.
    for (int i=0; i<4*STEP_RING; ++i) step_log_begin(0,0,11,0);
    Sleep(1500);

    FILE *log=fopen(log_path,"rb");
    CHECK(log);
    char line[4096]; unsigned installed0=0, steps=0, pair_lines=0, matched=0, bogus=0, dropped=0, material_named=0;
    while (fgets(line,sizeof(line),log)) {
        if (strstr(line,"\"installed\":0")) ++installed0;
        if (strstr(line,"\"installed\":1")) { printf("FAIL: installed on the test executable\n"); return 1; }
        if (!strncmp(line,"{\"step\":",8)) ++steps;
        if (!strncmp(line,"{\"pair\":",8)) {
            ++pair_lines;
            char *a=strstr(line,"\"pair_ptr\":\""), *b=strstr(line,"\"pair_expected\":\"");
            if (a && b && !strncmp(a+12,b+17,10)) ++matched;
            if (strstr(line,"\"reads_ok\":0")) ++bogus;
            if (pair_lines<=3) printf("%.400s...\n",line);
        }
        if (strstr(line,"\"dropped\":")) ++dropped;
        if (strstr(line,"\"material\":") && strstr(line,"\"0x10\":\"metall_thick\"")) ++material_named;
    }
    fclose(log);
    printf("installed0=%u steps=%u pairs=%u matched=%u bogus=%u dropped_lines=%u named=%u\n",installed0,steps,pair_lines,matched,bogus,dropped,material_named);
    CHECK(installed0==1 && pair_lines==count+1 && matched==count && bogus==1 && dropped>=1 && material_named==1);
    CHECK(steps>=count+1 && steps<=count+1+STEP_RING);
    printf("PASS\n");
    return 0;
}
