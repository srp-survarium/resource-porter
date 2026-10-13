"""Prepare a reversible, single-material Factory tree-card wind correction."""
from pathlib import Path
import argparse,hashlib,json,re,struct,subprocess
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--exe',type=Path,required=True);p.add_argument('--archive',type=Path,required=True)
p.add_argument('--source-db',type=Path,required=True);p.add_argument('--native-db',type=Path,required=True)
p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
resource='material_instances/ported/v010b/flora/sosna_bark_lod.material'
key='material.g_stage.effect.wind_motion.value';shadow='material.g_stage.is_cast_shadow.value'
def leaves(db,path):
 return dict(line.split('\t',1) for line in subprocess.check_output([str(a.exe),'dump-db-config','--db',str(db),'--path',path,'--leaves'],text=True).splitlines())
source=leaves(a.source_db,'material_instances/flora/sosna_bark_lod.material')
native=leaves(a.native_db,'material_instances/flora/sosna_bark_lod_not_move.material')
current=leaves(a.archive,resource)
assert source[key]==native[key]=='i32:0' and current[key]=='i32:2'
assert source[shadow]==current[shadow]=='bool:true'
models={}
for n in range(1,6):
 path=f'models/level/03/back_{n:02}.model/settings';m=leaves(a.source_db,path)
 assert m['material_settings.bark_sosna_lod_s.material_name']=='string:"flora/sosna_bark_lod"'
 ported=leaves(a.archive,path.replace('models/','models/ported/v010b/',1))
 assert ported['material_settings.bark_sosna_lod_s.material_name']=='string:"ported/v010b/flora/sosna_bark_lod"'
 models[path]={'source':m,'ported':ported}
baseline=a.output/'baseline.material';fixedpath=a.output/'fixed.material'
subprocess.run([str(a.exe),'extract','--db',str(a.archive),'--path',resource,'--output',str(baseline)],check=True,stdout=subprocess.DEVNULL)
raw=baseline.read_bytes();fixed=bytearray(raw);offset=0
for part in key.split('.'):
 table,_,_,kind,count=struct.unpack_from('<QQIHH',raw,offset);assert kind==3
 found=[]
 for i in range(count):
  child=table+24*i;name=struct.unpack_from('<Q',raw,child+8)[0]
  if raw[name:raw.index(0,name)].decode()==part:found.append(child)
 assert len(found)==1;offset=found[0]
assert struct.unpack_from('<H',raw,offset+20)[0]==1 and struct.unpack_from('<Q',raw,offset)[0]==2
struct.pack_into('<Q',fixed,offset,0);fixedpath.write_bytes(fixed)
assert sum(x!=y for x,y in zip(raw,fixed))==1
with a.archive.open('rb') as f:header=f.read(24);fat=f.read(struct.unpack_from('<I',header,20)[0])
stat=subprocess.check_output([str(a.exe),'stat','--db',str(a.archive),'--path',resource],text=True)
assert 'compressed: false, inlined: false' in stat
position=int(re.search(r'^pos_in_db=(\d+)$',stat,re.M)[1]);needle=struct.pack('<I4xQ',len(raw),position);at=fat.find(needle);assert at>=0 and fat.find(needle,at+1)<0
entry={'resource':resource,'position':position,'crc_offset':at+24+16,'baseline':baseline.name,'fixed':fixedpath.name,'baseline_sha256':hashlib.sha256(raw).hexdigest(),'fixed_sha256':hashlib.sha256(fixed).hexdigest()}
(a.output/'patch.json').write_text(json.dumps({'scope':'Only the authored-static sosna_bark_lod material wind selector changes from 2 to 0. Original cast-shadow=true and every other material byte remain unchanged. Prepare independently for each archive; offsets and hashes are archive-specific.','entries':[entry]},indent=2)+'\n')
(a.output/'contracts.json').write_text(json.dumps({'source':source,'native_stationary_template':native,'before':current,'models':models},indent=2)+'\n')
print(a.output/'patch.json')
