#!/usr/bin/env bash
# bake.sh <manifest> <stk2 root> <0.26g resources.db> <work dir> [bake.py options...]
#
# Path-trace a baked lobby manifest's slice (Blender Cycles) and build its pack
# drawn from the bake:
#   <work>/lit      the pack with the slice lit (what is baked)
#   <work>/bake     scene.json (prep.py), the pages, baked models and baked.json
#                   (bake.py), renders/ (traced-* path-traced, baked-* from the bake)
#   <work>/pack     the pack drawn from the bake
# With --gpu, Cycles runs on the GPU (BLENDER: a command running a Blender
# built with OptiX or CUDA, as blender.org's), one job at a time: on the
# lock LOBBY_BAKE_GPU_LOCK when set (blender.sh), or the one the BLENDER
# command takes itself.
# Needs vostok-port on PATH (or VOSTOK_PORT), ImageMagick, Python 3, Blender.
set -euo pipefail
manifest=$1 stk2=$2 db=$3 work=$4
shift 4
here=$(cd "$(dirname "$0")" && pwd)
port=${VOSTOK_PORT:-vostok-port}
. "$here/blender.sh"
mkdir -p "$work"
rm -rf "$work/lit" "$work/pack"
"$port" build-stk2-lobby-026 --manifest "$manifest" --stk2-root "$stk2" --target-db "$db" \
    --output "$work/lit" --work "$work/textures" --lit
VOSTOK_PORT=$port python3 "$here/prep.py" "$work/lit" "$db" "$work/bake" "$manifest"
blender -b --factory-startup --python "$here/bake.py" -- "$work/bake" "$@"
blender -b --factory-startup --python "$here/show.py" -- "$work/bake" "${show[@]}"
"$port" build-stk2-lobby-026 --manifest "$manifest" --stk2-root "$stk2" --target-db "$db" \
    --output "$work/pack" --work "$work/textures" --bake-dir "$work/bake"
