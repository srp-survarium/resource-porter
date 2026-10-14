static void *factory_020e_ambient_apply_code;
static size_t factory_020e_ambient_apply_size;
// Capture the actual post-process draw, including its shader and bound inputs.
static const GUID post_code_guid={0x9205d461,0x758c,0x44fe,{0xad,0x61,0xa1,0x13,0x71,0x31,0x22,0xc3}};
static HRESULT (STDMETHODCALLTYPE *original_create_ps)(ID3D11Device *,const void *,SIZE_T,ID3D11ClassLinkage *,ID3D11PixelShader **);
static void (STDMETHODCALLTYPE *original_draw)(ID3D11DeviceContext *,UINT,UINT);
static void (STDMETHODCALLTYPE *original_draw_indexed)(ID3D11DeviceContext *,UINT,UINT,INT);
static unsigned draw_ids[27];
static int reference_capture;
static int disable_legacy_god_rays;
static int skip_legacy_god_rays(ID3D11DeviceContext *context);
static const GUID god_rays_apply_guid={0x6dc7fd40,0x9815,0x4f33,{0xaa,0x59,0x45,0x82,0x0b,0x15,0x67,0x2e}};
static const GUID reference_kind_guid={0x9cd670e9,0x301a,0x4f11,{0x98,0xc5,0x33,0x11,0x76,0x44,0xa1,0x09}};
#include "reference_shader_index.h"
#include "native_god_rays_index.h"
#include "native_aberration_index.h"
#include "native_emissive_index.h"
#include "rain_shader_index.h"
#include "volume_capture_filter.c"
static void *legacy_local_diffuse_code;
static size_t legacy_local_diffuse_size;
#include "local_fresnel_one.c"
static void tag_source_local(ID3D11PixelShader *shader,const void *code,SIZE_T size);
static void capture_local_diffuse(ID3D11DeviceContext *context,const Camera *camera);
static int legacy_ambient_diffuse_enabled;
static int source_ambient_valid(void);
static void capture_ambient_diffuse(ID3D11DeviceContext *context);
#include "local_light_capture.c"
static int capture_rain_enabled;
static void tag_source_rain(ID3D11PixelShader *shader,const void *code,SIZE_T size);
static unsigned rain_request_id, rain_draw_counts[3], rain_draw_variant;
static unsigned rain_first_present, rain_last_present;
static const char *rain_labels[]={"rain","rain-drops","rain-resolve"};
static void report_rain_counts(const char *output) {
    char path[MAX_PATH+32];
    snprintf(path,sizeof(path),"%s.rain-counts.json",output);
    FILE *report=fopen(path,"w");
    if (report) {
        fprintf(report,"{\"observed_draws\":[%u,%u,%u],\"families\":[\"rain\",\"rain-drops\",\"rain-resolve\"],\"capture_cap_per_family\":128,\"first_present\":%u,\"last_present\":%u,\"single_frame\":%s}\n",rain_draw_counts[0],rain_draw_counts[1],rain_draw_counts[2],rain_first_present,rain_last_present,rain_first_present==rain_last_present?"true":"false");
        fclose(report);
    }
}
static unsigned rain_kind(const void *code,SIZE_T size) {
    if (!capture_rain_enabled || size<32) return 0;
    for (unsigned i=0;i<sizeof(rain_shaders)/sizeof(rain_shaders[0]);++i)
        if (size==rain_shaders[i].size && !memcmp((const char *)code+4,rain_shaders[i].digest,16))
            return rain_shaders[i].kind;
    return 0;
}
static int capture_emissives_enabled;
static unsigned emissive_request_id, emissive_draw_count, emissive_draw_variant;
static void *grainless_aberration_code[8];
static unsigned grainless_aberration_uses;
static const char *reference_labels[]={"post","cloud","probe","fog-add","sky","ssao-temporal","fog-mul","composition","material","present","ssao-accumulation","ssao-upsample","volume","probe","sun","translucency","skylight","sun-mask"};
static void capture_draw_target(ID3D11DeviceContext *context,const Camera *camera,unsigned kind,const char *suffix);
static unsigned post_output_pending, post_output_kind;
static unsigned probe_request_id, probe_shader_count;
static unsigned char probe_shader_digests[16][16];
static int capture_materials_enabled;
static unsigned material_request_id, material_shader_count;
static unsigned char material_shader_digests[128][16];
static void *legacy_lut_code;
static size_t legacy_lut_size;
static unsigned legacy_lut_uses;
static const GUID legacy_fog_guid={0x33b451e1,0x8051,0x4e0b,{0x81,0x62,0x27,0x6a,0xd5,0x21,0x50,0x31}};
static void *legacy_fog_code;
static size_t legacy_fog_size;
static unsigned legacy_fog_uses;
static ID3D11SamplerState *legacy_fog_sampler;
static void *legacy_sky_code, *legacy_cloud_code;
static size_t legacy_sky_size, legacy_cloud_size;
static unsigned legacy_sky_uses, legacy_cloud_uses;
static void *legacy_post_code;
static size_t legacy_post_size;
static unsigned legacy_post_uses, legacy_present_uses;
static void *legacy_present_code; static size_t legacy_present_size;
static char legacy_post_grain_path[MAX_PATH];
static ID3D11ShaderResourceView *legacy_post_grain_view;
static void *legacy_probe_code;
static size_t legacy_probe_size;
static unsigned legacy_probe_uses;
static void *legacy_probe_box_code;
static size_t legacy_probe_box_size;
static void *legacy_composition_code;
static size_t legacy_composition_size;
static unsigned legacy_composition_uses;
static ID3D11PixelShader *native_composition_shader;
static ID3D11PixelShader *comparison_composition_shader;
static void *comparison_composition_code;
static size_t comparison_composition_size;
static int comparison_source_diffuse;
static int live_source_diffuse;
static ID3D11ShaderResourceView *source_full_diffuse_view(void);
static const GUID composition_guid={0x94bd30e1,0x2601,0x49c4,{0xa4,0x37,0x72,0x15,0x6b,0x1d,0x4c,0xe2}};
static unsigned composition_output_id;
static void *legacy_ambient_decode_code;
static size_t legacy_ambient_decode_size;
static unsigned legacy_ambient_decode_uses;
static ID3D11BlendState *legacy_decode_blend;

static int contains(const void *data, size_t length, const char *text) {
    size_t size=strlen(text)+1;
    if (length<size) return 0;
    for (size_t i=0;i<=length-size;++i) if (!memcmp((const char *)data+i,text,size)) return 1;
    return 0;
}

#include "legacy_translucency.c"
#include "legacy_volume.c"

static HRESULT STDMETHODCALLTYPE create_ps(ID3D11Device *device,const void *code,SIZE_T size,ID3D11ClassLinkage *linkage,ID3D11PixelShader **shader) {
    const unsigned char expected[]={0x18,0x28,0x29,0x05,0xd7,0x75,0xeb,0xa6,0xbb,0x7b,0x12,0xe0,0x57,0x71,0x0d,0xfe};
    const unsigned char present_expected[]={0x20,0x02,0x37,0x28,0x32,0x61,0xd9,0xb2,0x6a,0x85,0xaf,0xf8,0x8a,0x85,0x99,0x29};
    int present_replaced=legacy_present_code && size==976 && !memcmp((const char *)code+4,present_expected,16);
    int replaced=legacy_lut_code && size==3508 && !memcmp((const char *)code+4,expected,16);
    if (reference_capture) {
        HRESULT result=original_create_ps(device,code,size,linkage,shader);
        if (SUCCEEDED(result) && shader && *shader && size>=32) {
            unsigned rain=rain_kind(code,size);
            if (rain) {
                ID3D11PixelShader_SetPrivateData(*shader,&post_code_guid,(UINT)size,code);
                ID3D11PixelShader_SetPrivateData(*shader,&reference_kind_guid,sizeof(rain),&rain);
            }
            for (unsigned i=0;i<sizeof(reference_shaders)/sizeof(reference_shaders[0]);++i)
                if (size==reference_shaders[i].size && !memcmp((const char *)code+4,reference_shaders[i].digest,16)) {
                    unsigned kind=reference_shaders[i].kind;
                    ID3D11PixelShader_SetPrivateData(*shader,&post_code_guid,(UINT)size,code);
                    ID3D11PixelShader_SetPrivateData(*shader,&reference_kind_guid,sizeof(kind),&kind);
                    break;
                }
        }
        return result;
    }
    const unsigned char volume_expected[]={0x2a,0xdd,0xeb,0x6d,0xf0,0x2a,0xa8,0xdb,0x8c,0x81,0x01,0x78,0x6d,0xde,0x95,0xe9};
    int volume_replaced=legacy_volume_code && size==3752 && !memcmp((const char *)code+4,volume_expected,16);
    int aberration_index=native_aberration_index(code,size);
    int grain_replaced=aberration_index>=0 && grainless_aberration_code[aberration_index];
    if (grain_replaced) {
        unsigned paired=native_aberration_shaders[aberration_index].grainless;
        code=grainless_aberration_code[aberration_index]; size=native_aberration_shaders[paired].size;
    }
    const unsigned char fog_expected[]={0x0f,0x99,0x55,0x0b,0xd7,0x4e,0x29,0x0a,0xcf,0x08,0xab,0x46,0xdd,0xc5,0xed,0x20};
    int fog_replaced=legacy_fog_code && size==3608 && !memcmp((const char *)code+4,fog_expected,16);
    const unsigned char sky_expected[]={0xe7,0x17,0xa1,0x72,0x3c,0x00,0x86,0x3f,0x9d,0x5f,0x7b,0x57,0x62,0x15,0x5e,0x57};
    const unsigned char cloud_expected[]={0xd2,0xef,0x52,0xb2,0x15,0x6d,0xeb,0x31,0xd3,0x83,0x18,0xc3,0x8e,0x13,0x02,0xb5};
    int sky_replaced=legacy_sky_code && size==2588 && !memcmp((const char *)code+4,sky_expected,16);
    int cloud_replaced=legacy_cloud_code && size==4132 && !memcmp((const char *)code+4,cloud_expected,16);
    const unsigned char post_expected[]={0x22,0xa1,0xd1,0x39,0x98,0x9d,0x32,0x3c,0x71,0xb8,0x68,0x2b,0x30,0xbe,0xeb,0x48};
    int post_replaced=legacy_post_code && size==3972 && !memcmp((const char *)code+4,post_expected,16);
    const unsigned char probe_expected[]={0xa7,0x0e,0x27,0x2e,0x38,0xe4,0x26,0x66,0xe1,0x39,0xc6,0xc9,0xdb,0x33,0x51,0x66};
    int probe_replaced=legacy_probe_code && size==6104 && !memcmp((const char *)code+4,probe_expected,16);
    const unsigned char probe_box_expected[]={0x4f,0x5e,0x7d,0xc1,0xcf,0x82,0xff,0xa2,0x51,0x2c,0x12,0x5a,0x42,0xfe,0x1b,0xe5};
    int probe_box_replaced=legacy_probe_box_code && size==6116 && !memcmp((const char *)code+4,probe_box_expected,16);
    const unsigned char composition_expected[]={0xde,0x3e,0x7a,0x63,0x18,0x34,0xe1,0x5d,0xb4,0xae,0x34,0x73,0x59,0x86,0x7c,0x30};
    int composition_replaced=legacy_composition_code && size==8416 && !memcmp((const char *)code+4,composition_expected,16);
    const unsigned char ssao_expected[]={0xe8,0x52,0xc8,0x1d,0x2c,0xf9,0xc9,0x55,0x4a,0x60,0x46,0xc5,0x31,0x39,0x0e,0xc8};
    int ssao_reference=size==660 && !memcmp((const char *)code+4,ssao_expected,16);
    int ambient_decode_replaced=legacy_ambient_decode_code && ssao_reference;
    if (composition_replaced && !native_composition_shader &&
        FAILED(original_create_ps(device,code,size,linkage,&native_composition_shader))) {
        error("cannot create native composition reference");
        composition_replaced=0;
    }
    if (composition_replaced && comparison_composition_code && !comparison_composition_shader &&
        FAILED(original_create_ps(device,comparison_composition_code,comparison_composition_size,linkage,&comparison_composition_shader))) {
        error("cannot create composition comparison shader");
        composition_replaced=0;
    }
    if (replaced) { code=legacy_lut_code; size=legacy_lut_size; }
    if (fog_replaced) { code=legacy_fog_code; size=legacy_fog_size; }
    if (sky_replaced) { code=legacy_sky_code; size=legacy_sky_size; }
    if (cloud_replaced) { code=legacy_cloud_code; size=legacy_cloud_size; }
    if (present_replaced) { code=legacy_present_code; size=legacy_present_size; }
    if (post_replaced) { code=legacy_post_code; size=legacy_post_size; }
    if (probe_replaced) { code=legacy_probe_code; size=legacy_probe_size; }
    if (probe_box_replaced) { code=legacy_probe_box_code; size=legacy_probe_box_size; }
    if (composition_replaced) { code=legacy_composition_code; size=legacy_composition_size; }
    if (ambient_decode_replaced) { code=legacy_ambient_decode_code; size=legacy_ambient_decode_size; }
    if (volume_replaced) { code=legacy_volume_code; size=legacy_volume_size; }
<<<<<<< HEAD
    int local_f0_index=local_fresnel_one_index(code,size,0);
    int local_f0_replaced=local_f0_index>=0 && local_fresnel_one_code[local_f0_index];
    if (local_f0_replaced) { code=local_fresnel_one_code[local_f0_index]; size=local_fresnel_one_shaders[local_f0_index].fixed_size; }
    HRESULT hr=original_create_ps(device,code,size,linkage,shader);
    if (local_f0_replaced) report_local_fresnel_creation(local_f0_index,hr,shader?*shader:NULL);
=======
    const unsigned char native_ambient_apply_digest[]={0xe4,0xf0,0xf3,0xc2,0xbb,0x70,0x68,0x86,0x88,0xc2,0xa1,0xcf,0xac,0x6c,0x1a,0xcb};
    int factory_apply_replaced=factory_020e_ambient_apply_code && size==956 &&
        !memcmp((const unsigned char *)code+4,native_ambient_apply_digest,16);
    if (factory_apply_replaced) { code=factory_020e_ambient_apply_code; size=factory_020e_ambient_apply_size; }
    HRESULT hr=original_create_ps(device,code,size,linkage,shader);
    if (factory_apply_replaced) {
        char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.ambient-020e-apply.json",control);
        FILE *receipt=fopen(path,"w");
        if (receipt) { fprintf(receipt,"{\"source\":\"020e\",\"shader_bytes\":%u,\"hresult\":%ld}\n",(unsigned)size,hr); fclose(receipt); }
    }

>>>>>>> e8c30dd (Preserve Factory020e gameplay proof and guarded lighting investigation)
    if (volume_replaced) {
        if (FAILED(hr) || !shader || !*shader) error("original cone shader rejected");
        else {
            unsigned flag=1; ID3D11PixelShader_SetPrivateData(*shader,&legacy_volume_guid,sizeof(flag),&flag);
            ++legacy_volume_uses;
            char path[MAX_PATH+32]; snprintf(path,sizeof(path),"%s.volume-overrides.json",control);
            FILE *file=fopen(path,"w");
            if (file) { fprintf(file,"{\"uses\":%u,\"factory_attenuation_scale\":13}\n",legacy_volume_uses); fclose(file); }
        }
    }
    if (grain_replaced) {
        if (SUCCEEDED(hr) && shader && *shader) {
            unsigned flag=3+native_aberration_shaders[aberration_index].sampler_count;
            ID3D11PixelShader_SetPrivateData(*shader,&legacy_fog_guid,sizeof(flag),&flag);
        }
        if (FAILED(hr)) error("native grainless aberration shader rejected");
        else ++grainless_aberration_uses;
        char path[MAX_PATH+32]; snprintf(path,sizeof(path),"%s.grain-fix.json",control);
        FILE *file=fopen(path,"w");
        if (file) { fprintf(file,"{\"replacements\":%u,\"native_index\":%d,\"paired_index\":%u,\"hresult\":%ld}\n",grainless_aberration_uses,aberration_index,native_aberration_shaders[aberration_index].grainless,hr); fclose(file); }
    }
    if (SUCCEEDED(hr) && shader && *shader) mark_sun_projection(*shader,code,size);
    if (SUCCEEDED(hr) && shader && *shader && native_god_rays_kind(code,size)==18) {
        unsigned flag=1; ID3D11PixelShader_SetPrivateData(*shader,&god_rays_apply_guid,sizeof(flag),&flag);
    }
    if (composition_replaced && SUCCEEDED(hr) && shader && *shader) {
        unsigned flag=1; ID3D11PixelShader_SetPrivateData(*shader,&composition_guid,sizeof(flag),&flag);
    }
    if (fog_replaced && SUCCEEDED(hr) && shader && *shader) {
        unsigned flag=1; ID3D11PixelShader_SetPrivateData(*shader,&legacy_fog_guid,sizeof(flag),&flag);
    }
    if (post_replaced && SUCCEEDED(hr) && shader && *shader && contains(code,size,"t_legacy_grain")) {
        unsigned flag=3; ID3D11PixelShader_SetPrivateData(*shader,&legacy_fog_guid,sizeof(flag),&flag);
    }
    if (ambient_decode_replaced && SUCCEEDED(hr) && shader && *shader) {
        unsigned flag=2; ID3D11PixelShader_SetPrivateData(*shader,&legacy_fog_guid,sizeof(flag),&flag);
    }
    if (replaced || fog_replaced || sky_replaced || cloud_replaced || post_replaced || probe_replaced || probe_box_replaced || composition_replaced || ambient_decode_replaced || present_replaced) {
        if (FAILED(hr)) error("legacy atmosphere shader rejected");
        else if (replaced) ++legacy_lut_uses;
        else if (fog_replaced) ++legacy_fog_uses;
        else if (sky_replaced) ++legacy_sky_uses;
        else if (cloud_replaced) ++legacy_cloud_uses;
        else if (post_replaced) ++legacy_post_uses;
        else if (present_replaced) ++legacy_present_uses;
        else if (composition_replaced) ++legacy_composition_uses;
        else if (ambient_decode_replaced) ++legacy_ambient_decode_uses;
        else ++legacy_probe_uses;
        char path[MAX_PATH+32]; snprintf(path,sizeof(path),"%s.shader-overrides.json",control);
        FILE *file=fopen(path,"w");
        if (file) { fprintf(file,"{\"legacy_lut_uses\":%u,\"legacy_fog_uses\":%u,\"legacy_sky_uses\":%u,\"legacy_cloud_uses\":%u,\"legacy_post_uses\":%u,\"legacy_probe_uses\":%u,\"legacy_composition_uses\":%u,\"legacy_ambient_decode_uses\":%u,\"hresult\":%ld}\n",legacy_lut_uses,legacy_fog_uses,legacy_sky_uses,legacy_cloud_uses,legacy_post_uses,legacy_probe_uses,legacy_composition_uses,legacy_ambient_decode_uses,hr); fclose(file); }
    }
    if (legacy_ssao_early && (probe_replaced || probe_box_replaced ||
        (legacy_ambient_diffuse_enabled && contains(code,size,"t_probe_cubemap_diffuse"))) && SUCCEEDED(hr) && shader && *shader) {
        unsigned marker=probe_box_replaced?2:probe_replaced?1:3;
        ID3D11PixelShader_SetPrivateData(*shader,&early_ssao_probe_guid,sizeof(marker),&marker);
    }
    if (SUCCEEDED(hr) && shader && *shader) tag_source_local(*shader,code,size);
    if (SUCCEEDED(hr) && shader && *shader) tag_source_rain(*shader,code,size);
    int local_index=local_light_index(code,size);
    if (local_index>=0 && SUCCEEDED(hr)) ++local_light_created[local_light_shaders[local_index].family];
    if (SUCCEEDED(hr) && shader && *shader && size<=1024*1024 &&
        (local_index>=0 || rain_kind(code,size) || (capture_emissives_enabled && is_native_emissive(code,size)) || is_native_aberration(code,size) || ssao_reference || native_god_rays_kind(code,size) || (contains(code,size,"t_color_grading_lut") && contains(code,size,"t_frame_luminance")) ||
        (contains(code,size,"sky_clouds_parameters0") || contains(code,size,"sky_clouds_texture")) || contains(code,size,"t_probe_cubemap_diffuse") ||
        contains(code,size,"t_rayleigh_scattering") || contains(code,size,"hba_parameters") ||
        contains(code,size,"skylight_parameters0") ||
        (contains(code,size,"volume_color") && contains(code,size,"attenuation_scale")) ||
        (contains(code,size,"sun_direction_parameter") && contains(code,size,"shadow_filter_radius")) ||
        (contains(code,size,"gamma_correction_factor") && contains(code,size,"t_base")) ||
        (capture_materials_enabled && contains(code,size,"deferred_material_parameters")) ||
        contains(code,size,"t_full_encoded_diffuse") ||
        (contains(code,size,"t_diffuse_accumulation") && contains(code,size,"t_specular_accumulation"))))
        ID3D11PixelShader_SetPrivateData(*shader,&post_code_guid,(UINT)size,code);
    return hr;
}

static int write_bytes(const char *path,const void *data,size_t size) {
    FILE *file=fopen(path,"wb"); if (!file) return 0;
    int ok=fwrite(data,1,size,file)==size;
    return fclose(file)==0 && ok;
}

#include "material_vertex_capture.c"
#include "rain_geometry_capture.c"

static void capture_probe_initial(ID3D11DeviceContext *context,const char *output) {
    ID3D11RenderTargetView *views[2]={0};
    ID3D11DeviceContext_OMGetRenderTargets(context,2,views,NULL);
    char base[MAX_PATH+48],path[MAX_PATH+80];
    snprintf(base,sizeof(base),"%s.probe-initial",output);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *file=fopen(path,"w"); unsigned count=0;
    if (file) fputs("[",file);
    for (unsigned i=0;i<2;++i) if (views[i]) {
        ID3D11Resource *resource=NULL; ID3D11RenderTargetView_GetResource(views[i],&resource);
        ID3D11Texture2D *texture=NULL;
        if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) {
            if (file) { if (count++) fputs(",",file); dump_texture(texture,base,file,i?"specular_before_first_probe":"diffuse_before_first_probe",i,NULL); }
            ID3D11Texture2D_Release(texture);
        }
        ID3D11Resource_Release(resource); ID3D11RenderTargetView_Release(views[i]);
    }
    if (file) { fputs("]\n",file); fclose(file); }
}

static void dump_volume(ID3D11Texture3D *texture,const char *base,FILE *index,unsigned slot,const D3D11_SHADER_RESOURCE_VIEW_DESC *view) {
    D3D11_TEXTURE3D_DESC desc; ID3D11Texture3D_GetDesc(texture,&desc);
    fprintf(index,"{\"slot\":%u,\"name\":\"volume_input_%u\",\"width\":%u,\"height\":%u,\"depth\":%u,\"format\":%u,\"view_format\":%u,\"view_dimension\":%u,\"view_most_detailed_mip\":%u",slot,slot,desc.Width,desc.Height,desc.Depth,desc.Format,view->Format,view->ViewDimension,view->Texture3D.MostDetailedMip);
    if (desc.Width>64 || desc.Height>64 || desc.Depth>64 || (desc.Format!=DXGI_FORMAT_R8G8B8A8_UNORM && desc.Format!=DXGI_FORMAT_B8G8R8A8_UNORM) || view->Texture3D.MostDetailedMip) {
        fputs(",\"skipped\":\"volume contract\"}",index); return;
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext *context=NULL; ID3D11Texture3D *staging=NULL;
    ID3D11Texture3D_GetDevice(texture,&device); ID3D11Device_GetImmediateContext(device,&context);
    desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ; desc.MiscFlags=0; desc.MipLevels=1;
    HRESULT hr=ID3D11Device_CreateTexture3D(device,&desc,NULL,&staging); int ok=0;
    if (SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopySubresourceRegion(context,(ID3D11Resource *)staging,0,0,0,0,(ID3D11Resource *)texture,0,NULL);
        D3D11_MAPPED_SUBRESOURCE mapped;
        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
        if (SUCCEEDED(hr)) {
            char path[MAX_PATH+80]; snprintf(path,sizeof(path),"%s.vol%02u.bin",base,slot);
            FILE *file=fopen(path,"wb");
            if (file) {
                ok=1;
                for (UINT z=0;z<desc.Depth;++z) for (UINT y=0;y<desc.Height;++y)
                    if (fwrite((char *)mapped.pData+z*mapped.DepthPitch+y*mapped.RowPitch,desc.Width*4,1,file)!=1) ok=0;
                if (fclose(file)) ok=0;
            }
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        }
    }
    fprintf(index,",\"bytes_per_pixel\":4,\"written\":%s,\"hresult\":%ld}",ok?"true":"false",hr);
    if (staging) ID3D11Texture3D_Release(staging);
    ID3D11DeviceContext_Release(context); ID3D11Device_Release(device);
}

static void capture_post_draw(ID3D11DeviceContext *context,UINT count,UINT start,INT base_vertex,int indexed) {
    Camera camera;
    EnterCriticalSection(&lock);
    int take=applied.enabled && capture_frame_id==applied.id && captured_id!=applied.id && GetTickCount()-applied_at>=applied.settle_ms;
    camera=applied;
    LeaveCriticalSection(&lock);
    if (!take) return;
    begin_local_light_capture(&camera);
    if (capture_rain_enabled && rain_request_id!=camera.id) {
        rain_request_id=camera.id;
        memset(rain_draw_counts,0,sizeof(rain_draw_counts));
        rain_first_present=rain_last_present=present_calls;
        report_rain_counts(camera.output);
    }
    if (capture_rain_enabled && rain_last_present!=present_calls) {
        rain_last_present=present_calls;
        report_rain_counts(camera.output);
    }
    ID3D11PixelShader *shader=NULL;
    ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    UINT size=0;
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,NULL);
    if (FAILED(hr) || !size || size>1024*1024) { ID3D11PixelShader_Release(shader); return; }
    void *code=malloc(size);
    if (!code) { ID3D11PixelShader_Release(shader); return; }
    hr=ID3D11PixelShader_GetPrivateData(shader,&post_code_guid,&size,code);
    unsigned reference_kind=0, reference_kind_size=sizeof(reference_kind);
    HRESULT reference_hr=reference_capture?ID3D11PixelShader_GetPrivateData(shader,&reference_kind_guid,&reference_kind_size,&reference_kind):S_OK;
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || FAILED(reference_hr)) { free(code); return; }
    unsigned kind=capture_emissives_enabled && is_native_emissive(code,size)?21:is_native_aberration(code,size)?22:contains(code,size,"sun_direction_parameter") && contains(code,size,"shadow_filter_radius")?13:contains(code,size,"volume_color") && contains(code,size,"attenuation_scale")?12:contains(code,size,"gamma_correction_factor") && contains(code,size,"t_base")?9:(contains(code,size,"t_full_encoded_diffuse") || (contains(code,size,"t_diffuse_accumulation") && contains(code,size,"t_specular_accumulation")))?7:
        capture_materials_enabled && contains(code,size,"deferred_material_parameters")?8:
        contains(code,size,"hba_parameters") || contains(code,size,"t_encoded_ambient") || (contains(code,size,"t_ssao_accumulator") && !contains(code,size,"t_probe_cubemap_diffuse"))?5:contains(code,size,"skylight_parameters0")?6:
        contains(code,size,"t_frame_luminance")?0:(contains(code,size,"sky_clouds_parameters0") || contains(code,size,"sky_clouds_texture"))?1:
        contains(code,size,"t_probe_cubemap_diffuse")?2:contains(code,size,"t_position")?3:4;
    if (reference_capture) kind=reference_kind;
    else if (rain_kind(code,size)) kind=rain_kind(code,size);
    else if (native_god_rays_kind(code,size)) kind=native_god_rays_kind(code,size);
    int local_index=local_light_index(code,size);
    if (!reference_capture && local_index>=0) {
        if (!capture_local_lights_enabled) { free(code); return; }
        kind=26;
    }
    if (!reference_capture && kind==7) {
        capture_ambient_diffuse(context);
        capture_local_diffuse(context,&camera);
    }
    if (!reference_capture && kind==12 && draw_ids[kind]!=camera.id && !select_volume_capture(context,&camera)) { free(code); return; }
    unsigned probe_variant=0;
    unsigned material_variant=0;
    if (kind==26) {
        local_light_variant=local_light_count++;
        ++local_light_observed[local_light_shaders[local_index].family];
        report_local_lights(&camera);
        if (local_light_variant>=LOCAL_LIGHT_CAPTURE_CAP) { free(code); return; }
    } else if (kind>=23 && kind<=25) {
        rain_draw_variant=rain_draw_counts[kind-23]++;
        report_rain_counts(camera.output);
        /* Original engine supports at most 100 cones. Report overflow too. */
        if (rain_draw_variant>=128) { free(code); return; }
    } else if (kind==21) {
        /* Depth-only draws can share bytecode with an emissive permutation. */
        ID3D11RenderTargetView *emissive_target=NULL;
        ID3D11DeviceContext_OMGetRenderTargets(context,1,&emissive_target,NULL);
        if (!emissive_target) { free(code); return; }
        ID3D11RenderTargetView_Release(emissive_target);
        if (emissive_request_id!=camera.id) { emissive_request_id=camera.id; emissive_draw_count=0; }
        if (emissive_draw_count>=64) { free(code); return; }
        emissive_draw_variant=emissive_draw_count++;
    } else if (kind==2) {
        if (probe_request_id!=camera.id) { probe_request_id=camera.id; probe_shader_count=0; }
        if (size<20 || probe_shader_count>=16) { free(code); return; }
        for (unsigned i=0;i<probe_shader_count;++i)
            if (!memcmp(probe_shader_digests[i],(char *)code+4,16)) { free(code); return; }
        probe_variant=probe_shader_count;
    } else if (kind==8) {
        if (material_request_id!=camera.id) { material_request_id=camera.id; material_shader_count=0; }
        if (size<20 || material_shader_count>=128) { free(code); return; }
        for (unsigned i=0;i<material_shader_count;++i)
            if (!memcmp(material_shader_digests[i],(char *)code+4,16)) { free(code); return; }
        material_variant=material_shader_count;
    } else if (draw_ids[kind]==camera.id || (kind==0 && !reference_capture && buffers_id!=camera.id) || (kind==9 && draw_ids[0]!=camera.id) ||
        (kind==5 && !reference_capture && ambient_id!=camera.id)) { free(code); return; }
    if (kind==2 && !probe_variant) capture_probe_initial(context,camera.output);
    char base[MAX_PATH+48],path[MAX_PATH+80];
    const char *labels[]={"post-draw","cloud-draw","probe-draw","fog-draw","sky-draw","hba-draw","skylight-draw","composition-draw","material-draw","present-draw","unused10","unused11","volume-draw","sun-shadow-draw","unused14","unused15","unused16","unused17","god-rays-apply-draw","god-rays-draw","god-rays-mask-draw","unused21","aberration-draw"};
    if (kind==26) {
        snprintf(base,sizeof(base),"%s.local-light-draw-%u",camera.output,local_light_variant);
        capture_local_light_state(context,base,(unsigned)local_index);
        capture_rain_geometry(context,base,count,start,base_vertex,indexed);
        capture_material_vertex(context,base);
    } else if (kind>=23 && kind<=25) {
        if (rain_draw_variant) snprintf(base,sizeof(base),"%s.%s-draw-%u",camera.output,rain_labels[kind-23],rain_draw_variant);
        else snprintf(base,sizeof(base),"%s.%s-draw",camera.output,rain_labels[kind-23]);
        capture_rain_geometry(context,base,count,start,base_vertex,indexed);
        capture_material_vertex(context,base);
    } else if (reference_capture) snprintf(base,sizeof(base),"%s.%s-draw",camera.output,reference_labels[kind]);
    else if (kind==21)
        snprintf(base,sizeof(base),"%s.emissive-draw-%u",camera.output,emissive_draw_variant);
    else if (kind==8)
        snprintf(base,sizeof(base),"%s.material-draw-%u",camera.output,material_variant);
    else if (kind==2 && probe_variant)
        snprintf(base,sizeof(base),"%s.probe-draw-%u",camera.output,probe_variant);
    else snprintf(base,sizeof(base),"%s.%s",camera.output,labels[kind]);
    if (!reference_capture && kind==18) {
        snprintf(path,sizeof(path),"%s.compatibility.json",base); FILE *metadata=fopen(path,"w");
        if (metadata) { fprintf(metadata,"{\"disable_god_rays_requested\":%s,\"draw_skipped\":%s}\n",disable_legacy_god_rays?"true":"false",skip_legacy_god_rays(context)?"true":"false"); fclose(metadata); }
    }
    if (kind==2) memcpy(probe_shader_digests[probe_shader_count++],(char *)code+4,16);
    if (kind==8) memcpy(material_shader_digests[material_shader_count++],(char *)code+4,16);
    if (((kind==8 || kind==1 || kind==12) && capture_materials_enabled) ||
        (kind==1 && capture_rain_enabled)) capture_material_vertex(context,base);
    if (kind==12 && capture_materials_enabled) capture_rain_geometry(context,base,count,start,base_vertex,indexed);
    snprintf(path,sizeof(path),"%s.dxbc",base);
    int ok=SUCCEEDED(hr) && write_bytes(path,code,size); free(code);
    if (!ok) { error("cannot save active post-process shader"); return; }
    ID3D11BlendState *blend=NULL; float factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    if (blend) {
        D3D11_BLEND_DESC desc; ID3D11BlendState_GetDesc(blend,&desc); ID3D11BlendState_Release(blend);
        snprintf(path,sizeof(path),"%s.blend.json",base); FILE *file=fopen(path,"w");
        if (file) { fprintf(file,"{\"enabled\":%d,\"source\":%u,\"destination\":%u,\"operation\":%u}\n",desc.RenderTarget[0].BlendEnable,desc.RenderTarget[0].SrcBlend,desc.RenderTarget[0].DestBlend,desc.RenderTarget[0].BlendOp); fclose(file); }
    }
    ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
    ID3D11Buffer *buffers[4]={0};
    ID3D11DeviceContext_PSGetConstantBuffers(context,0,4,buffers);
    for (unsigned i=0;i<4;++i) if (buffers[i]) {
        D3D11_BUFFER_DESC desc; ID3D11Buffer_GetDesc(buffers[i],&desc);
        desc.Usage=D3D11_USAGE_STAGING; desc.BindFlags=0; desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ; desc.MiscFlags=0;
        ID3D11Buffer *staging=NULL;
        hr=ID3D11Device_CreateBuffer(device,&desc,NULL,&staging);
        if (SUCCEEDED(hr)) {
            ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)buffers[i]);
            D3D11_MAPPED_SUBRESOURCE mapped;
            hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
            if (SUCCEEDED(hr)) {
                snprintf(path,sizeof(path),"%s.cb%u.bin",base,i);
                if (!write_bytes(path,mapped.pData,desc.ByteWidth)) error("cannot save post-process constants");
                ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
            }
            ID3D11Buffer_Release(staging);
        }
        ID3D11Buffer_Release(buffers[i]);
    }
    ID3D11Device_Release(device);
    {
        snprintf(path,sizeof(path),"%s.samplers.json",base); FILE *file=fopen(path,"w");
        if (file) {
            ID3D11SamplerState *samplers[16]={0}; ID3D11DeviceContext_PSGetSamplers(context,0,16,samplers);
            fputs("[",file); unsigned count=0;
            for (unsigned i=0;i<16;++i) if (samplers[i]) {
                D3D11_SAMPLER_DESC desc; ID3D11SamplerState_GetDesc(samplers[i],&desc);
                if (count++) fputs(",",file);
                fprintf(file,"{\"slot\":%u,\"filter\":%u,\"address\":[%u,%u,%u],\"min_lod\":%.9g,\"max_lod\":%.9g}",i,desc.Filter,desc.AddressU,desc.AddressV,desc.AddressW,desc.MinLOD,desc.MaxLOD);
                ID3D11SamplerState_Release(samplers[i]);
            }
            fputs("]\n",file); fclose(file);
        }
    }
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *index=fopen(path,"w");
    if (index) {
        ID3D11ShaderResourceView *views[16]={0};
        ID3D11DeviceContext_PSGetShaderResources(context,0,16,views);
        fputs("[",index); unsigned count=0;
        for (unsigned i=0;i<16;++i) if (views[i]) {
            ID3D11Resource *resource=NULL; ID3D11ShaderResourceView_GetResource(views[i],&resource);
            ID3D11Texture2D *texture=NULL;
            hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
            if (SUCCEEDED(hr)) {
                D3D11_SHADER_RESOURCE_VIEW_DESC view_desc;
                ID3D11ShaderResourceView_GetDesc(views[i],&view_desc);
                char name[48]; snprintf(name,sizeof(name),"shader_input_%u_view_format_%u",i,view_desc.Format);
                if (count++) fputs(",",index);
                if (kind==8) {
                    D3D11_TEXTURE2D_DESC desc; ID3D11Texture2D_GetDesc(texture,&desc);
                    fprintf(index,"{\"slot\":%u,\"name\":\"%s\",\"width\":%u,\"height\":%u,\"format\":%u,\"array_size\":%u,\"mip_levels\":%u,\"view_format\":%u,\"view_dimension\":%u,\"most_detailed_mip\":%u,\"view_mip_levels\":%u,\"skipped\":\"material metadata only\"}",i,name,desc.Width,desc.Height,desc.Format,desc.ArraySize,desc.MipLevels,view_desc.Format,view_desc.ViewDimension,view_desc.Texture2D.MostDetailedMip,view_desc.Texture2D.MipLevels);
                } else dump_texture(texture,base,index,name,i,&view_desc);
                ID3D11Texture2D_Release(texture);
            }
            if (kind==0) {
                D3D11_RESOURCE_DIMENSION dimension; ID3D11Resource_GetType(resource,&dimension);
                if (dimension==D3D11_RESOURCE_DIMENSION_TEXTURE3D) {
                    ID3D11Texture3D *volume=NULL;
                    if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture3D,(void **)&volume))) {
                        D3D11_SHADER_RESOURCE_VIEW_DESC view_desc; ID3D11ShaderResourceView_GetDesc(views[i],&view_desc);
                        if (count++) fputs(",",index);
                        dump_volume(volume,base,index,i,&view_desc); ID3D11Texture3D_Release(volume);
                    }
                }
            }
            ID3D11Resource_Release(resource); ID3D11ShaderResourceView_Release(views[i]);
        }
        fputs("]\n",index); fclose(index);
    }
    if (reference_capture || kind==0 || kind==1 || kind==3 || kind==9 || kind==12 || kind==13 || kind>=18) {
        capture_draw_target(context,&camera,kind,"before");
        post_output_pending=camera.id; post_output_kind=kind;
    }
    EnterCriticalSection(&lock); draw_ids[kind]=camera.id; LeaveCriticalSection(&lock);
}

typedef struct {
    ID3D11ShaderResourceView *view, *native_ao;
    int ao_bound;
    ID3D11BlendState *blend;
    ID3D11SamplerState *sampler;
    int sampler_bound;
    ID3D11SamplerState *grain_samplers[2];
    unsigned grain_sampler_count;
    float factor[4];
    UINT mask;
    int bound,blend_bound;
    UINT extra_slot;
} LegacyExtraState;
static void bind_legacy_extra(ID3D11DeviceContext *context,LegacyExtraState *state) {
    memset(state,0,sizeof(*state));
    if (!legacy_fog_code && !legacy_ambient_decode_code && !legacy_post_grain_path[0]) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&legacy_fog_guid,&size,&flag);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || !flag) return;
    if (flag==4 || flag==5) {
        // The authentic no-grain permutation omits s_nofilter at slot0.
        // Preserve the engine's original bindings by shifting only retained
        // samplers for this draw, then restoring the previous slots.
        unsigned count=flag-3;
        ID3D11SamplerState *mapped[2]={0};
        ID3D11DeviceContext_PSGetSamplers(context,0,count,state->grain_samplers);
        ID3D11DeviceContext_PSGetSamplers(context,1,count,mapped);
        ID3D11DeviceContext_PSSetSamplers(context,0,count,mapped);
        for (unsigned i=0;i<count;++i) if (mapped[i]) ID3D11SamplerState_Release(mapped[i]);
        state->grain_sampler_count=count;
        return;
    }
    if (flag==1) {
        // Original s_material1 clamps at the LUT edges. Native s_linear wraps;
        // short-distance Mie coordinates straddle the first texel boundary.
        if (!legacy_fog_sampler) {
            ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
            D3D11_SAMPLER_DESC desc={0}; desc.Filter=D3D11_FILTER_MIN_MAG_MIP_LINEAR;
            desc.AddressU=desc.AddressV=desc.AddressW=D3D11_TEXTURE_ADDRESS_CLAMP;
            desc.MaxLOD=FLT_MAX; desc.MaxAnisotropy=1; desc.ComparisonFunc=D3D11_COMPARISON_NEVER;
            hr=ID3D11Device_CreateSamplerState(device,&desc,&legacy_fog_sampler); ID3D11Device_Release(device);
            if (FAILED(hr)) { error("cannot create original fog clamp sampler"); return; }
        }
        ID3D11DeviceContext_PSGetSamplers(context,0,1,&state->sampler);
        ID3D11DeviceContext_PSSetSamplers(context,0,1,&legacy_fog_sampler); state->sampler_bound=1;
    }
    if (flag==3 && !legacy_post_grain_view) {
        unsigned char bytes[2048]; FILE *file=fopen(legacy_post_grain_path,"rb");
        if (!file) { error("original grain texture missing"); return; }
        int ok=fread(bytes,1,sizeof(bytes),file)==sizeof(bytes) && fgetc(file)==EOF; fclose(file);
        if (!ok) { error("original grain texture must be 64x64 BC1 mip zero"); return; }
        ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
        D3D11_TEXTURE2D_DESC desc={0}; desc.Width=desc.Height=64; desc.MipLevels=desc.ArraySize=desc.SampleDesc.Count=1;
        desc.Format=DXGI_FORMAT_BC1_UNORM; desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
        D3D11_SUBRESOURCE_DATA initial={bytes,128,0}; ID3D11Texture2D *texture=NULL;
        hr=ID3D11Device_CreateTexture2D(device,&desc,&initial,&texture);
        if (SUCCEEDED(hr)) { hr=ID3D11Device_CreateShaderResourceView(device,(ID3D11Resource *)texture,NULL,&legacy_post_grain_view); ID3D11Texture2D_Release(texture); }
        ID3D11Device_Release(device);
        if (FAILED(hr)) { error("cannot allocate original grain texture on scene device"); return; }
    }
    ID3D11ShaderResourceView *view=flag==1?legacy_mie_view:flag==2?legacy_ambient_view:flag==3?legacy_post_grain_view:NULL;
    state->extra_slot=flag==3?12:2;
    if (!view) { error("legacy draw extra resource missing"); return; }
    if (flag==2 && legacy_ssao_enabled) {
        if (!legacy_ssao_ready || ssao_ready_present!=present_calls) { error("original SSAO was not rendered this frame"); return; }
        ID3D11DeviceContext_PSGetShaderResources(context,0,1,&state->native_ao);
        Camera camera;
        EnterCriticalSection(&lock); camera=applied;
        int take=camera.enabled && capture_frame_id==camera.id && draw_ids[5]!=camera.id;
        LeaveCriticalSection(&lock);
        if (take && state->native_ao) {
            ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL;
            ID3D11ShaderResourceView_GetResource(state->native_ao,&resource);
            if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) {
                char base[MAX_PATH+32],path[MAX_PATH+64];
                snprintf(base,sizeof(base),"%s.ssao-native",camera.output);
                snprintf(path,sizeof(path),"%s.targets.json",base);
                FILE *file=fopen(path,"w");
                if (file) { fputs("[",file); dump_texture(texture,base,file,"native_ssao",0,NULL); fputs("]\n",file); fclose(file); }
                ID3D11Texture2D_Release(texture);
            }
            ID3D11Resource_Release(resource);
        }
        ID3D11DeviceContext_PSSetShaderResources(context,0,1,&legacy_ssao_view); state->ao_bound=1;
    }
    if (flag==2) {
        if (!legacy_decode_blend) {
            ID3D11Device *device=NULL; ID3D11DeviceContext_GetDevice(context,&device);
            D3D11_BLEND_DESC desc={0};
            desc.RenderTarget[0].SrcBlend=D3D11_BLEND_ONE;
            desc.RenderTarget[0].DestBlend=D3D11_BLEND_ZERO;
            desc.RenderTarget[0].BlendOp=D3D11_BLEND_OP_ADD;
            desc.RenderTarget[0].SrcBlendAlpha=D3D11_BLEND_ONE;
            desc.RenderTarget[0].DestBlendAlpha=D3D11_BLEND_ZERO;
            desc.RenderTarget[0].BlendOpAlpha=D3D11_BLEND_OP_ADD;
            desc.RenderTarget[0].RenderTargetWriteMask=D3D11_COLOR_WRITE_ENABLE_ALL;
            hr=ID3D11Device_CreateBlendState(device,&desc,&legacy_decode_blend);
            ID3D11Device_Release(device);
            if (FAILED(hr)) { error("cannot create ambient decode blend state"); return; }
        }
        ID3D11DeviceContext_OMGetBlendState(context,&state->blend,state->factor,&state->mask);
        ID3D11DeviceContext_OMSetBlendState(context,legacy_decode_blend,state->factor,state->mask);
        state->blend_bound=1;
    }
    ID3D11DeviceContext_PSGetShaderResources(context,state->extra_slot,1,&state->view);
    ID3D11DeviceContext_PSSetShaderResources(context,state->extra_slot,1,&view); state->bound=1;
}
static void restore_extra(ID3D11DeviceContext *context,LegacyExtraState *state) {
    if (state->grain_sampler_count) {
        ID3D11DeviceContext_PSSetSamplers(context,0,state->grain_sampler_count,state->grain_samplers);
        for (unsigned i=0;i<state->grain_sampler_count;++i)
            if (state->grain_samplers[i]) ID3D11SamplerState_Release(state->grain_samplers[i]);
    }
    if (state->sampler_bound) {
        ID3D11DeviceContext_PSSetSamplers(context,0,1,&state->sampler);
        if (state->sampler) ID3D11SamplerState_Release(state->sampler);
    }
    if (state->ao_bound) {
        ID3D11DeviceContext_PSSetShaderResources(context,0,1,&state->native_ao);
        if (state->native_ao) ID3D11ShaderResourceView_Release(state->native_ao);
    }
    if (state->bound) {
        ID3D11DeviceContext_PSSetShaderResources(context,state->extra_slot,1,&state->view);
        if (state->view) ID3D11ShaderResourceView_Release(state->view);
    }
    if (state->blend_bound) {
        ID3D11DeviceContext_OMSetBlendState(context,state->blend,state->factor,state->mask);
        if (state->blend) ID3D11BlendState_Release(state->blend);
    }
}
static void capture_composition_outputs(ID3D11DeviceContext *context,UINT count,UINT start,INT base_vertex,int indexed) {
    if (!native_composition_shader) return;
    Camera camera;
    EnterCriticalSection(&lock);
    camera=applied;
    int take=camera.enabled && captured_id!=camera.id && composition_output_id!=camera.id && draw_ids[7]==camera.id;
    LeaveCriticalSection(&lock);
    if (!take) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&composition_guid,&size,&flag);
    if (FAILED(hr) || !flag) { ID3D11PixelShader_Release(shader); return; }
    // Composition overwrites RT0. Refuse to replay if another variant blends.
    ID3D11BlendState *blend=NULL; float factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    if (blend) {
        D3D11_BLEND_DESC desc; ID3D11BlendState_GetDesc(blend,&desc); ID3D11BlendState_Release(blend);
        if (desc.RenderTarget[0].BlendEnable) { ID3D11PixelShader_Release(shader); error("composition reference requires overwrite blend"); return; }
    }
    ID3D11RenderTargetView *view=NULL; ID3D11DeviceContext_OMGetRenderTargets(context,1,&view,NULL);
    if (!view) { ID3D11PixelShader_Release(shader); return; }
    ID3D11Resource *resource=NULL; ID3D11RenderTargetView_GetResource(view,&resource);
    ID3D11RenderTargetView_Release(view);
    ID3D11Texture2D *texture=NULL;
    hr=ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture);
    ID3D11Resource_Release(resource);
    if (FAILED(hr)) { ID3D11PixelShader_Release(shader); return; }
    char base[MAX_PATH+48],path[MAX_PATH+80];
    snprintf(base,sizeof(base),"%s.composition-output",camera.output);
    snprintf(path,sizeof(path),"%s.targets.json",base);
    FILE *file=fopen(path,"w");
    if (file) {
        fputs("[",file); dump_texture(texture,base,file,"override",0,NULL); fputs(",",file);
        ID3D11DeviceContext_PSSetShader(context,native_composition_shader,NULL,0);
        if (indexed) original_draw_indexed(context,count,start,base_vertex); else original_draw(context,count,start);
        dump_texture(texture,base,file,"native_same_inputs",1,NULL);
        if (comparison_composition_shader) {
            ID3D11ShaderResourceView *source_view=comparison_source_diffuse?source_full_diffuse_view():NULL;
            ID3D11ShaderResourceView *saved_view=NULL;
            int ready=!comparison_source_diffuse || source_view;
            if (comparison_source_diffuse && !ready) error("composition comparison requires current complete source diffuse");
            if (comparison_source_diffuse && ready) {
                ID3D11DeviceContext_PSGetShaderResources(context,15,1,&saved_view);
                ID3D11DeviceContext_PSSetShaderResources(context,15,1,&source_view);
            }
            if (ready) {
                ID3D11DeviceContext_PSSetShader(context,comparison_composition_shader,NULL,0);
                if (indexed) original_draw_indexed(context,count,start,base_vertex); else original_draw(context,count,start);
                fputs(",",file); dump_texture(texture,base,file,comparison_source_diffuse?"comparison_source_diffuse":"comparison_same_inputs",2,NULL);
                snprintf(path,sizeof(path),"%s.comparison.dxbc",base);
                if (!write_bytes(path,comparison_composition_code,comparison_composition_size))
                    error("cannot save composition comparison shader");
            }
            if (comparison_source_diffuse && ready) {
                ID3D11DeviceContext_PSSetShaderResources(context,15,1,&saved_view);
                if (saved_view) ID3D11ShaderResourceView_Release(saved_view);
            }
        }
        ID3D11DeviceContext_PSSetShader(context,shader,NULL,0);
        if (indexed) original_draw_indexed(context,count,start,base_vertex); else original_draw(context,count,start);
        if (comparison_source_diffuse || live_source_diffuse) { fputs(",",file); dump_texture(texture,base,file,"restored_override",3,NULL); }
        fputs("]\n",file); fclose(file);
        composition_output_id=camera.id;
    }
    ID3D11Texture2D_Release(texture); ID3D11PixelShader_Release(shader);
}
static void capture_draw_target(ID3D11DeviceContext *context,const Camera *camera,unsigned kind,const char *suffix) {
    ID3D11RenderTargetView *targets[2]={0}; ID3D11DeviceContext_OMGetRenderTargets(context,2,targets,NULL);
    char rain_label[32];
    if (kind>=23 && kind<=25) {
        if (rain_draw_variant) snprintf(rain_label,sizeof(rain_label),"%s-%u",rain_labels[kind-23],rain_draw_variant);
        else snprintf(rain_label,sizeof(rain_label),"%s",rain_labels[kind-23]);
    }
    const char *label=kind>=23 && kind<=25?rain_label:reference_capture?reference_labels[kind]:kind==22?"aberration":kind==13?"sun-shadow":kind==18?"god-rays-apply":kind==19?"god-rays":kind==20?"god-rays-mask":kind==12?"volume":kind==9?"present":kind==3?"fog":kind==1?"cloud":"post";
    char base[MAX_PATH+48],path[MAX_PATH+80];
    if (kind==26) snprintf(base,sizeof(base),"%s.local-light-%u-%s",camera->output,local_light_variant,suffix);
    else if (kind==21) snprintf(base,sizeof(base),"%s.emissive-%u-%s",camera->output,emissive_draw_variant,suffix);
    else snprintf(base,sizeof(base),"%s.%s-%s",camera->output,label,suffix);
    snprintf(path,sizeof(path),"%s.targets.json",base); FILE *file=fopen(path,"w");
    if (file) fputs("[",file);
    unsigned count=0;
    for (unsigned i=0;i<2;++i) if (targets[i]) {
        ID3D11Resource *resource=NULL; ID3D11Texture2D *texture=NULL; ID3D11RenderTargetView_GetResource(targets[i],&resource);
        if (SUCCEEDED(ID3D11Resource_QueryInterface(resource,&IID_ID3D11Texture2D,(void **)&texture))) {
            if (file) { if (count++) fputs(",",file); dump_texture(texture,base,file,"draw_target",i,NULL); }
            D3D11_RENDER_TARGET_VIEW_DESC view; ID3D11RenderTargetView_GetDesc(targets[i],&view);
            if (!i) {
                snprintf(path,sizeof(path),"%s.view.json",base); FILE *metadata=fopen(path,"w");
                if (metadata) { fprintf(metadata,"{\"format\":%u,\"dimension\":%u}\n",view.Format,view.ViewDimension); fclose(metadata); }
            }
            ID3D11Texture2D_Release(texture);
        }
        ID3D11Resource_Release(resource); ID3D11RenderTargetView_Release(targets[i]);
    }
    if (file) { fputs("]\n",file); fclose(file); }
}
static void capture_post_output(ID3D11DeviceContext *context) {
    if (!post_output_pending) return;
    Camera camera;
    EnterCriticalSection(&lock); camera=applied; LeaveCriticalSection(&lock);
    unsigned pending=post_output_pending; post_output_pending=0;
    if (camera.id!=pending) return;
    capture_draw_target(context,&camera,post_output_kind,"output");
    if (reference_capture && post_output_kind==9) {
        EnterCriticalSection(&lock); buffers_id=camera.id; LeaveCriticalSection(&lock);
    }
}
// Original reference captures use r_use_god_rays off. Native 0.26 applies
// this optional effect earlier, so omitting it only in the post shader is insufficient.
static int skip_legacy_god_rays(ID3D11DeviceContext *context) {
    if (!disable_legacy_god_rays || reference_capture) return 0;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return 0;
    unsigned flag=0,size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&god_rays_apply_guid,&size,&flag);
    ID3D11PixelShader_Release(shader);
    if (FAILED(hr) || flag!=1) return 0;
    ID3D11BlendState *blend=NULL; ID3D11DeviceContext_OMGetBlendState(context,&blend,NULL,NULL);
    if (!blend) return 0;
    D3D11_BLEND_DESC desc; ID3D11BlendState_GetDesc(blend,&desc); ID3D11BlendState_Release(blend);
    const D3D11_RENDER_TARGET_BLEND_DESC *target=&desc.RenderTarget[0];
    return target->BlendEnable && target->SrcBlend==D3D11_BLEND_ONE &&
        target->DestBlend==D3D11_BLEND_ONE && target->BlendOp==D3D11_BLEND_OP_ADD;
}
#include "legacy_local_diffuse.c"
#include "legacy_ambient_diffuse.c"
#include "legacy_sun_diffuse.c"
#include "legacy_rain_shells.c"
typedef struct { ID3D11ShaderResourceView *view; ID3D11PixelShader *shader; int bound; } FullDiffuseState;
static void bind_full_diffuse(ID3D11DeviceContext *context,FullDiffuseState *state) {
    memset(state,0,sizeof(*state));
    if (!live_source_diffuse) return;
    ID3D11PixelShader *shader=NULL; ID3D11DeviceContext_PSGetShader(context,&shader,NULL,NULL);
    if (!shader) return;
    unsigned flag=0; UINT size=sizeof(flag);
    HRESULT hr=ID3D11PixelShader_GetPrivateData(shader,&composition_guid,&size,&flag);
    if (FAILED(hr) || flag!=1) { ID3D11PixelShader_Release(shader); return; }
    ID3D11ShaderResourceView *view=source_full_diffuse_view();
    if (view) {
        ID3D11DeviceContext_PSGetShaderResources(context,15,1,&state->view);
        ID3D11DeviceContext_PSSetShaderResources(context,15,1,&view); state->bound=1;
        ID3D11PixelShader_Release(shader);
    } else {
        // Menu/loading has no scene accumulator. Keep native composition until
        // every producer completed for this frame; never sample stale lighting.
        state->shader=shader;
        ID3D11DeviceContext_PSSetShader(context,native_composition_shader,NULL,0);
    }
    Camera camera; EnterCriticalSection(&lock); camera=applied;
    int take=camera.enabled && capture_frame_id==camera.id && captured_id!=camera.id && draw_ids[7]!=camera.id;
    LeaveCriticalSection(&lock);
    if (take) {
        char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.full-diffuse-live.json",camera.output);
        FILE *file=fopen(path,"w");
        if (file) {
            fprintf(file,"{\"present\":%u,\"diffuse_present\":%u,\"source_diffuse_bound\":%s,\"native_fallback\":%s}\n",present_calls,sun_aux_frame,view?"true":"false",view?"false":"true");
            fclose(file);
        }
        if (!view) error("live source diffuse capture used native fallback");
    }
}
static void restore_full_diffuse(ID3D11DeviceContext *context,FullDiffuseState *state) {
    if (state->bound) {
        ID3D11DeviceContext_PSSetShaderResources(context,15,1,&state->view);
        if (state->view) ID3D11ShaderResourceView_Release(state->view);
    }
    if (state->shader) {
        ID3D11DeviceContext_PSSetShader(context,state->shader,NULL,0);
        ID3D11PixelShader_Release(state->shader);
    }
}
#include "remains_texture_capture.c"
static void STDMETHODCALLTYPE draw(ID3D11DeviceContext *context,UINT count,UINT start) {
    observe_local_fresnel_one(context);
    capture_remains_texture(context);
    prepare_early_legacy_ssao(context);
    render_sun_projection(context,count,start,0,0);
    TranslucencyState translucency; bind_translucency(context,&translucency);
    LegacyExtraState state; bind_legacy_extra(context,&state);
    VolumeState volume; bind_volume(context,&volume);
    LocalDiffuseDraw local; prepare_local_diffuse(context,&local);
    AmbientDiffuseDraw ambient; prepare_ambient_diffuse(context,&ambient);
    if (local.ready) capture_ambient_diffuse(context);
    render_sun_aux(context,count,start,0,0);
    int source_rain_ready=render_source_rain(context);
    FullDiffuseState full; bind_full_diffuse(context,&full);
    capture_post_draw(context,count,start,0,0); if (!(source_rain_ready && apply_source_rain(context)) && !skip_legacy_god_rays(context)) original_draw(context,count,start); capture_post_output(context);
    finish_local_diffuse(context,&local,count,start,0,0);
    finish_ambient_diffuse(context,&ambient,count,start,0,0);
    capture_composition_outputs(context,count,start,0,0); restore_full_diffuse(context,&full); restore_extra(context,&state); restore_translucency(context,&translucency); restore_volume(context,&volume);
}
static void STDMETHODCALLTYPE draw_indexed(ID3D11DeviceContext *context,UINT count,UINT start,INT base) {
    observe_local_fresnel_one(context);
    capture_remains_texture(context);
    prepare_early_legacy_ssao(context);
    render_sun_projection(context,count,start,base,1);
    TranslucencyState translucency; bind_translucency(context,&translucency);
    LegacyExtraState state; bind_legacy_extra(context,&state);
    VolumeState volume; bind_volume(context,&volume);
    LocalDiffuseDraw local; prepare_local_diffuse(context,&local);
    AmbientDiffuseDraw ambient; prepare_ambient_diffuse(context,&ambient);
    if (local.ready) capture_ambient_diffuse(context);
    render_sun_aux(context,count,start,base,1);
    int source_rain_ready=render_source_rain(context);
    FullDiffuseState full; bind_full_diffuse(context,&full);
    capture_post_draw(context,count,start,base,1); if (!(source_rain_ready && apply_source_rain(context)) && !skip_legacy_god_rays(context)) original_draw_indexed(context,count,start,base); capture_post_output(context);
    finish_local_diffuse(context,&local,count,start,base,1);
    finish_ambient_diffuse(context,&ambient,count,start,base,1);
    capture_composition_outputs(context,count,start,base,1); restore_full_diffuse(context,&full); restore_extra(context,&state); restore_translucency(context,&translucency); restore_volume(context,&volume);
}
static int replace_method(void *object,size_t offset,void *hook,void **original) {
    void **method=(void **)((char *)*(void **)object+offset); DWORD protection;
    if (!VirtualProtect(method,sizeof(void *),PAGE_READWRITE,&protection)) return 0;
    *original=*method; *method=hook; VirtualProtect(method,sizeof(void *),protection,&protection);
    return 1;
}
static int load_shader_override(const char *variable,void **code,size_t *size) {
    char path[MAX_PATH];
    if (GetEnvironmentVariableA(variable,path,sizeof(path))) {
        FILE *file=fopen(path,"rb");
        if (!file) { error("legacy atmosphere shader missing"); return 0; }
        fseek(file,0,SEEK_END); long length=ftell(file); rewind(file);
        if (length<32 || length>1024*1024) { fclose(file); return 0; }
        *code=malloc((size_t)length); *size=(size_t)length;
        int ok=*code && fread(*code,1,*size,file)==*size;
        fclose(file);
        if (!ok || memcmp(*code,"DXBC",4) || *(uint32_t *)((char *)*code+24)!=*size) {
            error("invalid legacy atmosphere shader"); return 0;
        }
    }
    return 1;
}
static int configure_grainless_aberration(int original_tail) {
    char directory[MAX_PATH];
    DWORD length=GetEnvironmentVariableA("SRP_LEGACY_GRAINLESS_ABERRATION_DIR",directory,sizeof(directory));
    if (!length) return 1;
    if (length>=sizeof(directory) || !original_tail) {
        error("grainless aberration requires the paired original post tail"); return 0;
    }
    for (unsigned i=0;i<sizeof(native_aberration_shaders)/sizeof(native_aberration_shaders[0]);++i) {
        unsigned paired=native_aberration_shaders[i].grainless;
        if (paired==i) continue;
        char path[MAX_PATH];
        int count=snprintf(path,sizeof(path),"%s/%s",directory,native_aberration_shaders[paired].file);
        if (count<0 || (size_t)count>=sizeof(path)) return 0;
        FILE *file=fopen(path,"rb");
        if (!file) { error("native grainless aberration shader missing"); return 0; }
        size_t size=native_aberration_shaders[paired].size;
        void *code=malloc(size);
        int ok=code && fread(code,1,size,file)==size && fgetc(file)==EOF;
        fclose(file);
        if (!ok || memcmp(code,"DXBC",4) || native_aberration_index(code,size)!=(int)paired) {
            free(code); error("unexpected native grainless aberration shader"); return 0;
        }
        grainless_aberration_code[i]=code;
    }
    return 1;
}
#include "legacy_ssao.c"

static int install_shader_capture(ID3D11Device *device,ID3D11DeviceContext *context) {
    char enabled[8];
    if (!GetEnvironmentVariableA("SRP_CAPTURE_BUFFERS",enabled,sizeof(enabled)) || strcmp(enabled,"1")) return 1;
    reference_capture=!strcmp(build,"010b") || !strcmp(build,"011a");
    capture_materials_enabled=GetEnvironmentVariableA("SRP_CAPTURE_MATERIALS",enabled,sizeof(enabled)) && !strcmp(enabled,"1");
    capture_rain_enabled=GetEnvironmentVariableA("SRP_CAPTURE_RAIN",enabled,sizeof(enabled)) && !strcmp(enabled,"1");
    capture_local_lights_enabled=GetEnvironmentVariableA("SRP_CAPTURE_LOCAL_LIGHTS",enabled,sizeof(enabled)) && !strcmp(enabled,"1");
    if (capture_local_lights_enabled && strcmp(build,"026g")) { error("local-light observer requires native 026g"); return 0; }
    if ((capture_materials_enabled || capture_rain_enabled || capture_local_lights_enabled) && !replace_method(device,offsetof(ID3D11DeviceVtbl,CreateVertexShader),create_material_vs,(void **)&original_create_vs)) return 0;
    if (reference_capture) {
        return replace_method(device,offsetof(ID3D11DeviceVtbl,CreatePixelShader),create_ps,(void **)&original_create_ps) &&
            replace_method(context,offsetof(ID3D11DeviceContextVtbl,Draw),draw,(void **)&original_draw) &&
            replace_method(context,offsetof(ID3D11DeviceContextVtbl,DrawIndexed),draw_indexed,(void **)&original_draw_indexed);
    }
    if (!load_shader_override("SRP_LEGACY_020E_AMBIENT_APPLY",&factory_020e_ambient_apply_code,&factory_020e_ambient_apply_size)) return 0;
    const unsigned char source_ambient_apply_digest[]={0x4d,0xae,0x6e,0x51,0x66,0x11,0xce,0x1c,0x47,0xc3,0xd7,0x59,0xb1,0x66,0xb8,0x8d};
    if (factory_020e_ambient_enabled != (factory_020e_ambient_apply_code!=NULL) ||
        (factory_020e_ambient_apply_code && (factory_020e_ambient_apply_size!=704 ||
            memcmp((unsigned char *)factory_020e_ambient_apply_code+4,source_ambient_apply_digest,16)))) {
        error("020e ambient requires paired stage and exact source apply shader"); return 0;
    }
    if (!load_shader_override("SRP_LEGACY_LOCAL_DIFFUSE",&legacy_local_diffuse_code,&legacy_local_diffuse_size)) return 0;
    if (legacy_local_diffuse_code && (strcmp(build,"026g") || !contains(legacy_local_diffuse_code,legacy_local_diffuse_size,"SourceSpotConstants"))) { error("source local diffuse requires 026g and the source spot adapter"); return 0; }
    disable_legacy_god_rays=GetEnvironmentVariableA("SRP_LEGACY_DISABLE_GOD_RAYS",enabled,sizeof(enabled)) && !strcmp(enabled,"1");
    if (!configure_volume_filter() || (volume_filter_enabled && !replace_method(device,offsetof(ID3D11DeviceVtbl,CreateVertexShader),volume_filter_create_vs,(void **)&volume_filter_original_create_vs))) return 0;
    capture_emissives_enabled=GetEnvironmentVariableA("SRP_CAPTURE_EMISSIVES",enabled,sizeof(enabled)) && !strcmp(enabled,"1");
    DWORD volume_assets_length=GetEnvironmentVariableA("SRP_LEGACY_VOLUME_ASSETS",legacy_volume_assets,sizeof(legacy_volume_assets));
    if (!load_shader_override("SRP_LEGACY_VOLUME_CONE",&legacy_volume_code,&legacy_volume_size)) return 0;
    if (!!legacy_volume_code!=!!volume_assets_length || volume_assets_length>=sizeof(legacy_volume_assets)) {
        error("original cone shader requires its paired falloff assets"); return 0;
    }
    if (!load_shader_override("SRP_LEGACY_ATMOSPHERE_LUT",&legacy_lut_code,&legacy_lut_size) ||
        !load_shader_override("SRP_LEGACY_GEOMETRY_FOG",&legacy_fog_code,&legacy_fog_size) ||
        !load_shader_override("SRP_LEGACY_SKY",&legacy_sky_code,&legacy_sky_size) ||
        !load_shader_override("SRP_LEGACY_RAIN_CLOUDS",&legacy_cloud_code,&legacy_cloud_size) ||
        !load_shader_override("SRP_LEGACY_PRESENT_GAMMA",&legacy_present_code,&legacy_present_size) ||
        !load_shader_override("SRP_LEGACY_RAIN_TONEMAP",&legacy_post_code,&legacy_post_size) ||
        !load_shader_override("SRP_LEGACY_PROBE_DIFFUSE",&legacy_probe_code,&legacy_probe_size) ||
        !load_shader_override("SRP_LEGACY_PROBE_DIFFUSE_BOX",&legacy_probe_box_code,&legacy_probe_box_size) ||
        !load_shader_override("SRP_LEGACY_SUN_PROJECTION",&legacy_sun_projection_code,&legacy_sun_projection_size) ||
        !load_shader_override("SRP_LEGACY_COMPOSITION",&legacy_composition_code,&legacy_composition_size) ||
        !load_shader_override("SRP_LEGACY_COMPOSITION_REFERENCE",&comparison_composition_code,&comparison_composition_size) ||
        !load_shader_override("SRP_LEGACY_AMBIENT_DECODE",&legacy_ambient_decode_code,&legacy_ambient_decode_size)) return 0;
    DWORD grain_path_length=GetEnvironmentVariableA("SRP_LEGACY_POST_GRAIN",legacy_post_grain_path,sizeof(legacy_post_grain_path));
    int original_tail=legacy_post_code && contains(legacy_post_code,legacy_post_size,"t_legacy_grain");
    if (original_tail!=!!legacy_present_code || original_tail!=!!grain_path_length || grain_path_length>=sizeof(legacy_post_grain_path)) {
        error("original post tail requires paired grading, grain texture and final gamma shaders"); return 0;
    }
    if (!configure_grainless_aberration(original_tail)) return 0;
    int full_diffuse_shader=legacy_composition_code && contains(legacy_composition_code,legacy_composition_size,"t_full_encoded_diffuse");
    live_source_diffuse=GetEnvironmentVariableA("SRP_LEGACY_FULL_DIFFUSE",enabled,sizeof(enabled))==1 && enabled[0]=='1';
    if (live_source_diffuse!=full_diffuse_shader) {
        error("full diffuse composition requires explicit SRP_LEGACY_FULL_DIFFUSE opt-in and paired shader"); return 0;
    }
    int source_translucency=full_diffuse_shader || (legacy_composition_code && contains(legacy_composition_code,legacy_composition_size,"t_legacy_sun_projection"));
    if (source_translucency!=!!legacy_sun_projection_code) {
        error("source translucency requires its paired sun projection helper"); return 0;
    }
    if (source_translucency) {
        char path[MAX_PATH];
        DWORD length=GetEnvironmentVariableA("SRP_LEGACY_TRANSLUCENCY_CONSTANTS",path,sizeof(path));
        FILE *file=length && length<sizeof(path)?fopen(path,"rb"):NULL;
        if (!file) { error("source translucency constants missing"); return 0; }
        int ok=fread(legacy_translucency_color,1,sizeof(legacy_translucency_color),file)==sizeof(legacy_translucency_color) && fgetc(file)==EOF;
        fclose(file);
        for (unsigned i=0;i<4;++i) ok=ok && isfinite(legacy_translucency_color[i]) && legacy_translucency_color[i]>=0;
        if (!ok) { error("invalid source translucency sun constants"); return 0; }
    }
    if (comparison_composition_code && !legacy_composition_code) {
        error("composition comparison requires a composition override"); return 0;
    }
    int encoded_sphere=legacy_probe_code && contains(legacy_probe_code,legacy_probe_size,"encoded_color_parameters");
    int encoded_box=legacy_probe_box_code && contains(legacy_probe_box_code,legacy_probe_box_size,"encoded_color_parameters");
    if ((legacy_ambient_decode_code && (!encoded_sphere || !encoded_box)) ||
        ((encoded_sphere || encoded_box) && !legacy_ambient_decode_code)) {
        error("encoded probes require paired sphere, box and ambient decoder"); return 0;
    }
    if (!configure_legacy_ssao()) return 0;
    if (!configure_ambient_diffuse()) return 0;
    if (!configure_sun_aux()) return 0;
    if (live_source_diffuse && (!sun_aux_enabled || !sun_aux_translucency)) {
        error("live source diffuse requires complete ambient/local/sun/translucency auxiliary"); return 0;
    }
    if (!configure_source_rain()) return 0;
    comparison_source_diffuse=GetEnvironmentVariableA("SRP_DIAGNOSTIC_COMPOSITION_SOURCE_DIFFUSE",enabled,sizeof(enabled))==1 && enabled[0]=='1';
    if (comparison_source_diffuse && (!comparison_composition_code || !sun_aux_enabled || !sun_aux_translucency)) {
        error("source diffuse comparison requires reference shader and sun/translucency auxiliary"); return 0;
    }
    if (!configure_local_fresnel_one()) return 0;
    legacy_ambient_decode_enabled=legacy_ambient_decode_code!=NULL;
    legacy_fog_enabled=legacy_fog_code!=NULL;
    return replace_method(device,offsetof(ID3D11DeviceVtbl,CreatePixelShader),create_ps,(void **)&original_create_ps) &&
        replace_method(context,offsetof(ID3D11DeviceContextVtbl,Draw),draw,(void **)&original_draw) &&
        replace_method(context,offsetof(ID3D11DeviceContextVtbl,DrawIndexed),draw_indexed,(void **)&original_draw_indexed);
}
