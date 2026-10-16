#!/usr/bin/env bash
# Rebuild the 194 fill test package on -next 4923aaa9 (which already has the static collector visuals):
# fill packs (toxic-anomaly logic, level_03 rust fog, the growing track anomaly, battery containers and
# ammo crates at the player-chosen base spots), plus the earlier footstep bindings. One layout; no chargers.
set -euo pipefail
S=/tmp/claude-1000/-home-sheep-Projects-survarium-skhid-srp/0ff5db32-1871-4bc9-852d-20403b00e4ae/scratchpad
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
W=/mnt/hdd/Projects/day-map-gameplay-20261015; G=$W/porter-fill/ports/gameplay-fill; T=$W/porter-fill/tools/day-map-objectives
B="/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs"; NX=/tmp/three-day-map-candidate-next/resources.db
H=4923aaa9fb2ffe69cae65abd5dc662d2fff1770116ced529026c5c9c837a6dcd; F=$W/pkg-fill-test; V=/tmp/resource-porter-render-compare/tools/render-compare
PX=(--prefix projects/v011e_level_04/day_2)
for m in test/mattest_cube01 props/machines/turbine level/04/moss_alfa flora/bush_s_006; do PX+=(--prefix models/ported/v011e/$m.model); done
grep -q '"status": "\(preparing\|running\)"' /tmp/day-map-fill-runtime/state.json 2>/dev/null && { echo "fill session running"; exit 1; }
rm -rf $W/build-fill $W/fill-overlay $W/apply-work
(cd $W/porter-fill && $W/target/release/vostok-port build-survarium-maps-026 --manifest ports/survarium-011e-day-to-026g.json \
  --donor-db "$B/011e/app/resources.db" --server-donor-db "$B/011a/app/resources.db" --target-db $(cat $S/RETAIL_DB) \
  --output $W/build-fill --gameplay-fill $G/chemical-011e-anomaly.json --gameplay-fill $G/chemical-011e-rust-fog.json \
  --gameplay-fill $G/chemical-011e-track-anomaly.json --gameplay-fill $G/chemical-011e-resupply.json \
  --gameplay-fill $G/chemical-011e-containers.json) | tail -1
$P $T/prepare_overlay_patch.py --porter $W/target/release/vostok-port $W/build-fill $W/fill-overlay $NX --archive-sha256 $H "${PX[@]}" \
  --reason "194 gameplay fill + footstep bindings" > /dev/null
$P $T/prepare_overlay_patch.py --porter $W/target/release/vostok-port --check $W/fill-overlay $NX
rm -f $F/resources.db $F/resources.db.sha256
$W/target/release/vostok-port apply --base-db $NX --bundle $W/fill-overlay --work-dir $W/apply-work --output-db $F/resources.db | tail -1
sha256sum $F/resources.db | cut -d' ' -f1 > $F/resources.db.sha256
$P $V/verify_material_textures.py --porter $W/target/release/vostok-port $F/resources.db --output $W/verify-fill/textures.json > /dev/null
$P $V/verify_shader_variants.py --porter $W/target/release/vostok-port $NX $F/resources.db --output $W/verify-fill/shaders.json > /dev/null
$P - "$F" <<'PY'
import json, os, sys
t = sys.argv[1]; m = json.load(open(f'{t}/manifest.json'))
m['files']['resources.db'] = {'size': os.path.getsize(f'{t}/resources.db'), 'sha256': open(f'{t}/resources.db.sha256').read().split()[0]}
json.dump(m, open(f'{t}/manifest.json', 'w'), indent=2)
PY
cd $F && ./play-maps verify --runtime /tmp/day-map-fill-runtime
cat $F/resources.db.sha256
for f in $W/verify-fill/textures.json $W/verify-fill/shaders.json; do $P -c "import json,sys; d=json.load(open(sys.argv[1])); print(sys.argv[1].split('/')[-1], {k:(len(v) if isinstance(v,(list,dict)) else v) for k,v in d.items() if k not in ('db','baseline','candidate','shader_catalog_sha256')})" $f; done
echo REBUILD-DONE
