// SPDX-License-Identifier: GPL-3.0-or-later
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *executable;
static HWND target;
static unsigned matches;

static void pump_messages(void) {
    MSG message;
    while (PeekMessageA(&message,NULL,0,0,PM_REMOVE)) {
        TranslateMessage(&message);
        DispatchMessageA(&message);
    }
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

int main(int argc, char **argv) {
    if (argc!=4 || (strcmp(argv[2],"mouse-y") && (strlen(argv[2])!=1 || !strchr("wasde",argv[2][0]))) ||
        strlen(argv[1])<4 || argv[1][1]!=':' || argv[1][2]!='\\') {
        fprintf(stderr,"usage: walk_client.exe ABSOLUTE_WINDOWS_EXE_PATH w|a|s|d|e MILLISECONDS or mouse-y PIXELS\n");
        return 2;
    }
    BOOL mouse=strcmp(argv[2],"mouse-y")==0;
    char *end; long value=strtol(argv[3],&end,10);
    if (*end || (mouse && (value < -600 || value > 600))) return 2;
    unsigned long duration=(unsigned long)value;
    if (!mouse && (duration<100 || duration>(argv[2][0]=='e' ? 10000ul : 5000ul))) return 2;
    executable=argv[1]; EnumWindows(find_client,0);
    if (matches!=1) { fprintf(stderr,"expected one owned client, found %u\n",matches); return 1; }
    unsigned vk=(unsigned)(argv[2][0]-'a'+'A');
    if (!mouse && (GetAsyncKeyState(vk)&0x8000)) { fprintf(stderr,"key already held\n"); return 1; }
    HWND previous=GetForegroundWindow();
    BOOL activated=SetForegroundWindow(target);
    for (unsigned i=0;i<40 && GetForegroundWindow()!=target;++i) { pump_messages(); Sleep(25); }
    if (GetForegroundWindow()!=target) {
        DWORD target_pid=0, foreground_pid=0;
        GetWindowThreadProcessId(target,&target_pid);
        GetWindowThreadProcessId(GetForegroundWindow(),&foreground_pid);
        char title[256]={0}; GetWindowTextA(GetForegroundWindow(),title,sizeof(title));
        fprintf(stderr,"client did not gain focus (activation=%d, target=%p pid=%lu, foreground=%p pid=%lu, helper_pid=%lu, title=%s)\n",
            activated,(void *)target,(unsigned long)target_pid,(void *)GetForegroundWindow(),(unsigned long)foreground_pid,
            (unsigned long)GetCurrentProcessId(),title);
        return 1;
    }
    INPUT input={0};
    if (mouse) {
        input.type=INPUT_MOUSE; input.mi.dy=(LONG)value; input.mi.dwFlags=MOUSEEVENTF_MOVE;
        if (SendInput(1,&input,sizeof(input))!=1) return 1;
        printf("{\"mouse_y\":%ld}\n",value); return 0;
    }
    input.type=INPUT_KEYBOARD;
    input.ki.wScan=(WORD)MapVirtualKeyA(vk,MAPVK_VK_TO_VSC);
    input.ki.dwFlags=KEYEVENTF_SCANCODE;
    if (SendInput(1,&input,sizeof(input))!=1) { fprintf(stderr,"key down failed\n"); return 1; }
    DWORD start=GetTickCount(); int completed=1;
    while (GetTickCount()-start<duration) {
        pump_messages();
        if (!IsWindow(target) || GetForegroundWindow()!=target) { completed=0; break; }
        Sleep(10);
    }
    input.ki.dwFlags|=KEYEVENTF_KEYUP;
    if (SendInput(1,&input,sizeof(input))!=1) completed=0;
    if (GetForegroundWindow()==target && IsWindow(previous)) SetForegroundWindow(previous);
    printf("{\"key\":\"%c\",\"elapsed_ms\":%lu,\"completed\":%s}\n",argv[2][0],
        (unsigned long)(GetTickCount()-start),completed?"true":"false");
    return completed ? 0 : 1;
}
