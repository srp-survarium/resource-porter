/* Include in camera_hook.c after its Windows/stdio headers. Call from its
   existing observer worker; never from a suspended game thread. Native 0.26
   addresses are guarded. This only reads memory and appends observer output. */
static BOOL frame_read(DWORD address, void *out, SIZE_T bytes) {
    SIZE_T got=0;
    return ReadProcessMemory(GetCurrentProcess(),(const void *)(uintptr_t)address,out,bytes,&got)&&got==bytes;
}
static DWORD frame_word(const unsigned char *p,unsigned off) {
    DWORD value; memcpy(&value,p+off,4); return value;
}
static void diagnostic_frame_state(void) {
    const DWORD engine=0x00a7b6b8;
    DWORD vtable=0; unsigned char state[0x70],facade[0x180];
    if((uintptr_t)GetModuleHandleA(NULL)!=0x10000 || !frame_read(engine,&vtable,4)||vtable!=0x009db60c) return;
    if(!frame_read(engine+0x2b8,state,sizeof(state)))return;
    DWORD world=frame_word(state,0),render=0,render_frame=0,head_next=0,tail_next=0;
    if(!world||!frame_read(world,facade,sizeof(facade)))return;
    render=frame_word(facade,0x160);
    BOOL render_ok=render&&frame_read(render+0x28,&render_frame,4);
    DWORD producer_head=frame_word(facade,4),consumer_tail=frame_word(facade,0x40);
    BOOL head_ok=producer_head&&frame_read(producer_head+8,&head_next,4);
    BOOL tail_ok=consumer_tail&&frame_read(consumer_tail+8,&tail_next,4);
    char path[MAX_PATH+32];snprintf(path,sizeof(path),"%s.frames.txt",control);
    FILE *f=fopen(path,"a");if(!f)return;
    fprintf(f,"ms=%lu engine=%08lx facade=%08lx logic=%lu render=%08lx render_frame=%lu render_ok=%d channel_frame=%lu logic_enabled=%lu logic_ended=%lu editor_ended=%u is_editor=%u game_enabled=%u last_enabled=%u destruction=%lu producer_head=%08lx producer_next=%08lx producer_ok=%d consumer_tail=%08lx consumer_next=%08lx consumer_ok=%d deferred_count=%lu deferred_first=%08lx process_next=%u\n",
      GetTickCount(),engine,world,frame_word(state,0x34),render,render_frame,render_ok,
      frame_word(facade,0xa8),frame_word(facade,0x170),frame_word(facade,0x174),facade[0x178],facade[0x179],
      state[0x56],state[0x57],frame_word(state,0x40),producer_head,head_next,head_ok,consumer_tail,tail_next,tail_ok,
      frame_word(facade,0x90),frame_word(facade,0x98),facade[0xac]);
    fclose(f);
}
