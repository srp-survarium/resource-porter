// Read-only native local-light contracts. Each draw matters: one PS can serve
// many lights with different transforms, colors, ranges and shadow maps.
#include "local_light_index.h"
static int capture_local_lights_enabled;
static unsigned local_light_request_id,local_light_count,local_light_variant;
static unsigned local_light_first_present,local_light_last_present;
static unsigned local_light_created[5],local_light_observed[5];
#define LOCAL_LIGHT_CAPTURE_CAP 32
static int local_light_index(const void *code,SIZE_T size) {
    if ((!capture_local_lights_enabled && !legacy_local_diffuse_code) || size<32) return -1;
    for (unsigned i=0;i<sizeof(local_light_shaders)/sizeof(local_light_shaders[0]);++i)
        if (size==local_light_shaders[i].size && !memcmp((const char *)code+4,local_light_shaders[i].digest,16)) return (int)i;
    return -1;
}
static void report_local_lights(const Camera *camera) {
    char path[MAX_PATH+48]; snprintf(path,sizeof(path),"%s.local-light-counts.json",camera->output);
    FILE *file=fopen(path,"w"); if (!file) { error("cannot save local light coverage"); return; }
    fprintf(file,"{\"camera_id\":%u,\"families\":[\"point\",\"spot\",\"plane-spot\",\"obb\",\"capsule\"],\"observed_draws\":%u,\"capture_cap\":%u,\"overflow\":%u,\"first_present\":%u,\"last_present\":%u,\"single_frame\":%s,\"created_by_family\":[%u,%u,%u,%u,%u],\"observed_by_family\":[%u,%u,%u,%u,%u]}\n",
        camera->id,local_light_count,LOCAL_LIGHT_CAPTURE_CAP,local_light_count>LOCAL_LIGHT_CAPTURE_CAP?local_light_count-LOCAL_LIGHT_CAPTURE_CAP:0,
        local_light_first_present,local_light_last_present,local_light_first_present==local_light_last_present?"true":"false",
        local_light_created[0],local_light_created[1],local_light_created[2],local_light_created[3],local_light_created[4],
        local_light_observed[0],local_light_observed[1],local_light_observed[2],local_light_observed[3],local_light_observed[4]);
    fclose(file);
}
static void begin_local_light_capture(const Camera *camera) {
    if (!capture_local_lights_enabled) return;
    if (local_light_request_id!=camera->id) {
        local_light_request_id=camera->id; local_light_count=0;
        memset(local_light_observed,0,sizeof(local_light_observed));
        local_light_first_present=local_light_last_present=present_calls;
        report_local_lights(camera); // Explicit zero even if no local lights draw.
    } else if (local_light_last_present!=present_calls) {
        local_light_last_present=present_calls; report_local_lights(camera);
    }
}
static void capture_local_light_state(ID3D11DeviceContext *context,const char *base,unsigned identity) {
    char path[MAX_PATH+80]; snprintf(path,sizeof(path),"%s.state.json",base);
    FILE *file=fopen(path,"w"); if (!file) { error("cannot save local light state"); return; }
    ID3D11DepthStencilState *depth=NULL; UINT reference;
    ID3D11DeviceContext_OMGetDepthStencilState(context,&depth,&reference);
    fprintf(file,"{\"identity_index\":%u,\"family\":%u,\"present\":%u,\"stencil_reference\":%u,\"depth_stencil\":",identity,local_light_shaders[identity].family,present_calls,reference);
    if (depth) {
        D3D11_DEPTH_STENCIL_DESC d; ID3D11DepthStencilState_GetDesc(depth,&d); ID3D11DepthStencilState_Release(depth);
        fprintf(file,"{\"depth_enabled\":%u,\"depth_write_mask\":%u,\"depth_function\":%u,\"stencil_enabled\":%u,\"read_mask\":%u,\"write_mask\":%u,\"front\":[%u,%u,%u,%u],\"back\":[%u,%u,%u,%u]}",d.DepthEnable,d.DepthWriteMask,d.DepthFunc,d.StencilEnable,d.StencilReadMask,d.StencilWriteMask,d.FrontFace.StencilFailOp,d.FrontFace.StencilDepthFailOp,d.FrontFace.StencilPassOp,d.FrontFace.StencilFunc,d.BackFace.StencilFailOp,d.BackFace.StencilDepthFailOp,d.BackFace.StencilPassOp,d.BackFace.StencilFunc);
    } else fputs("null",file);
    ID3D11BlendState *blend=NULL; FLOAT factor[4]; UINT mask;
    ID3D11DeviceContext_OMGetBlendState(context,&blend,factor,&mask);
    fprintf(file,",\"sample_mask\":%u,\"blend_factor\":[%.9g,%.9g,%.9g,%.9g],\"blend\":",mask,factor[0],factor[1],factor[2],factor[3]);
    if (blend) {
        D3D11_BLEND_DESC d; ID3D11BlendState_GetDesc(blend,&d); ID3D11BlendState_Release(blend);
        fprintf(file,"{\"alpha_to_coverage\":%u,\"independent\":%u,\"targets\":[",d.AlphaToCoverageEnable,d.IndependentBlendEnable);
        for (unsigned i=0;i<2;++i) { const D3D11_RENDER_TARGET_BLEND_DESC *t=&d.RenderTarget[i];
            fprintf(file,"%s[%u,%u,%u,%u,%u,%u,%u,%u]",i?",":"",t->BlendEnable,t->SrcBlend,t->DestBlend,t->BlendOp,t->SrcBlendAlpha,t->DestBlendAlpha,t->BlendOpAlpha,t->RenderTargetWriteMask);
        }
        fputs("]}",file);
    } else fputs("null",file);
    ID3D11RasterizerState *raster=NULL; ID3D11DeviceContext_RSGetState(context,&raster);
    fputs(",\"rasterizer\":",file);
    if (raster) {
        D3D11_RASTERIZER_DESC d; ID3D11RasterizerState_GetDesc(raster,&d); ID3D11RasterizerState_Release(raster);
        fprintf(file,"{\"fill\":%u,\"cull\":%u,\"front_ccw\":%u,\"depth_bias\":%d,\"depth_bias_clamp\":%.9g,\"slope_bias\":%.9g,\"depth_clip\":%u,\"scissor\":%u}",d.FillMode,d.CullMode,d.FrontCounterClockwise,d.DepthBias,d.DepthBiasClamp,d.SlopeScaledDepthBias,d.DepthClipEnable,d.ScissorEnable);
    } else fputs("null",file);
    fputs("}\n",file); fclose(file);
}
