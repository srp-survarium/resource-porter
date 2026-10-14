#!/usr/bin/env bash
# delivery_test.sh MAPKEY SPEC_JSON ZONES_JSON EVIDENCE
# Diagnostic layout must already be applied. Per team: pick up the battery
# in front of the spawn, walk to the team's collector and use it.
set -uo pipefail
MAP=$1; SPEC=$2; ZONES=$3; EV=$4
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
T="$P /tmp/resource-porter-render-compare/tools/day-map-objectives/gameplay_ctl.py --evidence $EV"
D=/tmp/claude-1000/-home-sheep-Projects-survarium-skhid-srp/0ff5db32-1871-4bc9-852d-20403b00e4ae/scratchpad/decoder/target/release/decode-gameplay
/tmp/day-map-gameplay-20261014/start_session.sh "$MAP" "$EV" || exit 1
$T roster
roster=$(tail -1 "$EV/actions.jsonl")
for team in 0 1; do
  client=$($P -c "import json,sys; r=json.loads(sys.argv[1])['clients']; print(next(k for k,v in r.items() if v['team']==$team))" "$roster")
  bat=$($P -c "import json,sys; s=json.load(open(sys.argv[1])); b=s['batteries'][$team]['position']; print(b[0], b[1]+0.12, b[2])" "$SPEC")
  zone=$($P -c "import json,sys; print(*json.load(open(sys.argv[1]))['$team'])" "$ZONES")
  read zx zy zz <<< "$zone"
  $T grab $client team$team-spawn
  $T aim $client $bat
  $T grab $client team$team-aim-battery
  $T drive $client "key e 300;wait 800"
  $T grab $client team$team-after-pickup
  $T approach $client $zx $zy $zz 1.4
  $T aim $client $zx $zy $zz
  $T grab $client team$team-aim-collector
  $T drive $client "key e 300;wait 1500"
  $T grab $client team$team-after-use
  $T drive $client "key e 3000;wait 1500"
  $T grab $client team$team-after-hold
done
sleep 8
$T --decoder $D checkpoints 2 2 > /dev/null
