import sys, struct, numpy as np
def c565(c):
    return np.stack([((c>>11)&31)*255//31, ((c>>5)&63)*255//63, (c&31)*255//31],-1).astype(np.float32)
def bc1_colors(blocks, alpha_mode):
    c0=blocks[:,0].astype(np.uint32)|(blocks[:,1].astype(np.uint32)<<8); c1=blocks[:,2].astype(np.uint32)|(blocks[:,3].astype(np.uint32)<<8)
    a,b=c565(c0),c565(c1); four=(c0>c1)|(~alpha_mode)
    p2=np.where(four[:,None],(2*a+b)/3,(a+b)/2); p3=np.where(four[:,None],(a+2*b)/3,0)
    pal=np.stack([a,b,p2,p3],1)
    idx=blocks[:,4:8].astype(np.uint32); bits=idx[:,0]|(idx[:,1]<<8)|(idx[:,2]<<16)|(idx[:,3]<<24)
    sel=np.stack([(bits>>(2*i))&3 for i in range(16)],1)
    return np.take_along_axis(pal,sel[:,:,None].repeat(3,2),1)
def bc3_alpha(blocks):
    a0=blocks[:,0].astype(np.float32); a1=blocks[:,1].astype(np.float32)
    pal=np.zeros((len(blocks),8),np.float32); pal[:,0]=a0; pal[:,1]=a1
    big=a0>a1
    for i in range(1,7): pal[:,i+1]=np.where(big,((7-i)*a0+i*a1)/7, ((5-i)*a0+i*a1)/5 if i<5 else 0)
    pal[:,6]=np.where(big,pal[:,6],0); pal[:,7]=np.where(big,pal[:,7],255)
    bits=np.zeros(len(blocks),np.uint64)
    for i in range(6): bits|=blocks[:,2+i].astype(np.uint64)<<np.uint64(8*i)
    sel=np.stack([(bits>>np.uint64(3*i))&np.uint64(7) for i in range(16)],1).astype(np.int64)
    return np.take_along_axis(pal,sel,1)
def decode(data,w,h,fmt):
    bw,bh=max(1,(w+3)//4),max(1,(h+3)//4); bs=8 if fmt==b'DXT1' else 16
    blocks=np.frombuffer(data[:bw*bh*bs],np.uint8).reshape(-1,bs)
    if fmt==b'DXT1': rgb=bc1_colors(blocks,np.ones(len(blocks),bool)); a=np.full((len(blocks),16),255.)
    else: rgb=bc1_colors(blocks[:,8:],np.zeros(len(blocks),bool)); a=bc3_alpha(blocks[:,:8])
    px=np.concatenate([rgb,a[:,:,None]],2).reshape(bh,bw,4,4,4).transpose(0,2,1,3,4).reshape(bh*4,bw*4,4)
    return px[:h,:w]
for path in sys.argv[1:]:
    b=open(path,'rb').read(); h,w=struct.unpack_from('<II',b,12); mips=struct.unpack_from('<I',b,28)[0]; fmt=b[84:88]
    print(f'{path.split("/")[-1]}: {w}x{h} {fmt.decode()} mips={mips}')
    off=128; prev=None
    for m in range(mips):
        mw,mh=max(1,w>>m),max(1,h>>m); bs=8 if fmt==b'DXT1' else 16; size=max(1,(mw+3)//4)*max(1,(mh+3)//4)*bs
        img=decode(b[off:off+size],mw,mh,fmt); off+=size
        line=f'  mip{m} {mw:4d}: mean {img[...,:3].mean():6.1f} std {img[...,:3].std():5.1f} alpha mean {img[...,3].mean():5.1f}'
        if prev is not None and mw>=4:
            box=prev.reshape(mh,2,mw,2,4).mean((1,3))
            line+=f'  |mip - box(prev)| rgb {np.abs(img[...,:3]-box[...,:3]).mean():5.2f} a {np.abs(img[...,3]-box[...,3]).mean():5.2f}'
        print(line); prev=img
    print('  trailing bytes', len(b)-off)
