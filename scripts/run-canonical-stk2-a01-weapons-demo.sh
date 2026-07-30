#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 [--build-only] [--rebuild-weapons]" >&2
}

build_only=false
rebuild_weapons=false
while (($#)); do
  case "$1" in
    --build-only)
      build_only=true
      ;;
    --rebuild-weapons)
      rebuild_weapons=true
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "${script_dir}/.." && pwd)"
porter_bin="${repo_dir}/target/debug/vostok-port"

retail_db="${RETAIL_DB:-${repo_dir}/.work/resources-v0100b-original.db}"
retail_sha256="160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409"
map_bundle="${STK2_MAP_BUNDLE:-${repo_dir}/.work/stk2-a01-checkpoint-clean-terrain-020f-industrial-ambient-v26}"
weapon_source="${STK2_WEAPON_SOURCE_ROOT:-${repo_dir}/.work/stk2-content-source/resources}"
weapon_manifest="${repo_dir}/ports/stk2-ak74-lobby-to-0100b.json"
weapon_bundle="${repo_dir}/.work/canonical-stk2-weapons-from-retail-v1"
combined_bundle="${repo_dir}/.work/canonical-stk2-a01-weapons-bundle-v1"
apply_dir="${repo_dir}/.work/apply-canonical-stk2-a01-weapons-v1"
output_db="${CANONICAL_OUTPUT_DB:-${repo_dir}/.work/resources-v0100b-canonical-stk2-a01-weapons-v1.db}"
srp_bin_dir="${SRP_BIN_DIR:-${repo_dir}/.work/srp-stk2-a01-weapons-runtime-bin}"

for required in \
  "${porter_bin}" \
  "${retail_db}" \
  "${map_bundle}/port-manifest.json" \
  "${weapon_manifest}" \
  "${weapon_source}" \
  "${srp_bin_dir}/login-server" \
  "${srp_bin_dir}/browser-server" \
  "${srp_bin_dir}/lobby-server" \
  "${srp_bin_dir}/match-server"
do
  if [[ ! -e "${required}" ]]; then
    echo "missing canonical STK2 demo input: ${required}" >&2
    exit 1
  fi
done

actual_retail_sha256="$(sha256sum "${retail_db}" | cut -d' ' -f1)"
if [[ "${actual_retail_sha256}" != "${retail_sha256}" ]]; then
  echo "retail resources.db hash mismatch" >&2
  echo "expected=${retail_sha256}" >&2
  echo "actual=${actual_retail_sha256}" >&2
  exit 1
fi

if [[ "${rebuild_weapons}" == true || ! -f "${weapon_bundle}/port-manifest.json" ]]; then
  rm -rf -- "${weapon_bundle}"
  "${porter_bin}" build-stk2-content \
    --manifest "${weapon_manifest}" \
    --source-root "${weapon_source}" \
    --target-db "${retail_db}" \
    --output "${weapon_bundle}" \
    --preserve-lobby
fi

rm -rf -- "${combined_bundle}" "${apply_dir}"
"${porter_bin}" compose \
  --bundle "${map_bundle}" \
  --bundle "${weapon_bundle}" \
  --output "${combined_bundle}"

"${porter_bin}" validate-overlay \
  --base-db "${retail_db}" \
  --overlay "${combined_bundle}/overlay"

"${porter_bin}" apply \
  --base-db "${retail_db}" \
  --bundle "${combined_bundle}" \
  --work-dir "${apply_dir}" \
  --output-db "${output_db}" \
  --force

"${porter_bin}" verify \
  --db "${output_db}" \
  --bundle "${combined_bundle}"
"${porter_bin}" validate-models --db "${output_db}"

echo "canonical_retail_db=${retail_db}"
echo "canonical_map_bundle=${map_bundle}"
echo "canonical_weapon_bundle=${weapon_bundle}"
echo "canonical_output_db=${output_db}"
sha256sum "${output_db}"

if [[ "${build_only}" == true ]]; then
  exit 0
fi

exec env \
  MAP_DB="${output_db}" \
  SRP_BIN_DIR="${srp_bin_dir}" \
  SRP_MATCH_LOADOUT_PROFILE=stk2-weapons \
  "${script_dir}/run-map-demo.sh" stk2-a01-full
