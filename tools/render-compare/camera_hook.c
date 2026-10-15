// SPDX-License-Identifier: GPL-3.0-or-later
#define COBJMACROS
#include <windows.h>
#include <d3d11_1.h>
#include <float.h>
#include <dxgi.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

typedef struct {
    unsigned id;
    int enabled;
    float position[3], forward[3], fov, aspect, near_z, far_z;
    unsigned settle_ms;
    char output[MAX_PATH];
} Camera;

static CRITICAL_SECTION lock;
static Camera requested, applied;
static unsigned applied_id, captured_id, camera_calls, present_calls;
static unsigned buffers_id;
static unsigned capture_frame_id;
static int buffers_enabled;
static DWORD applied_at;
static unsigned image_width, image_height;
static char control[MAX_PATH], status_path[MAX_PATH+16], build[16], error_text[160];
static void *selected_director;
static unsigned char *reference_ui_enabled;
void *camera_trampoline;
static HRESULT (STDMETHODCALLTYPE *original_present)(IDXGISwapChain *, UINT, UINT);
static int install_shader_capture(ID3D11Device *device, ID3D11DeviceContext *context);

static void error(const char *message) {
    EnterCriticalSection(&lock);
    snprintf(error_text, sizeof(error_text), "%s", message);
    LeaveCriticalSection(&lock);
}

static void matrices(const Camera *c, float *view, float *projection) {
    float x = c->forward[0], y = c->forward[1], z = c->forward[2];
    float n = sqrtf(x*x + y*y + z*z);
    x /= n; y /= n; z /= n;
    float horizontal = sqrtf(x*x + z*z);
    float rx = z / horizontal, rz = -x / horizontal;
    const float rows[16] = {
        rx, 0, rz, 0,
        y*rz, z*rx-x*rz, -y*rx, 0,
        x, y, z, 0,
        c->position[0], c->position[1], c->position[2], 1
    };
    memcpy(view, rows, sizeof(rows));
    memset(projection, 0, 16*sizeof(float));
    float cotangent = 1.0f/tanf(c->fov*0.017453292519943295f/2.0f);
    projection[0] = cotangent/c->aspect;
    projection[5] = cotangent;
    projection[10] = c->far_z/(c->far_z-c->near_z);
    projection[11] = 1;
    projection[14] = -c->near_z*projection[10];
}

// The verified retail implementations receive camera_director in EDX.
// Override before base_game_scene copies the matrices and queues rendering.
void __cdecl override_camera(uint32_t *saved) {
    void *director = (void *)(uintptr_t)saved[!strcmp(build,"020e") ? 7 : 5];
    EnterCriticalSection(&lock);
    if (!selected_director) selected_director = director;
    if (director == selected_director) {
        ++camera_calls;
        if (requested.enabled==1 || requested.enabled==2) {
            float view[16], projection[16];
            matrices(&requested, view, projection);
            memcpy((char *)director + 4, view, sizeof(view));
            memcpy((char *)director + 0x44, projection, sizeof(projection));
            if (applied_id != requested.id) {
                applied_at = GetTickCount();
                applied_id = requested.id;
            }
            applied = requested;
        } else {
            if (applied_id != requested.id) applied_at=GetTickCount();
            applied=requested;
            applied_id=requested.id;
            float *view=(float *)((char *)director+4);
            float *projection=(float *)((char *)director+0x44);
            memcpy(applied.position,view+12,3*sizeof(float));
            memcpy(applied.forward,view+8,3*sizeof(float));
            applied.fov=2.0f*atanf(1.0f/projection[5])*57.295779513f;
        }
    }
    LeaveCriticalSection(&lock);
}

__attribute__((naked)) static void camera_entry(void) {
    __asm__ volatile(
        "pushfl\n\tpushal\n\tmovl %esp, %ebp\n\t"
        "subl $528, %esp\n\tandl $-16, %esp\n\tfxsave (%esp)\n\t"
        "pushl %ebp\n\tcall _override_camera\n\taddl $4, %esp\n\t"
        "fxrstor (%esp)\n\tmovl %ebp, %esp\n\tpopal\n\tpopfl\n\t"
        "jmp *_camera_trampoline\n\t");
}

static int install_camera(void) {
    uintptr_t rva;
    if (!strcmp(build, "010b")) rva = 0x5c72b0;
    else if (!strcmp(build, "011a")) rva = 0x5c6050;
    else if (!strcmp(build, "011e")) rva = 0x5c42c0;
    else if (!strcmp(build, "026g")) rva = 0x22ddd0;
    else if (!strcmp(build, "020e")) {
        rva = 0x16312c;
        unsigned char *base = (unsigned char *)GetModuleHandleW(NULL);
        unsigned char expected[] = {0x80,0x3d,0,0,0,0,0};
        uint32_t address = (uint32_t)(uintptr_t)(base+0x86fc06);
        memcpy(expected+2,&address,4);
        if (memcmp(base+0xda105,expected,sizeof(expected))) {
            error("0.20e UI flag instruction does not match pinned executable"); return 0;
        }
        reference_ui_enabled = base+0x86fc06;
    }
    else { error("unknown build"); return 0; }
    unsigned char *entry = (unsigned char *)GetModuleHandleW(NULL) + rva;
    const unsigned char standard[8] = {0x83,0xec,0x40,0x53,0x8b,0x5c,0x24,0x48};
    const unsigned char v020e[6] = {0x55,0x8b,0xec,0x83,0xec,0x48};
    const unsigned char *expected = !strcmp(build,"020e") ? v020e : standard;
    size_t length = !strcmp(build,"020e") ? sizeof(v020e) : sizeof(standard);
    if (memcmp(entry, expected, length)) {
        error("camera hook bytes do not match the pinned executable"); return 0;
    }
    unsigned char *trampoline = VirtualAlloc(NULL, 32, MEM_COMMIT|MEM_RESERVE, PAGE_EXECUTE_READWRITE);
    if (!trampoline) { error("cannot allocate camera trampoline"); return 0; }
    memcpy(trampoline, expected, length);
    trampoline[length] = 0xe9;
    *(int32_t *)(trampoline+length+1) = (int32_t)(entry+length-(trampoline+length+5));
    camera_trampoline = trampoline;
    DWORD protection;
    if (!VirtualProtect(entry, length, PAGE_EXECUTE_READWRITE, &protection)) {
        error("cannot write camera hook"); return 0;
    }
    entry[0] = 0xe9;
    *(int32_t *)(entry+1) = (int32_t)((unsigned char *)camera_entry-(entry+5));
    memset(entry+5, 0x90, length-5);
    FlushInstructionCache(GetCurrentProcess(), entry, length);
    VirtualProtect(entry, length, protection, &protection);
    return 1;
}

static int capture(IDXGISwapChain *swapchain, const Camera *camera) {
    ID3D11Texture2D *buffer = NULL, *staging = NULL;
    ID3D11Device *device = NULL;
    ID3D11DeviceContext *context = NULL;
    D3D11_TEXTURE2D_DESC desc;
    D3D11_MAPPED_SUBRESOURCE mapped;
    int result = 0;
    if (FAILED(IDXGISwapChain_GetBuffer(swapchain, 0, &IID_ID3D11Texture2D, (void **)&buffer))) goto done;
    ID3D11Texture2D_GetDesc(buffer, &desc);
    if (desc.SampleDesc.Count != 1 || (desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM &&
        desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM && desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM_SRGB &&
        desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM_SRGB)) {
        error("unsupported screenshot backbuffer format"); goto done;
    }
    ID3D11Texture2D_GetDevice(buffer, &device);
    ID3D11Device_GetImmediateContext(device, &context);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    desc.MiscFlags = 0;
    if (FAILED(ID3D11Device_CreateTexture2D(device, &desc, NULL, &staging))) goto done;
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)buffer);
    if (FAILED(ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &mapped))) goto done;
    FILE *file = fopen(camera->output, "wb");
    if (file) {
        BITMAPFILEHEADER header = {0};
        BITMAPINFOHEADER info = {0};
        header.bfType = 0x4d42;
        header.bfOffBits = sizeof(header)+sizeof(info);
        header.bfSize = header.bfOffBits+desc.Width*desc.Height*4;
        info.biSize = sizeof(info);
        info.biWidth = desc.Width;
        info.biHeight = -(LONG)desc.Height;
        info.biPlanes = 1;
        info.biBitCount = 32;
        info.biSizeImage = desc.Width*desc.Height*4;
        result = fwrite(&header, sizeof(header), 1, file)==1 && fwrite(&info, sizeof(info), 1, file)==1;
        unsigned char *row = malloc(desc.Width*4);
        if (!row) result = 0;
        for (unsigned y=0; result && y<desc.Height; ++y) {
            memcpy(row, (char *)mapped.pData+y*mapped.RowPitch, desc.Width*4);
            if (desc.Format==DXGI_FORMAT_R8G8B8A8_UNORM || desc.Format==DXGI_FORMAT_R8G8B8A8_UNORM_SRGB)
                for (unsigned x=0; x<desc.Width; ++x) { unsigned char r=row[x*4]; row[x*4]=row[x*4+2]; row[x*4+2]=r; }
            result = fwrite(row, desc.Width*4, 1, file)==1;
        }
        free(row);
        if (fclose(file)) result = 0;
    }
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, 0);
    if (result) {
        EnterCriticalSection(&lock);
        image_width=desc.Width; image_height=desc.Height;
        LeaveCriticalSection(&lock);
    }
done:
    if (context) ID3D11DeviceContext_Release(context);
    if (device) ID3D11Device_Release(device);
    if (staging) ID3D11Texture2D_Release(staging);
    if (buffer) ID3D11Texture2D_Release(buffer);
    return result;
}

static HRESULT STDMETHODCALLTYPE present(IDXGISwapChain *swapchain, UINT interval, UINT flags) {
    Camera camera;
    int take = 0;
    EnterCriticalSection(&lock);
    ++present_calls;
    // Arm GPU readbacks at a frame boundary after settling. Otherwise a request
    // can miss geometry draws yet capture lighting later in that same frame.
    if (!(flags & DXGI_PRESENT_TEST) && buffers_enabled && applied.enabled==1 &&
        applied_id!=captured_id && GetTickCount()-applied_at>=applied.settle_ms)
        capture_frame_id=applied_id;
    if (!(flags & DXGI_PRESENT_TEST) && applied.enabled && applied_id != captured_id &&
        (!buffers_enabled || applied.enabled==2 || applied.enabled==3 || buffers_id==applied_id) &&
        GetTickCount()-applied_at >= applied.settle_ms) { camera=applied; take=1; }
    LeaveCriticalSection(&lock);
    if (take && capture(swapchain, &camera)) {
        EnterCriticalSection(&lock); captured_id=camera.id; LeaveCriticalSection(&lock);
    }
    return original_present(swapchain, interval, flags);
}

static int install_present(void) {
    HWND window = CreateWindowExA(0, "STATIC", "SRP capture probe", WS_POPUP, 0,0,64,64,NULL,NULL,GetModuleHandleW(NULL),NULL);
    DXGI_SWAP_CHAIN_DESC desc = {0};
    desc.BufferDesc.Width=64; desc.BufferDesc.Height=64;
    desc.BufferDesc.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count=1; desc.BufferUsage=DXGI_USAGE_RENDER_TARGET_OUTPUT;
    desc.BufferCount=1; desc.OutputWindow=window; desc.Windowed=TRUE;
    IDXGISwapChain *swapchain=NULL;
    ID3D11Device *device=NULL;
    ID3D11DeviceContext *context=NULL;
    HRESULT result=D3D11CreateDeviceAndSwapChain(NULL,D3D_DRIVER_TYPE_HARDWARE,NULL,0,NULL,0,D3D11_SDK_VERSION,&desc,&swapchain,&device,NULL,&context);
    if (FAILED(result)) { DestroyWindow(window); error("cannot obtain DXGI Present vtable"); return 0; }
    void **vtable=*(void ***)swapchain;
    original_present=(void *)vtable[8];
    DWORD protection;
    int ok=VirtualProtect(&vtable[8],sizeof(void *),PAGE_READWRITE,&protection);
    if (ok) { vtable[8]=(void *)present; VirtualProtect(&vtable[8],sizeof(void *),protection,&protection); }
    // Original 0.11e references are screenshot-only: no shader observers or replacements.
    if (ok && strcmp(build,"011e")) ok=install_shader_capture(device,context);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    IDXGISwapChain_Release(swapchain);
    DestroyWindow(window);
    if (!ok) error("cannot install Present observer");
    return ok;
}

static void read_request(void) {
    FILE *file=fopen(control,"r");
    if (!file) return;
    Camera camera={0};
    int count=fscanf(file,"%u %d %f %f %f %f %f %f %f %f %f %f %u\n",
        &camera.id,&camera.enabled,&camera.position[0],&camera.position[1],&camera.position[2],
        &camera.forward[0],&camera.forward[1],&camera.forward[2],&camera.fov,&camera.aspect,
        &camera.near_z,&camera.far_z,&camera.settle_ms);
    char *line=count==13 ? fgets(camera.output,sizeof(camera.output),file) : NULL;
    fclose(file);
    if (!line) return;
    camera.output[strcspn(camera.output,"\r\n")]=0;
    float horizontal=camera.forward[0]*camera.forward[0]+camera.forward[2]*camera.forward[2];
    if (camera.enabled<0 || camera.enabled>3 || !camera.id || !camera.output[0] || !isfinite(horizontal) || horizontal<1e-8f ||
        !isfinite(camera.aspect) || !isfinite(camera.near_z) || !isfinite(camera.far_z) ||
        !(camera.fov>1 && camera.fov<170 && camera.aspect>0 && camera.near_z>0 && camera.far_z>camera.near_z)) return;
    for (int i=0;i<3;++i) if (!isfinite(camera.position[i]) || !isfinite(camera.forward[i])) return;
    EnterCriticalSection(&lock);
    if (requested.id != camera.id) selected_director=NULL;
    requested=camera;
    if (reference_ui_enabled && camera.enabled) *reference_ui_enabled=0;
    LeaveCriticalSection(&lock);
}

static void write_status(void) {
    char temporary[MAX_PATH+24];
    snprintf(temporary,sizeof(temporary),"%s.tmp",status_path);
    FILE *file=fopen(temporary,"w");
    if (!file) return;
    EnterCriticalSection(&lock);
    fprintf(file,"{\"build\":\"%s\",\"applied_id\":%u,\"captured_id\":%u,\"camera_calls\":%u,\"present_calls\":%u,\"width\":%u,\"height\":%u,\"position\":[%.9g,%.9g,%.9g],\"forward\":[%.9g,%.9g,%.9g],\"fov\":%.9g,\"error\":\"%s\"}\n",
        build,applied_id,captured_id,camera_calls,present_calls,image_width,image_height,
        applied.position[0],applied.position[1],applied.position[2],applied.forward[0],applied.forward[1],applied.forward[2],applied.fov,error_text);
    LeaveCriticalSection(&lock);
    fclose(file);
    MoveFileExA(temporary,status_path,MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH);
}

#include "frame_state_diagnostic.c"
#include "background_render.c"
#include "probe_sort.c"
#include "factory_rain_shadow.c"
#include "factory_020e_ambient.c"
#include "factory_020e_clouds.c"
#include "factory_rain_casters.c"
#include "factory_rain_cache.c"
#include "factory_rain_selection.c"
#include "step_diagnostic.c"

static DWORD WINAPI worker(void *unused) {
    (void)unused;
    unsigned tick=0;
    int frames=GetEnvironmentVariableA("SRP_CAPTURE_FRAME_STATE",NULL,0)!=0;
    for (;;) {
        read_request(); capture_background_step(); write_status();
        if (frames && !(tick++%10)) { diagnostic_frame_state(); probe_sort_status(); }
        Sleep(100);
    }
    return 0;
}

#include "render_buffers.c"
#include "shader_capture.c"

__declspec(dllexport) void __cdecl SRPCaptureInitialize(void) {
    GetEnvironmentVariableA("SRP_CAPTURE_BUILD",build,sizeof(build));
    GetEnvironmentVariableA("SRP_CAPTURE_CONTROL",control,sizeof(control));
    snprintf(status_path,sizeof(status_path),"%s.status.json",control);
    int ok;
    if (!strcmp(build,"011e")) {
        ok=control[0] && install_camera() && install_present();
    } else {
        ok=control[0] && configure_capture_background() && install_factory_020e_ambient() && install_factory_020e_clouds() && install_legacy_probe_sort() && install_factory_rain_shadow() && install_factory_rain_cache() && install_factory_rain_casters() && install_factory_rain_selection() && install_camera() && install_present() && install_post();
    }
    write_status();
    if (!ok) ExitProcess(1);
    install_step_diagnostic(); // opt-in; never stops the game
    HANDLE thread=CreateThread(NULL,0,worker,NULL,0,NULL);
    if (!thread) ExitProcess(1);
    CloseHandle(thread);
}

BOOL WINAPI DllMain(HINSTANCE instance,DWORD reason,void *reserved) {
    (void)reserved;
    if (reason==DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(instance);
        InitializeCriticalSection(&lock);
    }
    if (reason==DLL_PROCESS_DETACH) capture_background_restore();
    return TRUE;
}
