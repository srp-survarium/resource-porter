#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 2 ]]; then
  echo "usage: $0 FROZEN_CAMERA_HOOK_DLL PRIVATE_BUILD_DIRECTORY" >&2
  exit 2
fi
: "${CC:=i686-w64-mingw32-gcc}"
source_dir=$(cd -- "$(dirname -- "$0")" && pwd)
frozen=$(realpath -- "$1")
mkdir -p -- "$2"
output=$(realpath -- "$2")
if [[ $(dirname -- "$frozen") == "$output" ]]; then
  echo 'Use a separate build directory; this script never edits the client.' >&2
  exit 2
fi
# LDFLAGS supplies the cross compiler static thread-runtime search path when needed.
# shellcheck disable=SC2086
"$CC" -std=c11 -O2 -Wall -Wextra -Werror -static -shared \
  "$source_dir/collision_observer_proxy.c" ${LDFLAGS:-} -o "$output/camera_hook.dll"
cp -- "$frozen" "$output/camera_hook_render.dll"
sha256sum -- "$frozen" "$output/camera_hook.dll" "$output/camera_hook_render.dll" > "$output/sha256.txt"
printf '%s\n' "Built observer proxy in $output; frozen renderer retained byte-for-byte."
