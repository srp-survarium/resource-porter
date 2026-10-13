// SPDX-License-Identifier: GPL-3.0-or-later
#include <windows.h>
#include <stdio.h>
#include <string.h>
static int changed;
static const char *executable;
static BOOL CALLBACK resize(HWND window, LPARAM unused) {
    (void)unused;
    char title[256]; RECT client;
    GetWindowTextA(window,title,sizeof(title));
    if (strncmp(title,"Survarium",9) || !GetClientRect(window,&client) || client.right<400) return TRUE;
    if (executable) {
        DWORD pid, length=MAX_PATH;
        char path[MAX_PATH];
        GetWindowThreadProcessId(window,&pid);
        HANDLE process=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid);
        if (!process) return TRUE;
        BOOL found=QueryFullProcessImageNameA(process,0,path,&length);
        CloseHandle(process);
        if (!found) return TRUE;
        const char *name=strrchr(path,'\\');
        if (_stricmp(name ? name+1 : path,executable)) return TRUE;
    }
    RECT outer={0,0,1280,720};
    AdjustWindowRectEx(&outer,GetWindowLongA(window,GWL_STYLE),FALSE,GetWindowLongA(window,GWL_EXSTYLE));
    if (!SetWindowPos(window,NULL,0,0,outer.right-outer.left,outer.bottom-outer.top,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE)) return TRUE;
    printf("resized %s client from %ldx%ld to 1280x720\n",title,client.right,client.bottom);
    changed=1;
    return FALSE;
}
int main(int argc, char **argv) {
    if (argc==3 && !strcmp(argv[1],"--exe")) executable=argv[2];
    else if (argc!=1) {
        fprintf(stderr,"usage: viewport.exe [--exe executable-name]\n");
        return 2;
    }
    EnumWindows(resize,0);
    return changed ? 0 : 1;
}
