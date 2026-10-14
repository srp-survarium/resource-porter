#!/usr/bin/env python3
from pathlib import Path
import argparse,json,sys,subprocess,time,hashlib,importlib.util,shutil
parser=argparse.ArgumentParser(description="Replay frozen011e poses on an unchanged managed candidate after GPU handoff.")
parser.add_argument('root',type=Path)
parser.add_argument('--package',type=Path,required=True)
parser.add_argument('--runtime',type=Path,required=True)
parser.add_argument('--port',type=int,default=2266)
parser.add_argument('--launch',action='store_true')
args=parser.parse_args()
root=args.root.resolve();runtime=args.runtime.resolve();package=args.package.resolve();launcher=package/'launch.py'
assert launcher.is_file()
for name in ['chemical','radar']:
 poses=json.loads((root/(name+'-poses.json')).read_text())
 assert 1<=len(poses)<=18 and all(p['buffers'] is False for p in poses)
 assert not (root/('target-'+name)).exists(), 'capture output already exists'
if not args.launch:
 print('Validated inputs; use --launch only after GPU handoff.');sys.exit(0)
spec=importlib.util.spec_from_file_location('capture',Path(__file__).resolve().parents[1]/'render-compare/capture.py');capture=importlib.util.module_from_spec(spec);spec.loader.exec_module(capture)
def run(*args):
 return subprocess.run([sys.executable,str(launcher),*args,'--runtime',str(runtime),'--port',str(args.port)],check=True,capture_output=True,text=True,timeout=60)
for name in ['chemical','radar']:
 out=root/('target-'+name);out.mkdir(exist_ok=False)
 key=name+'-011e-day';preset=package/'presets'/key/'preset.json';db=package/'resources.db';before=db.stat();record={'preset':json.loads(preset.read_text()),'preset_sha256':hashlib.sha256(preset.read_bytes()).hexdigest(),'database':str(db),'database_size':before.st_size,'database_mtime_ns':before.st_mtime_ns}
 (out/'provenance.json').write_text(json.dumps(record,indent=2)+'\n')
 try:
  (out/'launch.json').write_text(run('start',key,'--ui','off').stdout)
  deadline=time.monotonic()+180;last=None;since=None;reselected=False
  while time.monotonic()<deadline:
   state=json.loads((runtime/'state.json').read_text())
   if state.get('status')=='stopped':raise RuntimeError('target launcher stopped before readiness')
   status_path=runtime/'camera.txt.status.json';status=json.loads(status_path.read_text()) if status_path.exists() else {}
   if status.get('error'):raise RuntimeError(status['error'])
   logs=(runtime/'server.log').read_text(errors='replace') if (runtime/'server.log').exists() else ''
   if not reselected and 'resources loaded' in logs and 'All startup states acknowledged' in logs:
    # Drop the observer's lobby-director selection without overriding game camera.
    control=runtime/'camera.txt';temporary=control.with_suffix('.tmp')
    temporary.write_text(f'{int(time.time()*1000)%0xffffffff} 0 0 0 0 0 0 1 65 1.7777777777777777 .05 500 0\nZ:\\tmp\\unused-reference-readiness.bmp\n')
    temporary.replace(control);reselected=True
   counts=(status.get('camera_calls',0),status.get('present_calls',0))
   if 'resources loaded' in logs and 'All startup states acknowledged' in logs and min(counts)>20:
    if last and all(x>y for x,y in zip(counts,last)):
     since=since or time.monotonic()
     if time.monotonic()-since>2:
      (out/'readiness.json').write_text(json.dumps(status,indent=2)+'\n');break
    else:since=None
   last=counts;time.sleep(.5)
  else:raise TimeoutError('target scene readiness failed')
  poses=json.loads((root/(name+'-poses.json')).read_text())
  capture.capture(runtime/'camera.txt',[dict(poses[0],name='readiness-warmup',settle_ms=5000)],out/'warmup',timeout=60)
  capture.capture(runtime/'camera.txt',poses,out/'poses',timeout=60)
 finally:
  (out/'stopped.json').write_text(run('stop').stdout)
  for file in ['camera.txt.status.json','camera.txt.shader-overrides.json','state.json','server.log','client.log']:
   if(runtime/file).exists():shutil.copy2(runtime/file,out/file)
  profile=runtime/'wineprefix/drive_c/users/sheep/Documents/Survarium/user.cfg'
  if profile.exists():shutil.copy2(profile,out/'profile-after.cfg')
  after=db.stat();assert(before.st_size,before.st_mtime_ns)==(after.st_size,after.st_mtime_ns),'database changed during read-only capture'
  assert record['preset_sha256']==hashlib.sha256(preset.read_bytes()).hexdigest(),'preset changed during capture'
