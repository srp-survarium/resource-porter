// SPDX-License-Identifier: GPL-3.0-or-later
// Send one bounded input script to exactly one owned, already focused client.
// Usage: drive_client.exe ABSOLUTE_WINDOWS_EXE_PATH "STEP;STEP;..."
//   key NAME MS   hold w|a|s|d|e|k|v|space|ctrl|shift for 50..10000 ms
//   mouse DX DY   relative raw mouse movement, |DX|,|DY| <= 4000 counts
//   click MS      hold the left mouse button for 50..3000 ms
//   wait MS       idle for 0..10000 ms
// Every key is released before exit; losing focus aborts the script.
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *executable;
static HWND target;
static unsigned matches;

static void pump(void) {
    MSG message;
    while (PeekMessageA(&message,NULL,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageA(&message); }
}

static BOOL CALLBACK find_client(HWND window, LPARAM unused) {
    (void)unused;
    char title[256], path[MAX_PATH]; RECT client; DWORD pid, length=MAX_PATH;
    GetWindowTextA(window,title,sizeof(title));
    if (strncmp(title,"Survarium",9) || !IsWindowVisible(window) ||
        !GetClientRect(window,&client) || client.right<400) return TRUE;
    GetWindowThreadProcessId(window,&pid);
    HANDLE process=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid);
    if (!process) return TRUE;
    BOOL found=QueryFullProcessImageNameA(process,0,path,&length);
    CloseHandle(process);
    if (found && !_stricmp(path,executable)) { target=window; ++matches; }
    return TRUE;
}

static int focused(void) { return IsWindow(target) && GetForegroundWindow()==target; }

static int hold_until(DWORD duration) {
    DWORD start=GetTickCount();
    while (GetTickCount()-start<duration) { pump(); if (!focused()) return 0; Sleep(5); }
    return 1;
}

static unsigned virtual_key(const char *name) {
    static const struct { const char *name; unsigned vk; } keys[]={
        {"w",'W'},{"a",'A'},{"s",'S'},{"d",'D'},{"e",'E'},{"k",'K'},{"v",'V'},
        {"space",VK_SPACE},{"ctrl",VK_LCONTROL},{"shift",VK_LSHIFT}};
    for (size_t i=0;i<sizeof(keys)/sizeof(keys[0]);++i) if (!strcmp(name,keys[i].name)) return keys[i].vk;
    return 0;
}

static int send_key(unsigned vk, int up) {
    INPUT input={0};
    input.type=INPUT_KEYBOARD;
    input.ki.wScan=(WORD)MapVirtualKeyA(vk,MAPVK_VK_TO_VSC);
    input.ki.dwFlags=KEYEVENTF_SCANCODE|(up?KEYEVENTF_KEYUP:0);
    if (vk==VK_LCONTROL || vk==VK_LSHIFT) { input.ki.wVk=(WORD)vk; input.ki.dwFlags&=~KEYEVENTF_SCANCODE; }
    return SendInput(1,&input,sizeof(input))==1;
}

static int send_button(int up) {
    INPUT input={0};
    input.type=INPUT_MOUSE; input.mi.dwFlags=up?MOUSEEVENTF_LEFTUP:MOUSEEVENTF_LEFTDOWN;
    return SendInput(1,&input,sizeof(input))==1;
}

static int move_mouse(long dx, long dy) {
    // Small relative steps; large single deltas can be clamped by the game.
    while (dx || dy) {
        long sx=dx>40?40:dx<-40?-40:dx, sy=dy>40?40:dy<-40?-40:dy;
        INPUT input={0};
        input.type=INPUT_MOUSE; input.mi.dx=sx; input.mi.dy=sy; input.mi.dwFlags=MOUSEEVENTF_MOVE;
        if (SendInput(1,&input,sizeof(input))!=1) return 0;
        dx-=sx; dy-=sy;
        if (!hold_until(8)) return 0;
    }
    return 1;
}

int main(int argc, char **argv) {
    if (argc!=3 || strlen(argv[1])<4 || argv[1][1]!=':' || argv[1][2]!='\\') {
        fprintf(stderr,"usage: drive_client.exe ABSOLUTE_WINDOWS_EXE_PATH \"key w 500;mouse 100 0;wait 200\"\n");
        return 2;
    }
    executable=argv[1]; EnumWindows(find_client,0);
    if (matches!=1) { fprintf(stderr,"expected one owned client, found %u\n",matches); return 1; }
    SetForegroundWindow(target);
    for (unsigned i=0;i<40 && !focused();++i) { pump(); Sleep(25); }
    if (!focused()) { fprintf(stderr,"client did not gain focus\n"); return 1; }
    char *script=_strdup(argv[2]); int ok=1; unsigned steps=0;
    for (char *step=strtok(script,";"); ok && step; step=strtok(NULL,";")) {
        char verb[16]={0}, name[16]={0}; long a=0, b=0;
        while (*step==' ') ++step;
        if (!*step) continue;
        if (sscanf(step,"%15s",verb)!=1) { ok=0; break; }
        if (!strcmp(verb,"key")) {
            unsigned vk;
            if (sscanf(step,"%*s %15s %ld",name,&a)!=2 || a<50 || a>10000 || !(vk=virtual_key(name))) { ok=0; break; }
            if (!send_key(vk,0)) { ok=0; break; }
            ok=hold_until((DWORD)a);
            if (!send_key(vk,1)) ok=0;
        } else if (!strcmp(verb,"mouse")) {
            if (sscanf(step,"%*s %ld %ld",&a,&b)!=2 || labs(a)>4000 || labs(b)>4000) { ok=0; break; }
            ok=move_mouse(a,b);
        } else if (!strcmp(verb,"click")) {
            if (sscanf(step,"%*s %ld",&a)!=1 || a<50 || a>3000) { ok=0; break; }
            if (!send_button(0)) { ok=0; break; }
            ok=hold_until((DWORD)a);
            if (!send_button(1)) ok=0;
        } else if (!strcmp(verb,"wait")) {
            if (sscanf(step,"%*s %ld",&a)!=1 || a<0 || a>10000) { ok=0; break; }
            ok=hold_until((DWORD)a);
        } else { ok=0; break; }
        if (ok) ++steps;
    }
    free(script);
    printf("{\"steps\":%u,\"completed\":%s}\n",steps,ok?"true":"false");
    return ok?0:1;
}
