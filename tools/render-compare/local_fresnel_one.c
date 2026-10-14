// Optional source F0=1 diffuse exception. All shadow/specular instructions stay native.
#include "local_fresnel_one_index.h"
#define LOCAL_FRESNEL_COUNT (sizeof(local_fresnel_one_shaders)/sizeof(local_fresnel_one_shaders[0]))
static void *local_fresnel_one_code[LOCAL_FRESNEL_COUNT];
static unsigned local_fresnel_one_created[LOCAL_FRESNEL_COUNT];
static const GUID local_fresnel_one_guid={0xd799611e,0x4738,0x437d,{0x94,0x72,0x35,0x1d,0xfb,0x91,0x41,0xe2}};
static int local_fresnel_one_index(const void *code,SIZE_T size,int fixed) {
    if (size<32) return -1;
    for (unsigned i=0;i<LOCAL_FRESNEL_COUNT;++i) {
        if (size==(fixed?local_fresnel_one_shaders[i].fixed_size:local_fresnel_one_shaders[i].size) &&
            !memcmp((const char *)code+4,fixed?local_fresnel_one_shaders[i].fixed_digest:local_fresnel_one_shaders[i].digest,16)) return (int)i;
    }
    return -1;
}
static int configure_local_fresnel_one(void) {
    char directory[MAX_PATH];
    DWORD length=GetEnvironmentVariableA("SRP_LOCAL_FRESNEL_ONE_DIR",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || strcmp(build,"026g") || legacy_local_diffuse_code) {
        error("local F0 exception requires native 026g local lighting"); return 0;
    }
    for (unsigned i=0;i<LOCAL_FRESNEL_COUNT;++i) {
        char path[MAX_PATH];
        int count=snprintf(path,sizeof(path),"%s/%s",directory,local_fresnel_one_shaders[i].file);
        if (count<0 || (size_t)count>=sizeof(path)) return 0;
        FILE *file=fopen(path,"rb");
        if (!file) { error("local F0 exception shader missing"); return 0; }
        size_t size=local_fresnel_one_shaders[i].fixed_size;
        void *code=malloc(size);
        int ok=code && fread(code,1,size,file)==size && fgetc(file)==EOF;
        fclose(file);
        if (!ok || memcmp(code,"DXBC",4) || *(uint32_t *)((char *)code+24)!=size || local_fresnel_one_index(code,size,1)!=(int)i) {
            free(code); error("local F0 exception identity mismatch"); return 0;
        }
        local_fresnel_one_code[i]=code;
    }
    return 1;
}
static void report_local_fresnel_creation(int index,HRESULT result,ID3D11PixelShader *shader) {
    if (FAILED(result) || !shader) { error("local F0 exception rejected"); return; }
    unsigned marker=(unsigned)index+1;
    ID3D11PixelShader_SetPrivateData(shader,&local_fresnel_one_guid,sizeof(marker),&marker);
    ++local_fresnel_one_created[index];
    char path[MAX_PATH+40]; snprintf(path,sizeof(path),"%s.local-fresnel-one.json",control);
    FILE *file=fopen(path,"w"); if (!file) return;
    fputs("{\"scope\":\"source F0=1 diffuse subset; native specular\",\"created\":[",file);
    for (unsigned i=0;i<LOCAL_FRESNEL_COUNT;++i) fprintf(file,"%s%u",i?",":"",local_fresnel_one_created[i]);
    fputs("]}\n",file); fclose(file);
}
static void observe_local_fresnel_one(ID3D11DeviceContext *context) {
    if (!local_fresnel_one_code[0]) return;
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    DWORD threshold=camera.settle_ms>250?camera.settle_ms-250:0;
    int take=camera.enabled && captured_id!=camera.id && GetTickCount()-applied_at>=threshold;
    LeaveCriticalSection(&lock); if (!take) return;
    static unsigned request,seen;
    if (request!=camera.id) { request=camera.id; seen=0; }
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned marker=0; UINT size=sizeof(marker);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&local_fresnel_one_guid,&size,&marker);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || !marker || marker>LOCAL_FRESNEL_COUNT || (seen&(1u<<(marker-1)))) return;
    seen|=1u<<(marker-1);
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.local-fresnel-one.json",camera.output);
    FILE *file=fopen(path,"w"); if (!file) return;
    fprintf(file,"{\"request\":%u,\"present\":%lu,\"observed_variants\":[",request,(unsigned long)present_calls);
    int comma=0;
    for (unsigned i=0;i<LOCAL_FRESNEL_COUNT;++i) if (seen&(1u<<i)) {
        fprintf(file,"%s\"%s\"",comma?",":"",local_fresnel_one_shaders[i].file); comma=1;
    }
    fputs("]}\n",file); fclose(file);
}
