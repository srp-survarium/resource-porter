// Source Factory shell motion, independently checked against frozen 0.10b draws.
#include <math.h>
#include <stdint.h>
typedef struct { double previous[3],offsets[2]; int initialized; } LegacyRainMotion;
static double rain_random(uint32_t *seed) {
    *seed=*seed*0x08088405u+1u;
    return (*seed>>12)/1048576.0;
}
static void rain_matrix3(const double a[9],const double b[9],double out[9]) {
    for (unsigned r=0;r<3;++r) for (unsigned c=0;c<3;++c) {
        out[r*3+c]=0;
        for (unsigned k=0;k<3;++k) out[r*3+c]+=a[r*3+k]*b[k*3+c];
    }
}
static void rain_source_origin(LegacyRainMotion *state,const float eye[3],const float view[12],float origin[3]) {
    double axes[2][2]={{view[8],view[10]},{view[0],view[2]}};
    if (!state->initialized) {
        for (unsigned j=0;j<3;++j) state->previous[j]=eye[j];
        state->initialized=1;
    }
    for (unsigned i=0;i<2;++i) {
        double length=hypot(axes[i][0],axes[i][1]);
        if (length) { axes[i][0]/=length; axes[i][1]/=length; }
        double value=state->offsets[i]+((eye[0]-state->previous[0])*axes[i][0]+(eye[2]-state->previous[2])*axes[i][1])*.5;
        if (value>=1) value-=trunc(value);
        else if (value<=-1) value=1-(fabs(value)-fabs(trunc(value)));
        state->offsets[i]=value;
    }
    origin[0]=(float)(eye[0]-state->offsets[0]*axes[0][0]-state->offsets[1]*axes[1][0]);
    origin[1]=eye[1];
    origin[2]=(float)(eye[2]-state->offsets[0]*axes[0][1]-state->offsets[1]*axes[1][1]);
    for (unsigned j=0;j<3;++j) state->previous[j]=eye[j];
}
static void rain_source_layers(float scene_time,const float origin[3],float world[5][12],float radii[5]) {
    double offsets[10][2]; uint32_t seed=1000;
    for (unsigned i=0;i<10;++i) {
        offsets[i][1]=rain_random(&seed)*2-1;
        offsets[i][0]=rain_random(&seed)*2-1;
    }
    seed=1000; double multiplier=1;
    for (unsigned layer=0;layer<5;++layer) {
        unsigned i=layer+5;
        double radius=i*multiplier,yaw=rain_random(&seed)*6.283185307179586;
        double x=cos(scene_time*.2)*.1+offsets[i][0]*.1,z=sin(scene_time*.2)*.1+offsets[i][1]*.1;
        double cx=cos(x),sx=sin(x),cy=cos(yaw),sy=sin(yaw),cz=cos(z),sz=sin(z);
        const double rz[9]={cz,sz,0,-sz,cz,0,0,0,1};
        const double rx[9]={1,0,0,0,cx,sx,0,-sx,cx};
        const double ry[9]={cy,0,-sy,0,1,0,sy,0,cy};
        double temp[9],rotation[9]; rain_matrix3(rz,rx,temp); rain_matrix3(temp,ry,rotation);
        for (unsigned r=0;r<3;++r) {
            for (unsigned c=0;c<3;++c) world[layer][r*4+c]=(float)(rotation[r*3+c]*2*radius*(c==1?10:1));
            world[layer][r*4+3]=origin[r];
        }
        radii[layer]=(float)radius; multiplier*=1.1;
    }
}
