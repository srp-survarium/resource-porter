#!/usr/bin/env bash
# clean_test.sh MAPKEY EVIDENCE : unmodified candidate; spawns, battery spawns, two suicides/respawns per client.
set -uo pipefail
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
T="$P /tmp/resource-porter-render-compare/tools/day-map-objectives/gameplay_ctl.py --evidence $2"
D=/tmp/claude-1000/-home-sheep-Projects-survarium-skhid-srp/0ff5db32-1871-4bc9-852d-20403b00e4ae/scratchpad/decoder/target/release/decode-gameplay
/tmp/day-map-gameplay-20261014/start_session.sh "$1" "$2" || exit 1
$T roster
for s in 1 2; do $T grab $s spawn-client$s; done
for round in 1 2; do
  for s in 1 2; do $T drive $s "key k 300;wait 300"; done
  sleep 12
  for s in 1 2; do $T grab $s respawn$round-client$s; done
done
sleep 6
$T --decoder $D checkpoints 2 10 > /dev/null
