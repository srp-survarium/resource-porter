#!/usr/bin/env python3
"""Freeze five refined variants around an already verified combined database."""
from pathlib import Path
import hashlib,json,os,shutil,subprocess,sys,tarfile,time

ROOT=Path('/home/sheep/Projects/survarium/resource-porter/.work')
if len(sys.argv)!=3:
    raise SystemExit('usage: build_local.py OUTPUT_PACKAGE VERIFIED_COMBINED_DB')
COMBINED_DB=Path(sys.argv[2]).resolve()
assert COMBINED_DB.is_file()
PACKAGE=Path(sys.argv[1]).resolve()
PACKAGE.mkdir(parents=True,exist_ok=False)
records={}

def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while block:=f.read(8*1024*1024):h.update(block)
    return h.hexdigest()

def copy_file(source,relative):
    source=Path(source).resolve();target=PACKAGE/relative;target.parent.mkdir(parents=True,exist_ok=True)
    before=source.stat();h=hashlib.sha256();written=0;next_notice=1024**3
    with source.open('rb') as src,target.open('xb') as dst:
        while block:=src.read(8*1024*1024):
            dst.write(block);h.update(block);written+=len(block)
            if written>=next_notice:
                print(f'{relative}: {written/1024**3:.1f} GiB copied',flush=True);next_notice+=1024**3
    after=source.stat()
    assert (before.st_size,before.st_mtime_ns)==(after.st_size,after.st_mtime_ns),f'source changed: {source}'
    target.chmod(before.st_mode & 0o777)
    records[str(relative)]={'size':written,'sha256':h.hexdigest(),'source':str(source)}
    return str(relative)

def copy_tree(source,relative):
    source=Path(source).resolve()
    for child in sorted(source.rglob('*')):
        if child.is_file():copy_file(child,Path(relative)/child.relative_to(source))
    return str(relative)

def preset(name,environment,hook,config):
    folder=Path('presets')/name;assets={};settings={}
    for key,value in environment.items():
        if value.startswith('Z:\\'):
            source=Path(value[2:].replace('\\','/'))
            relative=folder/'assets'/key
            assets[key]=copy_tree(source,relative) if source.is_dir() else copy_file(source,relative)
        else:settings[key]=value
    hook_path=copy_file(hook,folder/'camera_hook.dll')
    config_path=copy_file(config,folder/'user.cfg')
    data={'environment':settings,'assets':assets,'hook':hook_path,'config':config_path}
    (PACKAGE/folder/'preset.json').write_text(json.dumps(data,indent=2)+'\n')
    return str(folder/'preset.json')

def cfg(prefix):return Path(prefix)/'drive_c/users/sheep/Documents/Survarium/user.cfg'

factory=ROOT/'render-compare/reproduction/factory-live-rain-clouds-20261013'
weather=ROOT/'render-compare/day-evening/reproduction/final-weather-20261013'
chemical=ROOT/'chemical-plant-011a/comparison/checkpoint/probe-last-mip-20261013'
school=Path('/tmp/resource-porter-school-rain-021/.work/school-rain-021')
recipe=json.loads((factory/'recipe.json').read_text());environment=dict(recipe['environment'])
for key,relative in recipe['assets'].items():environment[key]='Z:'+str(factory/relative).replace('/','\\')
profiles={}
profiles['factory-010-rain']=preset('factory-010-rain',environment,factory/'camera_hook.dll',cfg(ROOT/'021-to-026/wineprefix'))
for variant in ('day','evening'):
    name='factory-010-'+variant
    profiles[name]=preset(name,json.loads((weather/(variant+'-recipe.json')).read_text()),weather/'camera_hook.dll',cfg(ROOT/'render-compare/day-evening/026g/wineprefix'))
profiles['chemical-011-day']=preset('chemical-011-day',json.loads((chemical/'recipe-last.json').read_text()),chemical/'observer/camera_hook.dll',cfg(ROOT/'chemical-plant-011a/comparison/026g/wineprefix'))
school_launch=json.loads((school/'comparison/026g/school-probe-rain-52/launch-state.json').read_text())
profiles['school-021-rain']=preset('school-021-rain',school_launch['overrides'],school/'comparison/026g/client/binaries/x86/camera_hook.dll',cfg(school/'comparison/026g/wineprefix'))
assert records['presets/school-021-rain/camera_hook.dll']['sha256']==school_launch['hook_sha256']

copy_tree(ROOT/'021-to-026/client/resources','client/resources')
copy_file(factory/'survarium-capture-v2.exe','client/binaries/x86/survarium-capture.exe')
for name in ('d3d11.dll','dxgi.dll'):copy_file(ROOT/'021-to-026/client/binaries/x86'/name,Path('client/binaries/x86')/name)
copy_file(school/'comparison/026g/client/binaries/x86/bugtrap.dll','client/binaries/x86/bugtrap.dll')
copy_file('/home/sheep/Projects/survarium/srp/target/debug/match-server-026','server/match-server-026')
copy_file(Path(__file__).with_name('launch.py'),'launch.py')
copy_file(__file__,'build_local.py')
copy_file(Path(__file__).with_name('combine.rs'),'combine.rs')
copy_file(Path(__file__).resolve().parents[2]/'docs/playable-map-pack.md','README.md')

prefix=ROOT/'021-to-026/wineprefix'
template=PACKAGE/'wineprefix.tar.gz'
def prefix_filter(info):
    if '/Documents/Survarium/' in info.name and not info.name.endswith('/user.cfg'):return None
    if any(part in ('Temp','temp','Crashes') for part in Path(info.name).parts):return None
    return info
with tarfile.open(template,'w:gz',compresslevel=1) as archive:archive.add(prefix,arcname='wineprefix',filter=prefix_filter)
records['wineprefix.tar.gz']={'size':template.stat().st_size,'sha256':digest(template),'source':str(prefix)}
print('Preset assets, client and Wine template saved',flush=True)

copy_file(COMBINED_DB,'resources.db')
maps=[]
def entry(key,title,map_id,profile,validation):
    maps.append(dict(key=key,title=title,map_id=map_id,database='resources.db',preset=profiles[profile],validation=validation))
for index,variant in enumerate(('day','evening','rain')):
    entry('factory-010-'+variant,'Factory 0.10b / '+variant,180+index,'factory-010-'+variant,
          'Matched-camera rendering checked; movement automation remains unverified.' if variant=='rain' else 'Frozen 32-view recipe; remaining sky/material differences documented.')
entry('chemical-011-day','Chemical Plant 0.11a / day',193,'chemical-011-day','Latest 52-view last-mip recipe; some shadow differences and localized collision issues remain.')
entry('school-021-rain','School 0.21d / rain',165,'school-021-rain','Original rain and latest validated probe correction; final broad run was stopped at user request.')
manifest={'format':1,'created':time.strftime('%Y-%m-%dT%H:%M:%S%z'),'maps':maps,
 'wine':'/nix/store/kd22mbkmwsnvfyn1w14r6xcwpw9fnwzv-wine-wow64-staging-11.8/bin/wine',
 'python':'/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3',
 'default_runtime':'/home/sheep/Projects/survarium/resource-porter/.work/playable-map-pack-runtime',
 'default_port':2260,'default_ui':'on','camera':'actual character; no director request file is created',
 'notes':['Fidelity investigations and all workers are paused.','This is a local Linux/Wine package; its Wine and Python runtimes remain installed in /nix/store.','Custom silent BugTrap from the School client is included.']}
(PACKAGE/'play-maps').write_text('#!/bin/sh\nset -eu\nexec '+manifest['python']+' \"$(dirname -- \"$0\")/launch.py\" \"$@\"\n')
(PACKAGE/'play-maps').chmod(0o755)
(PACKAGE/'maps.json').write_text(json.dumps(manifest,indent=2)+'\n')
(PACKAGE/'README.txt').write_text('Survarium 0.26 playable map pack\n\nFive map variants across three locations. Normal character controller; UI on by default.\n\n'+
 '\n'.join(f"{m['key']:24} {m['title']} (map {m['map_id']})\n  {m['validation']}" for m in maps)+
 '\n\nUse the Python interpreter in maps.json:\n  python launch.py list\n  python launch.py start factory-010-rain\n  python launch.py status\n  python launch.py stop\n  python launch.py verify\n\nOnly the package-owned port/prefix/client is stopped. Other game sessions are untouched.\nShaders and databases are copied into this package; no worker /tmp paths are needed to launch.\nDo not treat stopped comparison runs as completed validation.\n')
for path in sorted(PACKAGE.rglob('*')):
    if path.is_file() and str(path.relative_to(PACKAGE)) not in records:
        records[str(path.relative_to(PACKAGE))]={'size':path.stat().st_size,'sha256':digest(path)}
(PACKAGE/'manifest.json').write_text(json.dumps({'files':records},indent=2)+'\n')
print('Package copied; verifying every stored byte',flush=True)
for relative,record in records.items():
    target=PACKAGE/relative
    assert target.stat().st_size==record['size'] and digest(target)==record['sha256'],relative
    if relative=='resources.db':print('Verified '+relative,flush=True)
(PACKAGE/'verification.json').write_text(json.dumps({'verified_files':len(records),'verified_bytes':sum(r['size'] for r in records.values()),'maps':len(maps),'success':True},indent=2)+'\n')
print(json.dumps({'package':str(PACKAGE),'maps':len(maps),'files':len(records)}),flush=True)
