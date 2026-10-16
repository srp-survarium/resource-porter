#!/usr/bin/env bash
# Rebuild the 194 fill test package layouts A and B on -next 2c286a7d: fill packs (toxic-anomaly logic,
# level_03 rust fog, native-spot ammo crates, native chargers with their static visuals), the earlier
# footstep bindings, and the restored static collector visuals of 194 and 195.
set -euo pipefail
S=/tmp/claude-1000/-home-sheep-Projects-survarium-skhid-srp/0ff5db32-1871-4bc9-852d-20403b00e4ae/scratchpad
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
W=/mnt/hdd/Projects/day-map-gameplay-20261015; G=$W/porter-fill/ports/gameplay-fill; T=$W/porter-fill/tools/day-map-objectives
B="/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs"; NX=/tmp/three-day-map-candidate-next/resources.db
H=2c286a7d4b89f27d9011c325f0aa61ac5ab273c312648c06fb7ba03a108ef478; F=$W/pkg-fill-test; V=/tmp/resource-porter-render-compare/tools/render-compare
PX=(--prefix projects/v011e_level_04/day_2 --prefix projects/v011e_level_03/day_2)
for m in test/mattest_cube01 props/machines/turbine level/04/moss_alfa flora/bush_s_006; do PX+=(--prefix models/ported/v011e/$m.model); done
grep -q '"status": "\(preparing\|running\)"' /tmp/day-map-fill-runtime/state.json 2>/dev/null && { echo "fill session running"; exit 1; }
cur=$(cat $F/LAYOUT.txt); [ "$cur" = A ] || $F/fill-layout.sh A
rm -f $F/resources.db $F/layouts/*.db $F/layouts/*.sha256
for L in A B; do
  C=$([ $L = A ] && echo collectors-base || echo collectors-train)
  rm -rf $W/build-fill-$L $W/fill-overlay-$L $W/apply-work-$L
  (cd $W/porter-fill && $W/target/release/vostok-port build-survarium-maps-026 --manifest ports/survarium-011e-day-to-026g.json \
    --donor-db "$B/011e/app/resources.db" --server-donor-db "$B/011a/app/resources.db" --target-db $(cat $S/RETAIL_DB) \
    --output $W/build-fill-$L --gameplay-fill $G/chemical-011e-anomaly.json --gameplay-fill $G/chemical-011e-rust-fog.json --gameplay-fill $G/chemical-011e-resupply.json \
    --gameplay-fill $G/chemical-011e-$C.json) | tail -1
  # 195 may change only by its restored collector visuals.
  $W/target/release/vostok-port extract --db $NX --path projects/v011e_level_03/day_2/client_project --output $W/build-fill-$L/195-baseline > /dev/null
  $P $T/check_collector_visuals_only.py $W/build-fill-$L/195-baseline $W/build-fill-$L/overlay/projects/v011e_level_03/day_2/client_project
  $P $T/prepare_overlay_patch.py --porter $W/target/release/vostok-port $W/build-fill-$L $W/fill-overlay-$L $NX --archive-sha256 $H "${PX[@]}" \
    --reason "194 gameplay fill layout $L + footstep bindings" > /dev/null
  $P $T/prepare_overlay_patch.py --porter $W/target/release/vostok-port --check $W/fill-overlay-$L $NX
  $W/target/release/vostok-port apply --base-db $NX --bundle $W/fill-overlay-$L --work-dir $W/apply-work-$L --output-db $F/layouts/$L.db | tail -1
  sha256sum $F/layouts/$L.db > $F/layouts/$L.sha256
  $P $V/verify_material_textures.py --porter $W/target/release/vostok-port $F/layouts/$L.db --output $W/verify-fill/textures-$L.json > /dev/null
  $P $V/verify_shader_variants.py --porter $W/target/release/vostok-port $NX $F/layouts/$L.db --output $W/verify-fill/shaders-$L.json > /dev/null
done
mv $F/layouts/A.db $F/resources.db; echo A > $F/LAYOUT.txt
$P - "$F" <<'PY'
import json, os, sys
t = sys.argv[1]; m = json.load(open(f'{t}/manifest.json'))
m['files']['resources.db'] = {'size': os.path.getsize(f'{t}/resources.db'), 'sha256': open(f'{t}/layouts/A.sha256').read().split()[0]}
json.dump(m, open(f'{t}/manifest.json', 'w'), indent=2)
PY
cd $F && ./play-maps verify --runtime /tmp/day-map-fill-runtime && ./fill-layout.sh B && ./play-maps verify --runtime /tmp/day-map-fill-runtime && ./fill-layout.sh A
cat $F/layouts/*.sha256
for f in $W/verify-fill/*.json; do $P -c "import json,sys; d=json.load(open(sys.argv[1])); print(sys.argv[1].split('/')[-1], {k:(len(v) if isinstance(v,(list,dict)) else v) for k,v in d.items() if k not in ('db','baseline','candidate','shader_catalog_sha256')})" $f; done
echo REBUILD-DONE
