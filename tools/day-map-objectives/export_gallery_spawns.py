import hashlib,json,math,os,re,subprocess
from pathlib import Path
exe='/home/sheep/Projects/survarium/resource-porter/target/release/vostok-port'
env=dict(os.environ,LD_LIBRARY_PATH='/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib')
packed='/mnt/hdd/Projects/survarium-map-pack-20261013/resources.db'
specs=[('factory-010-day',180,'/nix/store/5yk74ndm49i0vrrrmdrypzwrh8qkflxm-survarium-resources/resources.db','level_03','v010w'),('chemical-011-day',193,'/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011a/app/resources.db','level_04','v011a')]
def leaves(db,path):
 text=subprocess.check_output([exe,'dump-db-config','--db',db,'--path',path,'--leaves'],env=env,text=True)
 return {key:json.loads(value.split(':',1)[1]) for key,value in (line.split('\t',1) for line in text.splitlines())}
result={'schema_version':1,'scope':'all authored candidate battery spawn points, not only ten active batteries in one match','screenshots_per_candidate':10,'total_candidates':59,'planned_screenshots':590,'coordinate_system':'world metres; Y up','pose_status':'proposals only; must test visibility and use runtime battery centre after settling','maps':[]}
for slug,mapid,db,level,namespace in specs:
 sourcepath=f'projects/{level}/server_project';targetpath=f'projects/{namespace}_{level}/day_2/client_project';source=leaves(db,sourcepath);target=leaves(packed,targetpath)
 ids=sorted({int(re.match(r'objects.victory_item_spawners\[(\d+)\]',k)[1]) for k in source if k.startswith('objects.victory_item_spawners[')})
 spawns=[]
 for index in ids:
  old=f'objects.victory_item_spawners[{index}]';new=f'server_objects.victory_item_spawners[{index}]'
  for field in ['point_id','position','rotation','full_name','priority']:
   assert source[old+'.'+field]==target[new+'.'+field],(slug,index,field)
  pos=source[old+'.position'];rot=source[old+'.rotation'];pointid=source[old+'.point_id']
  assert target[new+'.spawned_item.position']==pos and target[new+'.spawned_item.rotation']==rot
  samples=[]
  # Eight horizontal ring views, then contextual elevated and close detail views.
  for angle in range(0,360,45):
   a=math.radians(angle);camera=[pos[0]+1.8*math.sin(a),pos[1]+.75,pos[2]+1.8*math.cos(a)]
   samples.append({'view':f'azimuth-{angle:03d}','camera':camera,'target':[pos[0],pos[1]+.18,pos[2]],'fov_degrees':65})
  samples.extend([{'view':'context-high','camera':[pos[0],pos[1]+2.4,pos[2]+3.6],'target':[pos[0],pos[1]+.15,pos[2]],'fov_degrees':65},{'view':'detail-low','camera':[pos[0]+.7,pos[1]+.35,pos[2]+.8],'target':[pos[0],pos[1]+.15,pos[2]],'fov_degrees':65}])
  for sample in samples:
   direction=[sample['target'][i]-sample['camera'][i] for i in range(3)];length=math.sqrt(sum(v*v for v in direction));sample['forward']=[v/length for v in direction]
  spawns.append({'candidate_id':f'{slug}-spawn-{pointid:02d}','source_array_index':index,'source_point_id':pointid,'source_full_name':source[old+'.full_name'],'source_name':source[old+'.name'],'priority':source[old+'.priority'],'authored_position':pos,'authored_rotation':rot,'packed_transform_matches_source':True,'diagnostic_batch':index//10,'diagnostic_point_id':index%10,'active_runtime_battery_id':None,'settled_position':None,'capture_status':'not captured','proposed_views':samples})
 result['maps'].append({'slug':slug,'map_id':mapid,'source_db':db,'source_server_project':sourcepath,'packed_db':packed,'packed_project':targetpath,'candidate_count':len(spawns),'release_active_battery_count':10,'diagnostic_batches':[{'batch':b,'candidate_ids':[s['candidate_id'] for s in spawns if s['diagnostic_batch']==b],'victory_items_count':len([s for s in spawns if s['diagnostic_batch']==b])} for b in range((len(spawns)+9)//10)],'candidates':spawns})
Path('/tmp/battery-gallery-spawns.json').write_text(json.dumps(result,indent=2)+'\n')
print([(m['slug'],m['candidate_count'],len(m['diagnostic_batches'])) for m in result['maps']])
