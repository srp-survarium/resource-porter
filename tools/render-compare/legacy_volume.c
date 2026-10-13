// Scoped Factory cone experiment. Preserve the native geometry and blend state.
static const GUID legacy_volume_guid={0xae468a09,0x2f27,0x42cf,{0x9b,0x2e,0x9c,0x1b,0xfa,0x80,0x6a,0xa1}};
static void *legacy_volume_code;
static size_t legacy_volume_size;
static char legacy_volume_assets[MAX_PATH];
static ID3D11ShaderResourceView *legacy_volume_views[2];
static int legacy_volume_failed;
static unsigned legacy_volume_uses;
typedef struct { ID3D11ShaderResourceView *spot,*sphere; int bound; } VolumeState;

static int volume_texture(ID3D11Device *device,const char *name,ID3D11ShaderResourceView **view) {
    char path[MAX_PATH];
    if (snprintf(path,sizeof(path),"%s/%s",legacy_volume_assets,name)>=(int)sizeof(path)) return 0;
    unsigned char bytes[11049]={0};
    FILE *file=fopen(path,"rb"); if (!file) return 0;
    int ok=fread(bytes,1,sizeof(bytes),file)==sizeof(bytes) && fgetc(file)==EOF;
    fclose(file);
    uint32_t height,width,mips; memcpy(&height,bytes+12,4); memcpy(&width,bytes+16,4); memcpy(&mips,bytes+28,4);
    if (!ok || memcmp(bytes,"DDS ",4) || memcmp(bytes+84,"DXT1",4) || width!=128 || height!=128 || mips!=6 || bytes[11048]!=1) return 0;
    D3D11_SUBRESOURCE_DATA levels[6]={0}; unsigned offset=128;
    for (unsigned i=0;i<6;++i) {
        unsigned blocks=((128u>>i)+3)/4;
        levels[i].pSysMem=bytes+offset; levels[i].SysMemPitch=blocks*8;
        offset+=blocks*blocks*8;
    }
    if (offset!=11048) return 0;
    D3D11_TEXTURE2D_DESC desc={0};
    desc.Width=desc.Height=128; desc.MipLevels=6; desc.ArraySize=desc.SampleDesc.Count=1;
    desc.Format=DXGI_FORMAT_BC1_UNORM_SRGB; desc.Usage=D3D11_USAGE_IMMUTABLE;
    desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
    ID3D11Texture2D *texture=NULL;
    HRESULT hr=ID3D11Device_CreateTexture2D(device,&desc,levels,&texture);
    if (SUCCEEDED(hr)) {
        hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)texture,NULL,view);
        ID3D11Texture2D_Release(texture);
    }
    return SUCCEEDED(hr);
}
static void bind_volume(ID3D11DeviceContext *context,VolumeState *state) {
    memset(state,0,sizeof(*state));
    if (!legacy_volume_code || legacy_volume_failed) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&legacy_volume_guid,&size,&flag);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || flag!=1) return;
    if (!legacy_volume_views[0]) {
        ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
        int ok=volume_texture(device,"old-spot_falloff.dds",&legacy_volume_views[0]) &&
               volume_texture(device,"old-sphere_falloff.dds",&legacy_volume_views[1]);
        ID3D11Device_Release(device);
        if (!ok) { legacy_volume_failed=1; error("cannot load original cone falloff textures"); return; }
    }
    ID3D11DeviceContext_PSGetShaderResources(context,2,1,&state->spot);
    ID3D11DeviceContext_PSGetShaderResources(context,14,1,&state->sphere);
    ID3D11DeviceContext_PSSetShaderResources(context,2,1,&legacy_volume_views[0]);
    ID3D11DeviceContext_PSSetShaderResources(context,14,1,&legacy_volume_views[1]);
    state->bound=1;
}
static void restore_volume(ID3D11DeviceContext *context,VolumeState *state) {
    if (!state->bound) return;
    ID3D11DeviceContext_PSSetShaderResources(context,2,1,&state->spot);
    ID3D11DeviceContext_PSSetShaderResources(context,14,1,&state->sphere);
    if (state->spot) ID3D11ShaderResourceView_Release(state->spot);
    if (state->sphere) ID3D11ShaderResourceView_Release(state->sphere);
}
