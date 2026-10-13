// SPDX-License-Identifier: GPL-3.0-or-later
#include <windows.h>
#include <stdio.h>
#include <string.h>
static int changed;
static BOOL CALLBACK resize(HWND window, LPARAM unused) {
    (void)unused;
    char title[256]; RECT client;
    GetWindowTextA(window,title,sizeof(title));
    if (strncmp(title,"Survarium",9) || !GetClientRect(window,&client) || client.right<400) return TRUE;
    RECT outer={0,0,1280,720};
    AdjustWindowRectEx(&outer,GetWindowLongA(window,GWL_STYLE),FALSE,GetWindowLongA(window,GWL_EXSTYLE));
    if (!SetWindowPos(window,NULL,0,0,outer.right-outer.left,outer.bottom-outer.top,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE)) return TRUE;
    printf("resized %s client from %ldx%ld to 1280x720\n",title,client.right,client.bottom);
    changed=1;
    return FALSE;
}
int main(void) { EnumWindows(resize,0); return changed ? 0 : 1; }
