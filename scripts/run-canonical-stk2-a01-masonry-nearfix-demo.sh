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

checkpoint_dir="${repo_dir}/.work/checkpoints/canonical-stk2-a01-v2-20260730"
base_db="${checkpoint_dir}/resources.db"
base_sha256="5cfecbd2f850ddefc2fe1f01ba42ecf63c512fb9f562e25b2a6a9a7b40384e3f"
retail_db="${repo_dir}/.work/resources-v0100b-original.db"
retail_sha256="160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409"
bundle="${repo_dir}/.work/canonical-stk2-a01-v2-masonry-nearfix-v3"
output_db="${repo_dir}/.work/resources-v0100b-canonical-stk2-a01-v2-masonry-nearfix-v3.db"

brick_material="material_instances/ported/stk2/stk2_a01_full/source_materials/architecture/mine/brick_red.material"
white_brick_material="material_instances/ported/stk2/stk2_a01_full/tiled/bricks/brick_bl.material"
concrete_material="material_instances/ported/stk2/stk2_a01_full/source_materials/architecture/mine/concrete.material"
secondary_concrete_material="material_instances/ported/stk2/stk2_a01_full/tiled/concrete/concrete_tile_b.material"
concrete_floor_material="material_instances/ported/stk2/stk2_a01_full/source_materials/architecture/mine/crete_floor.material"
metal_tiles_material="material_instances/ported/stk2/stk2_a01_full/architecture/mine/metal_tiles.material"
ruberoid_material="material_instances/ported/stk2/stk2_a01_full/source_materials/tiled/roof/ruberoid_dark_grey.material"
wood_roof_material="material_instances/ported/stk2/stk2_a01_full/tiled/wood/wood_tile_a.material"
vertex_color_nonblended_template="material_instances/tiled/bricks/brick_02.material"

for required in "${porter_bin}" "${base_db}" "${retail_db}"; do
  if [[ ! -e "${required}" ]]; then
    echo "missing masonry-fix input: ${required}" >&2
    exit 1
  fi
done

actual_base_sha256="$(sha256sum "${base_db}" | cut -d' ' -f1)"
if [[ "${actual_base_sha256}" != "${base_sha256}" ]]; then
  echo "canonical v2 checkpoint hash mismatch" >&2
  echo "expected=${base_sha256}" >&2
  echo "actual=${actual_base_sha256}" >&2
  exit 1
fi
actual_retail_sha256="$(sha256sum "${retail_db}" | cut -d' ' -f1)"
if [[ "${actual_retail_sha256}" != "${retail_sha256}" ]]; then
  echo "retail resources.db hash mismatch" >&2
  echo "expected=${retail_sha256}" >&2
  echo "actual=${actual_retail_sha256}" >&2
  exit 1
fi

if [[ "${reuse}" == false ]]; then
  rm -rf -- "${bundle}"
  "${porter_bin}" build-material-patch \
    --source-db "${base_db}" \
    --target-db "${retail_db}" \
    --material "${brick_material}=${vertex_color_nonblended_template}" \
    --material "${white_brick_material}=${vertex_color_nonblended_template}" \
    --material "${concrete_material}=${vertex_color_nonblended_template}" \
    --material "${secondary_concrete_material}=${vertex_color_nonblended_template}" \
    --material "${concrete_floor_material}=${vertex_color_nonblended_template}" \
    --material "${metal_tiles_material}=${vertex_color_nonblended_template}" \
    --material "${ruberoid_material}=${vertex_color_nonblended_template}" \
    --material "${wood_roof_material}=${vertex_color_nonblended_template}" \
    --output "${bundle}"

  "${porter_bin}" validate-overlay \
    --base-db "${base_db}" \
    --overlay "${bundle}/overlay"
  "${porter_bin}" apply \
    --base-db "${base_db}" \
    --bundle "${bundle}" \
    --output-db "${output_db}" \
    --force
  "${porter_bin}" verify --db "${output_db}" --bundle "${bundle}"
  "${porter_bin}" validate-models --db "${output_db}"
fi

if [[ ! -f "${output_db}" ]]; then
  echo "missing masonry-fix database: ${output_db}" >&2
  exit 1
fi

echo "canonical_base_db=${base_db}"
echo "masonry_patch_bundle=${bundle}"
echo "masonry_output_db=${output_db}"
sha256sum "${output_db}"

if [[ "${build_only}" == true ]]; then
  exit 0
fi

exec env \
  CANONICAL_OUTPUT_DB="${output_db}" \
  "${script_dir}/run-canonical-stk2-a01-mixed-roster-demo.sh" --reuse
