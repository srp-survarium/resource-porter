#!/usr/bin/env bash
# static_session.sh MAPKEY EVIDENCE CLIENTS -- COMMAND... : input-free session on the
# next candidate. Holds the shared runtime lock as the GPU lease, waits for the world,
# runs COMMAND, then stops the session and verifies its processes exited.
set -uo pipefail
MAP=$1; EV=$2; CLIENTS=$3; shift 4
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
PKG=${PKG:-/tmp/three-day-map-candidate-next}
export RUNTIME=${RUNTIME:-/tmp/day-map-gameplay-20261014/runtime-next}
mkdir -p "$EV"
# The lease is fd 9 of this script only: it is released when the script exits
# and is closed for every child (9>&-), so nothing can outlive it holding the lock.
exec 9>>/tmp/three-day-map-runtime/run.lock
until [ "$(ps -eo args | grep -c '[s]urvarium-capture')" = 0 ] && flock -n 9; do echo "waiting for GPU lease"; sleep 60; done
cleanup() {
  pid=$(grep -m1 '"pid"' $RUNTIME/state.json 2>/dev/null | grep -o '[0-9]*')
  if [ -n "$pid" ] && tr '\0' ' ' < /proc/$pid/cmdline 2>/dev/null | grep -q run_gameplay_session.py; then kill -TERM $pid; timeout 90 bash -c "while kill -0 $pid 2>/dev/null; do sleep 1; done"; fi
  cp $RUNTIME/server.log $RUNTIME/state.json "$EV/" 2>/dev/null
  flock -u 9
  ps -eo pid,args | grep -E '[s]urvarium-capture|[r]un_gameplay_session' || echo "all session processes exited"
}
trap cleanup EXIT
cd /tmp
setsid "$P" /tmp/resource-porter-gameplay-objects/tools/day-map-objectives/run_gameplay_session.py "$PKG" "$MAP" \
  --runtime "$RUNTIME" --port 2272 --clients "$CLIENTS" --ui off > "$EV/session.log" 2>&1 < /dev/null 9>&- &
sleep 3
timeout 900 bash -c "until grep -qE 'Verified checkpoint from slot 0: [0-9]+ bytes at ([3-9][0-9]{4}|[0-9]{6,}) ms' $RUNTIME/server.log 2>/dev/null; do sleep 2; done" || { echo "world did not start"; exit 1; }
"$@" 9>&-
