#!/usr/bin/env bash
# probe_spawns.sh RUNTIME EVIDENCE : roster plus two live-camera samples per client, no input.
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
T=/tmp/resource-porter-gameplay-objects/tools/day-map-objectives/gameplay_ctl.py
$P $T --runtime $1 --evidence $2 roster
for round in 1 2; do sleep 10; for s in 1 2; do $P $T --runtime $1 --evidence $2 status $s; done; done
