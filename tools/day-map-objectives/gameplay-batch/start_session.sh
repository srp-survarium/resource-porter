#!/usr/bin/env bash
# start_session.sh MAPKEY EVIDENCE_DIR : launch my two-client session in the background.
set -euo pipefail
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
mkdir -p "$2"
cd /tmp
setsid "$P" /tmp/resource-porter-render-compare/tools/day-map-objectives/run_gameplay_session.py "${PKG:-/tmp/three-day-map-candidate}" "$1" \
  --runtime "${RUNTIME:-/tmp/three-day-map-runtime}" --port "${PORT:-2266}" --clients 2 --ui on > "$2/session.log" 2>&1 < /dev/null &
sleep 3
grep -E '"(status|pid|map_id)"' "${RUNTIME:-/tmp/three-day-map-runtime}/state.json" | head -3
# Match play starts after the 20 s countdown; wait for a checkpoint past 25 s.
timeout 240 bash -c 'until grep -qE "Verified checkpoint from slot 0: [0-9]+ bytes at ([3-9][0-9]{4}|[0-9]{6,}) ms" ${RUNTIME:-/tmp/three-day-map-runtime}/server.log 2>/dev/null; do sleep 2; done'
grep -m1 -E "Verified checkpoint .* at ([3-9][0-9]{4}|[0-9]{6,}) ms" ${RUNTIME:-/tmp/three-day-map-runtime}/server.log
