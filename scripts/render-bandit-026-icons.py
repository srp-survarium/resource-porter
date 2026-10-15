#!/usr/bin/env python3
"""Render the converted Bandit balaclava and add its icon to an unused atlas cell.

Only the balaclava ships; it takes cell [2, 6] of the added sixth column.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import zlib
import numpy as np
from PIL import Image, ImageDraw

spec = importlib.util.spec_from_file_location('duty_render', Path(__file__).with_name('render-duty-026-icons.py'))
renderer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(renderer)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--textures', type=Path, required=True)
    ap.add_argument('--lobby-swf', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args(); a.output.mkdir(parents=True, exist_ok=True)
    def mesh(model, surface, texture, left=False):
        p = a.bundle / f'overlay/models/ported/bandit_026/{model}.skinned_model/render/{surface}/converted_model'
        b = p.read_bytes(); n = struct.unpack_from('<I', b, 8)[0]
        raw = np.frombuffer(b[12:12+n*48], np.uint8).reshape(n,48)
        floats = raw.copy().view('<f4').reshape(n,12)
        pos = floats[:,:3].astype(float)
        faces = np.frombuffer(b,dtype='<u2',offset=24+48*n).reshape(-1,3)
        if left: faces = faces[(pos[faces,0]<0).all(axis=1)]
        tex = np.asarray(Image.open(a.textures/f'bandit_01_{texture}.tga').convert('RGB'),dtype=float)/255
        return pos, floats[:,10:12].astype(float), raw[:,28:31][:,::-1].astype(float)/127.5-1, faces, tex, np.unique(faces)
    head = [mesh('mask','mask','attach')]
    prefix,tags = renderer.swf_tags(a.lobby_swf)
    tag_index,char_id,argb = renderer.atlas_tag(tags)
    h,w = argb.shape[:2]
    assert w in (855,1026) and h == 2048
    atlas = np.zeros((h,1026,4),np.uint8);atlas[:,:w] = argb
    # Row i + 1 of the sixth column: the balaclava is [2, 6].
    specs = [('balaclava',head,15,0,0)]
    comparison=Image.new('RGB',(684,166*len(specs)),(30,33,36))
    for i,(name,meshes,yaw,pitch,roll) in enumerate(specs,1):
        assert not atlas[i*91:i*91+83,855:].any(), 'target icon cell occupied'
        render=renderer.render(meshes,yaw,pitch,roll=roll)
        render.save(a.output/f'{name}-render.png')
        shown=render.resize((342,166));comparison.paste(shown,(342,(i-1)*166),shown)
        # Matching native template thumbnails; names/positions retain their category.
        native=[(19,5)][i-1]
        row,col=native
        og=Image.fromarray(argb[:,:,[1,2,3,0]]).crop(((col-1)*171,(row-1)*91,col*171,(row-1)*91+83)).resize((342,166))
        comparison.paste(og,(0,(i-1)*166),og)
        icon=np.asarray(render.resize((171,83),Image.Resampling.LANCZOS));alpha=icon[:,:,3:4].astype(np.uint16)
        atlas[i*91:i*91+83,855:,0]=icon[:,:,3]
        atlas[i*91:i*91+83,855:,1:]=(icon[:,:,:3].astype(np.uint16)*alpha+127)//255
    comparison.save(a.output/'comparison.png')
    tags[tag_index]=(36,struct.pack('<HBHH',char_id,5,1026,h)+zlib.compress(atlas.tobytes(),9))
    body=prefix[8:]+b''.join(struct.pack('<HI',(kind<<6)|63,len(payload))+payload for kind,payload in tags)
    swf=b'CWS'+prefix[3:4]+struct.pack('<I',8+len(body))+zlib.compress(body,9)
    target=a.bundle/'overlay/flash_movies/lobby_menu.swf';target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(swf)
    _,checktags=renderer.swf_tags(target);_,_,check=renderer.atlas_tag(checktags)
    assert np.array_equal(check[:,:855],argb[:,:855])
    assert np.array_equal(check[:91,:w],argb[:91,:w])
    assert all(x==y for j,(x,y) in enumerate(zip(tags,checktags)) if j!=tag_index)
    record=json.loads((a.bundle/'port-manifest.json').read_text())
    path='flash_movies/lobby_menu.swf'
    record['files']=[f for f in record['files'] if f['path']!=path]
    record['files'].append({'path':path,'source':'rendered:Bandit-026','size':len(swf),'sha256':hashlib.sha256(swf).hexdigest()})
    record['files'].sort(key=lambda f:f['path'])
    (a.bundle/'port-manifest.json').write_text(json.dumps(record,indent=2)+'\n')

if __name__ == '__main__': main()
