#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

usage() {
  echo "usage: $0 [--build-only] [--reuse]" >&2
}

build_only=false
reuse=false
while (($#)); do
  case "$1" in
    --build-only)
      build_only=true
      ;;
    --reuse)
      reuse=true
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

base_db="${repo_dir}/.work/resources-v0100b-canonical-stk2-a01-weapons-v1.db"
armor_manifest="${repo_dir}/ports/dolg-exoskeleton-stk2-to-0100b.json"
donor_model="${repo_dir}/.work/stk2-character-source/resources/exported/models/character/human/dolg/dolg_exo.skinned_model"
donor_textures="${repo_dir}/.work/stk2-character-source/resources/sources/textures/character/human/dolg"
armor_bundle="${repo_dir}/.work/canonical-dolg-exoskeleton-after-a01-weapons-v1"
output_db="${CANONICAL_OUTPUT_DB:-${repo_dir}/.work/resources-v0100b-canonical-stk2-a01-weapons-exo-v2.db}"
srp_bin_dir="${SRP_BIN_DIR:-${repo_dir}/.work/srp-a01-showcase-runtime-bin}"

if [[ "${reuse}" == false ]]; then
  "${script_dir}/run-canonical-stk2-a01-weapons-demo.sh" --build-only

  rm -rf -- "${armor_bundle}"
  "${porter_bin}" build-armor \
    --manifest "${armor_manifest}" \
    --donor-model-archive "${donor_model}" \
    --donor-texture-dir "${donor_textures}" \
    --target-db "${base_db}" \
    --output "${armor_bundle}"

  "${porter_bin}" validate-overlay \
    --base-db "${base_db}" \
    --overlay "${armor_bundle}"
  "${porter_bin}" apply \
    --base-db "${base_db}" \
    --bundle "${armor_bundle}" \
    --output-db "${output_db}" \
    --force
  "${porter_bin}" verify --db "${output_db}" --bundle "${armor_bundle}"
  "${porter_bin}" validate-models --db "${output_db}"
fi

for required in \
  "${output_db}" \
  "${srp_bin_dir}/login-server" \
  "${srp_bin_dir}/browser-server" \
  "${srp_bin_dir}/lobby-server" \
  "${srp_bin_dir}/match-server"
do
  if [[ ! -e "${required}" ]]; then
    echo "missing mixed-roster demo input: ${required}" >&2
    exit 1
  fi
done

echo "canonical_extended_db=${output_db}"
sha256sum "${output_db}"

if [[ "${build_only}" == true ]]; then
  exit 0
fi

exec env \
  MAP_DB="${output_db}" \
  SRP_BIN_DIR="${srp_bin_dir}" \
  SRP_MATCH_LOADOUT_PROFILE=stk2-mixed-roster \
  "${script_dir}/run-map-demo.sh" stk2-a01-full
