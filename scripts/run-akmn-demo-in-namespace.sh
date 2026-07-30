#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="${repo_dir}/.work"
client_dir="${CLIENT_DIR:-${work_dir}/client-v0100b-akmn}"
srp_repo="${SRP_REPO_DIR:-/home/sheep/Projects/surv/srp}"
srp_bin_dir="${SRP_BIN_DIR:-${work_dir}/srp-0100b-target/debug}"
wine_prefix="${WINE_PREFIX:-${work_dir}/wineprefix}"
server_log_dir="${SERVER_LOG_DIR:-${work_dir}/srp-0100b-logs}"
client_log="${CLIENT_LOG:-${work_dir}/client-v0100b-akmn.log}"
max_resources_size_mb="${MAX_RESOURCES_SIZE_MB:-1536}"

if [[ ! "${max_resources_size_mb}" =~ ^[0-9]+$ ]] \
  || (( max_resources_size_mb < 128 || max_resources_size_mb > 2048 )); then
  echo "MAX_RESOURCES_SIZE_MB must be an integer from 128 through 2048" >&2
  exit 1
fi

for required in \
  "${client_dir}/binaries/win32/survarium.exe" \
  "${client_dir}/resources.db" \
  "${srp_repo}/certs/survarium_login_server.key"
do
  if [[ ! -e "${required}" ]]; then
    echo "missing runtime artifact: ${required}" >&2
    exit 1
  fi
done

if [[ ! -x "${srp_bin_dir}/srp" ]]; then
  for required in \
    "${srp_bin_dir}/login-server" \
    "${srp_bin_dir}/browser-server" \
    "${srp_bin_dir}/lobby-server" \
    "${srp_bin_dir}/match-server"
  do
    if [[ ! -x "${required}" ]]; then
      echo "missing runtime artifact: ${required}" >&2
      exit 1
    fi
  done
fi

# A loopback-only namespace left the Wine client permanently at "Connecting..."
# without reaching SRP. Give it a private non-loopback adapter, matching the
# environment in which the client worked on Windows. Build 802 still reports
# local_ip=unknown, but its Winsock login flow works with this adapter present.
ip link set lo up
ip link add survarium0 type dummy
ip address add 10.100.0.1/24 dev survarium0
ip link set survarium0 up

export RUST_LOG="${RUST_LOG:-info}"
mkdir -p "${server_log_dir}"

server_pids=()
if [[ -x "${srp_bin_dir}/srp" ]]; then
  (
    cd -- "${srp_repo}"
    exec "${srp_bin_dir}/srp"
  ) >"${server_log_dir}/srp-combined.log" 2>&1 &
  server_pids+=("$!")
else
  for server in login-server browser-server lobby-server match-server; do
    (
      cd -- "${srp_repo}"
      exec "${srp_bin_dir}/${server}"
    ) >"${server_log_dir}/${server}.log" 2>&1 &
    server_pids+=("$!")
  done
fi

cleanup() {
  WINEPREFIX="${wine_prefix}" wineserver -k 2>/dev/null || true
  kill -KILL "${server_pids[@]}" 2>/dev/null || true
  wait "${server_pids[@]}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in {1..100}; do
  if ss -ltn | grep -q '127.0.0.1:1234' \
    && ss -ltn | grep -q '127.0.0.1:80'; then
    break
  fi
  sleep 0.1
done

if ! ss -ltn | grep -q '127.0.0.1:1234'; then
  echo "SRP did not open the login port; see ${server_log_dir}" >&2
  exit 1
fi
if ! ss -ltn | grep -q '127.0.0.1:80'; then
  echo "SRP did not open the browser port; see ${server_log_dir}" >&2
  exit 1
fi

# Prove that the browser endpoint is reachable from this exact namespace before
# Wine starts. The response is retained with the other runtime artifacts.
curl --fail --silent --show-error \
  'http://127.0.0.1/hello?unused=1' \
  >"${work_dir}/browser-loopback-probe.txt"

export WINEPREFIX="${wine_prefix}"
export WINEDEBUG="${WINEDEBUG:--all}"
export WINEDLLOVERRIDES="d3d8,d3d9,d3d10core,d3d11,dxgi=n"
export LOCALE_ARCHIVE_2_27="${RUSSIAN_LOCALE_DIR:?Russian locale was not provided by the outer launcher}/lib/locale/locale-archive"
export LANG="ru_RU.UTF-8"
export LC_ALL="ru_RU.UTF-8"

# A profile-specific WINEPREFIX may not exist on its first launch. Initialize
# it inside this network namespace before installing DXVK or editing the
# registry, so its wineserver is also created in the correct namespace.
wineboot -u >/dev/null 2>&1

# DXVK: replace WineD3D with Vulkan-backed D3D. The game is 32-bit, so its
# native DLLs belong in syswow64. The outer launcher resolves this immutable
# Nix store path before entering the network namespace.
dxvk_dir="${DXVK_DIR:?DXVK_DIR was not provided by the outer launcher}"
cp -f --no-preserve=mode \
  "${dxvk_dir}/x32/d3d8.dll" \
  "${dxvk_dir}/x32/d3d9.dll" \
  "${dxvk_dir}/x32/d3d10core.dll" \
  "${dxvk_dir}/x32/d3d11.dll" \
  "${dxvk_dir}/x32/dxgi.dll" \
  "${wine_prefix}/drive_c/windows/syswow64/"

# Keep the Windows locale Russian as well as the host-side process locale.
# The old client decodes localized narrow strings through the active Windows
# ANSI code page, so an en-US prefix turns CP1251 diagnostics into mojibake.
wine reg add 'HKCU\Control Panel\International' /v Locale /d 00000419 /f >/dev/null
wine reg add 'HKCU\Control Panel\International' /v LocaleName /d ru-RU /f >/dev/null
wine reg add 'HKCU\Control Panel\International' /v sLanguage /d RUS /f >/dev/null
wine reg add 'HKCU\Control Panel\Desktop' /v PreferredUILanguages /t REG_MULTI_SZ /d ru-RU /f >/dev/null
wine reg add 'HKLM\System\CurrentControlSet\Control\Nls\CodePage' /v ACP /d 1251 /f >/dev/null
wine reg add 'HKLM\System\CurrentControlSet\Control\Nls\CodePage' /v OEMCP /d 866 /f >/dev/null
wine reg add 'HKLM\System\CurrentControlSet\Control\Nls\Language' /v Default /d 0419 /f >/dev/null

# Scaleform requests Arial by family name. The scratch Wine prefix has no
# Microsoft core fonts, but the runtime includes the metrically compatible
# Liberation Sans family. Register the standard Wine font replacement so GDI's
# FontProvider resolves Arial instead of emitting hundreds of thousands of
# missing-font diagnostics.
wine reg add 'HKCU\Software\Wine\Fonts\Replacements' /v Arial /d 'Liberation Sans' /f >/dev/null

cd -- "${client_dir}/binaries/win32"
wine survarium.exe \
  -client=127.0.0.1:1234 \
  -no_splash_screen \
  "-max_resources_size=${max_resources_size_mb}" \
  -no_warning_on_page_file_size \
  -log_to_stdout \
  >"${client_log}" 2>&1
