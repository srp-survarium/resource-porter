#!/usr/bin/env bash
# validate_spawn_single.sh MAPKEY PROJECT_RES INDEX EVIDENCE : one point, one client (spawned as team 0), no input
set -uo pipefail
O=/tmp/day-map-gameplay-20261014; P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
T=/tmp/resource-porter-gameplay-objects/tools/day-map-objectives
D=$O/pkg-next-diag; DB=$D/resources.db; WANT=$(cat $D/resources.db.sha256)
MAP=$1; RES=$2; IDX=$3; EV=$4; mkdir -p "$EV"
[ "$(sha256sum < $DB | cut -c1-64)" = "$WANT" ] || { echo "private DB not at baseline"; exit 2; }
PLAN=$EV/plan
[ -e $PLAN/plan.json ] || $P $T/prepare_spawn_narrowing.py --porter /tmp/claude-1000/-home-sheep-Projects-survarium-skhid-srp/0ff5db32-1871-4bc9-852d-20403b00e4ae/scratchpad/target/release/vostok-port $DB $RES $PLAN --points $IDX --team 0 > /dev/null || exit 2
$P /tmp/resource-porter-render-compare/tools/render-compare/patch_archive_entries.py $DB $PLAN/plan.json apply > $EV/apply.json
cat > $EV/probe.sh <<PROBE
#!/usr/bin/env bash
for round in 1 2; do sleep 10; $P $T/gameplay_ctl.py --runtime $O/runtime-diag --evidence $EV status 1; done
PROBE
chmod +x $EV/probe.sh
PKG=$D RUNTIME=$O/runtime-diag $O/static_session.sh $MAP $EV/session 1 -- $EV/probe.sh > $EV/run.log 2>&1
$P /tmp/resource-porter-render-compare/tools/render-compare/patch_archive_entries.py $DB $PLAN/plan.json revert > $EV/revert.json
[ "$(sha256sum < $DB | cut -c1-64)" = "$WANT" ] && echo "$EV: restored" || { echo "$EV: DATABASE NOT RESTORED"; exit 3; }
