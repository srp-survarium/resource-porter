#!/usr/bin/env bash
# run_batch.sh [STEP...] : the approved runtime batch. Each step: apply diagnostic
# (if any) -> two-client run -> stop -> revert -> verify database hash.
set -uo pipefail
O=/tmp/day-map-gameplay-20261014
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
PATCH="$P /tmp/resource-porter-render-compare/tools/render-compare/patch_archive_entries.py"
CAND=/tmp/three-day-map-candidate/resources.db
CAND_SHA=8e72816cfc341178e4b32ebad4d3138b07b137af9be46b94e89b15252d545b16
BEFORE_SHA=bc411413367c5dd9d0792a4f4db27b27cb18b33b67759bfef1ddde470f685196
applied=""; applied_db=""; lockpid=""
cleanup() {
  RUNTIME=${RUNTIME:-/tmp/three-day-map-runtime} $O/stop_session.sh "${E:-$O/runs/aborted}" >/dev/null 2>&1
  if [ -n "$applied" ]; then $PATCH "$applied_db" "$applied" revert >/dev/null && echo "reverted $applied"; applied=""; fi
  if [ -n "$lockpid" ]; then kill $lockpid 2>/dev/null; lockpid=""; fi
}
trap 'cleanup; exit 1' INT TERM ERR
busy() { $P -c "
import fcntl,sys
f=open('/tmp/three-day-map-runtime/run.lock','a')
try: fcntl.flock(f, fcntl.LOCK_EX|fcntl.LOCK_NB)
except BlockingIOError: sys.exit(0)
sys.exit(1)"; }
step() {  # step NAME MAPKEY PLAN(or -) SCRIPT ARGS...
  local name=$1 map=$2 plan=$3; shift 3
  E=$O/runs/$name; mkdir -p "$E"
  until { [ -n "${SKIP_LOCKCHECK:-}" ] || ! busy; } && [ "$(ps -eo args | grep -c '[s]urvarium-capture')" = 0 ]; do echo "waiting for GPU lock"; sleep 60; done
  if [ "$plan" != - ]; then applied_db=${DB:-$CAND}; $PATCH "$applied_db" "$plan" apply > "$E/diag-apply.json"; applied=$plan; fi
  echo "== $name ($(date +%T))"
  "$@" "$E" > "$E/run.log" 2>&1 || echo "step $name script exit $?"
  RUNTIME=${RUNTIME:-/tmp/three-day-map-runtime} $O/stop_session.sh "$E" | tail -2
  if [ -n "$applied" ]; then $PATCH "$applied_db" "$applied" revert > "$E/diag-revert.json"; applied=""; fi
  local want=$CAND_SHA; [ "${DB:-$CAND}" != "$CAND" ] && want=$BEFORE_SHA
  [ "$(sha256sum < ${DB:-$CAND} | cut -c1-64)" = "$want" ] && echo "database restored ($want)" || { echo "DATABASE HASH MISMATCH"; exit 2; }
}
want_step() { [ $# -eq 0 ] || [[ " $STEPS " == *" $1 "* ]]; }
STEPS="$*"
want_step boiler-after && step 194-boiler-after chemical-011e-day $O/diag/194-boiler/plan.json $O/boiler_walk.sh
if want_step boiler-before; then
  # Same walk on the pre-fix database in a private copy; hold the shared lock meanwhile.
  until ! busy && [ "$(ps -eo args | grep -c '[s]urvarium-capture')" = 0 ]; do echo "waiting for GPU lock"; sleep 60; done
  flock -n /tmp/three-day-map-runtime/run.lock sleep 1200 & lockpid=$!
  sleep 1
  SKIP_LOCKCHECK=1 PKG=$O/pkg-before RUNTIME=$O/runtime-before PORT=2268 DB=$O/pkg-before/resources.db \
    step 194-boiler-before chemical-011e-day $O/diag/194-boiler-before/plan.json env PKG=$O/pkg-before RUNTIME=$O/runtime-before PORT=2268 $O/boiler_walk.sh
  kill $lockpid 2>/dev/null; lockpid=""
fi
for m in "194 chemical-011e-day" "195 radar-011e-day" "196 radar-020e-day"; do
  set -- $m
  want_step clean-$1 && step $1-clean $2 - $O/clean_test.sh $2
  want_step delivery-$1 && step $1-delivery $2 $O/diag/$1-delivery/plan.json $O/delivery_test.sh $2 $O/diag/$1-delivery.json $O/diag/$1-zones.json
done
trap - INT TERM ERR
echo "batch complete $(date +%T)"
