/* Proxy for the existing camera import. The frozen renderer DLL is loaded unchanged. */
#include "collision_observer.c"
static HMODULE proxy_module;
static char proxy_stop[MAX_PATH+64];
static DWORD WINAPI collision_worker(void *unused) {
    (void)unused;
    for (;;) {
        collision_observer_step();
        if (GetFileAttributesA(proxy_stop)!=INVALID_FILE_ATTRIBUTES) {
            collision_observer_restore();return 0;
        }
        Sleep(10);
    }
}
__declspec(dllexport) void __cdecl SRPCaptureInitialize(void) {
    char library[MAX_PATH],control_path[MAX_PATH];
    DWORD n=GetModuleFileNameA(proxy_module,library,sizeof(library));
    if (!n || n>=sizeof(library)) ExitProcess(74);
    char *name=strrchr(library,'\\');
    if (!name || (size_t)(name-library)+sizeof("\\camera_hook_render.dll")>sizeof(library)) ExitProcess(74);
    strcpy(name,"\\camera_hook_render.dll");
    HMODULE renderer=LoadLibraryA(library);
    if (!renderer) ExitProcess(74);
    void (__cdecl *initialize)(void)=(void (__cdecl *)(void))GetProcAddress(renderer,"SRPCaptureInitialize");
    if (!initialize) ExitProcess(74);
    initialize();
    n=GetEnvironmentVariableA("SRP_CAPTURE_CONTROL",control_path,sizeof(control_path));
    if (!n || n>=sizeof(control_path) || !collision_observer_init(control_path)) ExitProcess(74);
    if (!collision_owned) return;
    snprintf(proxy_stop,sizeof(proxy_stop),"%s.collision-stop",control_path);
    HANDLE thread=CreateThread(NULL,0,collision_worker,NULL,0,NULL);
    if (!thread) {collision_observer_restore();ExitProcess(74);}
    CloseHandle(thread);
}
BOOL WINAPI DllMain(HINSTANCE module,DWORD reason,void *reserved) {
    (void)reserved;
    if (reason==DLL_PROCESS_ATTACH) {proxy_module=module;DisableThreadLibraryCalls(module);}
    /* Process teardown stops physics and worker threads. No CRT IO under loader lock.
     * Do not FreeLibrary this proxy while the process is running. */
    return TRUE;
}
