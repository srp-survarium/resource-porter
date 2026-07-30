#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "${script_dir}/.." && pwd)"
wine_prefix="${WINE_PREFIX:-${repo_dir}/.work/wineprefix}"
dxvk_dir="${DXVK_DIR:-}"
russian_locale_dir="${RUSSIAN_LOCALE_DIR:-}"

if [[ -z "${dxvk_dir}" ]]; then
  dxvk_dir="$(nix build nixpkgs#dxvk --no-link --print-out-paths)"
fi
for dxvk_dll in d3d8 d3d9 d3d10core d3d11 dxgi; do
  if [[ ! -f "${dxvk_dir}/x32/${dxvk_dll}.dll" ]]; then
    echo "missing 32-bit DXVK DLL: ${dxvk_dir}/x32/${dxvk_dll}.dll" >&2
    exit 1
  fi
done
export DXVK_DIR="${dxvk_dir}"

if [[ -z "${russian_locale_dir}" ]]; then
  russian_locale_dir="$(nix build nixpkgs#glibcLocales --no-link --print-out-paths)"
fi
russian_locale_archive="${russian_locale_dir}/lib/locale/locale-archive"
if [[ ! -f "${russian_locale_archive}" ]]; then
  echo "missing Russian locale archive: ${russian_locale_archive}" >&2
  exit 1
fi
if [[ "$(
  env -u LC_ALL -u LC_CTYPE \
    LOCALE_ARCHIVE_2_27="${russian_locale_archive}" \
    LANG=ru_RU.UTF-8 \
    locale charmap
)" != "UTF-8" ]]; then
  echo "Nix locale archive does not provide ru_RU.UTF-8" >&2
  exit 1
fi
export RUSSIAN_LOCALE_DIR="${russian_locale_dir}"
export LOCALE_ARCHIVE_2_27="${russian_locale_archive}"
export LANG="ru_RU.UTF-8"
export LC_ALL="ru_RU.UTF-8"

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
