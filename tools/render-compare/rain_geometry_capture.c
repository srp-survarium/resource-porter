// Rain's live vertex constants and geometry are required to compare particle
// size, density and motion; matching pixel shaders alone does not establish it.
static void dump_rain_buffer(ID3D11DeviceContext *context,ID3D11Buffer *buffer,const char *path) {
    if (!buffer) return;
    D3D11_BUFFER_DESC desc; ID3D11Buffer_GetDesc(buffer,&desc);
    if (!desc.ByteWidth || desc.ByteWidth>16*1024*1024) return;
    desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;desc.MiscFlags=0;
    ID3D11Device *device=NULL;ID3D11Buffer *staging=NULL;
    ID3D11DeviceContext_GetDevice(context,&device);
    HRESULT hr=ID3D11Device_CreateBuffer(device,&desc,NULL,&staging);
    if(SUCCEEDED(hr)) {
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)buffer);
        D3D11_MAPPED_SUBRESOURCE mapped;
        if(SUCCEEDED(ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped))) {
            if(!write_bytes(path,mapped.pData,desc.ByteWidth)) error("cannot save rain geometry buffer");
            ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        }
        ID3D11Buffer_Release(staging);
    }
    ID3D11Device_Release(device);
}
static void capture_rain_geometry(ID3D11DeviceContext *context,const char *base,UINT count,UINT start,INT base_vertex,int indexed) {
    char path[MAX_PATH+80];ID3D11Buffer *buffers[4]={0};
    ID3D11DeviceContext_VSGetConstantBuffers(context,0,4,buffers);
    for(unsigned i=0;i<4;++i) if(buffers[i]) {
        snprintf(path,sizeof(path),"%s.vs-cb%u.bin",base,i);dump_rain_buffer(context,buffers[i],path);ID3D11Buffer_Release(buffers[i]);
    }
    ID3D11Buffer *vertex=NULL,*index=NULL;UINT stride=0,offset=0,index_offset=0;DXGI_FORMAT format;D3D11_PRIMITIVE_TOPOLOGY topology;
    ID3D11DeviceContext_IAGetVertexBuffers(context,0,1,&vertex,&stride,&offset);
    ID3D11DeviceContext_IAGetIndexBuffer(context,&index,&format,&index_offset);
    ID3D11DeviceContext_IAGetPrimitiveTopology(context,&topology);
    snprintf(path,sizeof(path),"%s.geometry.json",base);FILE *f=fopen(path,"w");
    if(f){fprintf(f,"{\"indexed\":%d,\"count\":%u,\"start\":%u,\"base_vertex\":%d,\"stride\":%u,\"vertex_offset\":%u,\"index_offset\":%u,\"index_format\":%u,\"topology\":%u,\"present\":%u}\n",indexed,count,start,base_vertex,stride,offset,index_offset,format,topology,present_calls);fclose(f);}
    if(vertex){snprintf(path,sizeof(path),"%s.vertices.bin",base);dump_rain_buffer(context,vertex,path);ID3D11Buffer_Release(vertex);}
    if(index){snprintf(path,sizeof(path),"%s.indices.bin",base);dump_rain_buffer(context,index,path);ID3D11Buffer_Release(index);}
}
