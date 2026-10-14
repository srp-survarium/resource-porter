#!/usr/bin/env bash
# stop_session.sh EVIDENCE_DIR : stop my session, keep logs, verify exit.
set -uo pipefail
R=${RUNTIME:-/tmp/three-day-map-runtime}
cp $R/server.log $R/state.json "$1/" 2>/dev/null; cp $R/client.log "$1/client1.log"; cp $R/peer2/client.log "$1/client2.log"
pid=$(grep -m1 '"pid"' $R/state.json | grep -o '[0-9]*')
if tr '\0' ' ' < /proc/$pid/cmdline 2>/dev/null | grep -q run_gameplay_session.py; then kill -TERM $pid; fi
timeout 90 bash -c "while kill -0 $pid 2>/dev/null; do sleep 1; done"
grep '"status"' $R/state.json
ps -eo pid,args | grep -E 'survarium-capture|run_gameplay_session|three-day-map-runtime' | grep -v grep || echo "all session processes exited"
