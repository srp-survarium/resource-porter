#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
: "${CC:=i686-w64-mingw32-gcc}"
# Supply LDFLAGS when the compiler's static thread runtime needs a library path.
# shellcheck disable=SC2086
"$CC" -std=c11 -O2 -Wall -Wextra -Werror -static -shared camera_hook.c \
  -o camera_hook.dll ${LDFLAGS:-} -ld3d11 -ldxgi -ldxguid -lm
# shellcheck disable=SC2086
"$CC" -std=c11 -O2 -Wall -Wextra -Werror -static viewport.c \
  -o viewport.exe ${LDFLAGS:-}
