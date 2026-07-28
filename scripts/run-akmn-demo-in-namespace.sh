#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="${repo_dir}/.work"
client_dir="${work_dir}/client-v0100b-akmn"
srp_repo="${SRP_REPO_DIR:-/home/sheep/Projects/surv/srp}"
srp_bin_dir="${work_dir}/srp-0100b-target/debug"
wine_prefix="${work_dir}/wineprefix"
server_log_dir="${work_dir}/srp-0100b-logs"
client_log="${work_dir}/client-v0100b-akmn.log"

for required in \
  "${client_dir}/binaries/win32/survarium.exe" \
  "${client_dir}/resources.db" \
  "${srp_bin_dir}/login-server" \
  "${srp_bin_dir}/browser-server" \
  "${srp_bin_dir}/lobby-server" \
  "${srp_bin_dir}/match-server" \
  "${srp_repo}/certs/survarium_login_server.key"
do
  if [[ ! -e "${required}" ]]; then
    echo "missing runtime artifact: ${required}" >&2
    exit 1
  fi
done

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
for server in login-server browser-server lobby-server match-server; do
  (
    cd -- "${srp_repo}"
    exec "${srp_bin_dir}/${server}"
  ) >"${server_log_dir}/${server}.log" 2>&1 &
  server_pids+=("$!")
done

cleanup() {
  WINEPREFIX="${wine_prefix}" wineserver -k 2>/dev/null || true
  kill -KILL "${server_pids[@]}" 2>/dev/null || true
  wait "${server_pids[@]}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in {1..50}; do
  if ss -ltn | grep -q '127.0.0.1:1234'; then
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
cd -- "${client_dir}/binaries/win32"
wine survarium.exe \
  -client=127.0.0.1:1234 \
  -no_splash_screen \
  -max_resources_size=1024 \
  -no_warning_on_page_file_size \
  -log_to_stdout \
  >"${client_log}" 2>&1
