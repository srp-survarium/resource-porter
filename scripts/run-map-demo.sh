#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 || ( $# -eq 2 && "${2:-}" != "--check" ) ]]; then
  echo "usage: $0 PROFILE [--check]" >&2
  echo "profiles: level03-native-0100b level04-011e level04-011e-mono level01-day-020f level01-day-020f-native level02-day-020f level02-day-020f-native level03-day-020f level03-day-020f-native level03-day-020f-minimal level03-day-020f-visuals level04-day-020f level04-day-020f-native level05-day-020f level05-day-020f-native level03-day-023h lobby-playable stk2-a01-full stk2-a02-blockpost-slice stk2-a02-full stk2-a03-full stk2-bar-shooting-gallery-derived stk2-bar-shooting-gallery-full stk2-chaz-mir-part" >&2
  exit 2
fi

profile="$1"
check_only=false
if [[ ${2:-} == "--check" ]]; then
  check_only=true
fi
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "${script_dir}/.." && pwd)"
base_checkout="$(cd -- "${repo_dir}/.." && pwd)"
base_client_dir="${BASE_CLIENT_DIR:-${repo_dir}/.work/client-v0100b-akmn}"
server_profile="${profile}"
match_loadout_profile="configured"

case "${profile}" in
  level01-day-020f-native)
    map_db="${base_checkout}/resource-db-builder/.work/resources-v0100b-level01-native-sharedfix.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-stock-map-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  level02-day-020f-native)
    map_db="${base_checkout}/resource-db-builder/.work/resources-v0100b-level02-native-sharedfix.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-stock-map-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  level04-day-020f-native)
    map_db="${base_checkout}/resource-db-builder/.work/resources-v0100b-level04-native-sharedfix.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-stock-map-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  level05-day-020f-native)
    map_db="${base_checkout}/resource-db-builder/.work/resources-v0100b-level05-native-fixed.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-stock-map-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  level03-day-020f-native)
    map_db="${base_checkout}/resource-db-builder/.work/resources-v0100b-level03-native-sharedfix.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-stock-map-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  stk2-a02-full)
    map_db="${repo_dir}/.work/resources-v0100b-stk2-a02-target-native-v22.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-a02-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  stk2-a03-full)
    map_db="${repo_dir}/.work/resources-v0100b-stk2-a03-target-native-v4.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-a02-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  stk2-a01-full)
    map_db="${repo_dir}/.work/resources-v0100b-stk2-a01-checkpoint-wallfix-exo-tir-showcase-v31.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-a01-showcase-runtime-bin"
    match_loadout_profile="stk2-showcase"
    ;;
  stk2-bar-shooting-gallery-full)
    map_db="${repo_dir}/.work/resources-v0100b-stk2-bar-gallery-full-v6.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-a02-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  stk2-chaz-mir-part)
    map_db="${repo_dir}/.work/resources-v0100b-stk2-chaz-mir-v6.db"
    profile_srp_bin_dir="${repo_dir}/.work/srp-a02-runtime-bin"
    match_loadout_profile="stock-v0100b"
    ;;
  level04-011e | level04-011e-mono | level03-day-020f | \
    level03-day-023h | lobby-playable)
    map_db="${repo_dir}/.work/resources-v0100b-current-maps-v11.db"
    ;;
  level03-day-020f-minimal)
    map_db="${repo_dir}/.work/resources-v0100b-level03-020f-minimal-v12.db"
    ;;
  level03-day-020f-visuals)
    map_db="${repo_dir}/.work/resources-v0100b-level03-020f-structural-v15.db"
    ;;
  level03-native-0100b | \
    level01-day-020f | level02-day-020f | \
    level04-day-020f | level05-day-020f | \
    stk2-a02-blockpost-slice | \
    stk2-bar-shooting-gallery-derived)
    map_db="${repo_dir}/.work/resources-v0100b-full-latest-reviewed-all-maps-v8.db"
    ;;
  retail-level03)
    # Control: untouched retail database and a retail map. If this crashes
    # too, the fault is in the client/Wine stack rather than the conversion.
    map_db="${RETAIL_DB}"
    server_profile="level03-evening-0100b"
    match_loadout_profile="stock-v0100b"
    export SRP_STOCK_PROFILE_ONLY=1
    ;;
  soc-mp-pool-nocollision |   soc-mp-pool-keepobjects)
    map_db="${repo_dir}/.work/soc-release/resources-v0100b-soc-pool-ab.db"
    match_loadout_profile="stock-v0100b"
    export SRP_STOCK_PROFILE_ONLY=1
    ;;
  soc-mp-pool | soc-cordon | soc-darkvalley | \
    soc-rostok | soc-factory | soc-atp | soc-railroad | soc-lost-village | \
    soc-military-1 | soc-military-2 | soc-agroprom | soc-workshop | soc-bath)
    # Shadow of Chernobyl maps, from the release built by build-soc-map.
    # stock-v0100b loadout: this database has no STK2 inventory items, and
    # handing out dictionary IDs it does not contain crashes the lobby.
    map_db="${repo_dir}/.work/soc-release/${SOC_DB:-resources-v0100b-soc-maps-v1.db}"
    match_loadout_profile="stock-v0100b"
    # This database is retail plus converted maps and nothing else, so the
    # lobby must not hand out the imported STK2 dictionary IDs.
    export SRP_STOCK_PROFILE_ONLY=1
    ;;
  fear-office)
    # F.E.A.R. multiplayer maps, from the release built by build-lilith-map.
    # Same stock-loadout constraints as the SoC maps: retail dictionary only.
    map_db="${repo_dir}/.work/fear-release/${FEAR_DB:-resources.db}"
    match_loadout_profile="stock-v0100b"
    export SRP_STOCK_PROFILE_ONLY=1
    ;;
  *)
    echo "unknown map profile: ${profile}" >&2
    exit 2
    ;;
esac
map_db="${MAP_DB:-${map_db}}"
map_db="$(realpath -- "${map_db}")"

for required in \
  "${base_client_dir}/binaries" \
  "${base_client_dir}/resources" \
  "${map_db}"
do
  if [[ ! -e "${required}" ]]; then
    echo "missing map runtime artifact: ${required}" >&2
    exit 1
  fi
done

client_dir="${repo_dir}/.work/client-map-${profile}"
mkdir -p "${client_dir}"
ln -sfn "${base_client_dir}/binaries" "${client_dir}/binaries"
ln -sfn "${base_client_dir}/resources" "${client_dir}/resources"
ln -sfn "${map_db}" "${client_dir}/resources.db"

export CLIENT_DIR="${client_dir}"
export WINE_PREFIX="${repo_dir}/.work/wineprefix-map-demo"
export SRP_REPO_DIR="${SRP_REPO_DIR:-${base_checkout}/srp}"
export SRP_RUNTIME_DIR="${SRP_RUNTIME_DIR:-${repo_dir}/.work/stk2-content-srp-worktree}"
export SRP_BIN_DIR="${SRP_BIN_DIR:-${profile_srp_bin_dir:-${SRP_REPO_DIR}/target/debug}}"
export SRP_MAP_PROFILE="${server_profile}"
export SRP_MATCH_LOADOUT_PROFILE="${SRP_MATCH_LOADOUT_PROFILE:-${match_loadout_profile}}"
export SERVER_LOG_DIR="${repo_dir}/.work/srp-map-${profile}-logs"
export CLIENT_LOG="${repo_dir}/.work/client-map-${profile}.log"

for server in login-server browser-server lobby-server match-server; do
  if [[ ! -x "${SRP_BIN_DIR}/${server}" ]]; then
    echo "missing SRP runtime binary: ${SRP_BIN_DIR}/${server}" >&2
    exit 1
  fi
done

if [[ "${check_only}" == true ]]; then
  echo "profile=${profile}"
  echo "map_db=${map_db}"
  echo "client_dir=${client_dir}"
  echo "srp_bin_dir=${SRP_BIN_DIR}"
  echo "match_loadout_profile=${SRP_MATCH_LOADOUT_PROFILE}"
  exit 0
fi

exec "${script_dir}/run-akmn-demo.sh"
