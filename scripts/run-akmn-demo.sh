#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "${script_dir}/.." && pwd)"
wine_prefix="${repo_dir}/.work/wineprefix"

# A wineserver owns the network namespace of the process that created it.
# Stop any persistent instance before unshare, otherwise Wine can reconnect to
# a host-namespace server and its 127.0.0.1 will not be SRP's 127.0.0.1.
WINEPREFIX="${wine_prefix}" wineserver -k 2>/dev/null || true
WINEPREFIX="${wine_prefix}" wineserver -w 2>/dev/null || true

# A private network namespace lets an ordinary user bind the stock client's
# required browser port 80. Wine and SRP run in the same namespace and talk
# exclusively over its loopback device.
exec unshare --user --map-root-user --net \
  "${script_dir}/run-akmn-demo-in-namespace.sh"
