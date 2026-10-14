#!/usr/bin/env bash
# boiler_walk.sh EVIDENCE : team-0 character walks from the diagnostic spawn
# along the user's reported heading into the boiler, then around it.
set -uo pipefail
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
T="$P /tmp/resource-porter-render-compare/tools/day-map-objectives/gameplay_ctl.py --runtime ${RUNTIME:-/tmp/three-day-map-runtime} --evidence $1"
D=/tmp/claude-1000/-home-sheep-Projects-survarium-skhid-srp/0ff5db32-1871-4bc9-852d-20403b00e4ae/scratchpad/decoder/target/release/decode-gameplay
/tmp/day-map-gameplay-20261014/start_session.sh chemical-011e-day "$1" || exit 1
$T roster
c=$($P -c "import json,sys; r=json.loads(sys.argv[1])['clients']; print(next(k for k,v in r.items() if v['team']==0))" "$(tail -1 $1/actions.jsonl)")
$T grab $c boiler-start
# Level the view along the reported heading (point 6 m ahead at eye height).
$T aim $c -61.538 7.06 28.688
$T grab $c boiler-heading
for i in 1 2 3 4 5 6; do $T drive $c "key w 500;wait 300"; $T status $c; done
$T grab $c boiler-after-forward
# Around the boiler's east side, 2.1-2.3 m from the tank centres (X -66.77).
for wp in "-64.6 7.06 27.9" "-64.6 7.06 31.9" "-64.6 7.06 36.2" "-64.6 7.06 40.3"; do
  $T approach $c $wp 0.6
  $T grab $c "boiler-wp-${wp// /_}"
done
