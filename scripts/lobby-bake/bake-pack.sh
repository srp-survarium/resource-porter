#!/usr/bin/env bash
# bake-pack.sh <built pack dir> <0.26g resources.db> <scene spec> <work dir> [bake.py options...]
#
# Path-trace a lobby pack another builder made (the 0.46c2/0.69d lobby ports,
# a pseudo-pack of the classic lobby) as its scene spec (scenes/<lobby>.json)
# says, CPU only (Blender Cycles):
#   <work>/scene.json   prep.py's
#   <work>/pages, baked, baked.json, renders/traced-*   bake.py's
#   <work>/renders/baked-*   show.py's: the views as the game would draw the bake
# The pack drawn from its bake: vostok-port draw-lobby-bake --pack <pack>
# --bake-dir <work> (as build-stk2-lobby-026 --bake-dir is for the STK2
# lobbies, bake.sh).
# --gpu as bake.sh's.
# Needs vostok-port on PATH (or VOSTOK_PORT), Python 3, Blender.
set -euo pipefail
pack=$1 db=$2 spec=$3 work=$4
shift 4
here=$(cd "$(dirname "$0")" && pwd)
. "$here/blender.sh"
mkdir -p "$work"
VOSTOK_PORT=${VOSTOK_PORT:-vostok-port} python3 "$here/prep.py" "$pack" "$db" "$work" "$spec"
blender -b --factory-startup --python "$here/bake.py" -- "$work" "$@"
blender -b --factory-startup --python "$here/show.py" -- "$work" "${show[@]}"
