#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 [--reuse] [--database PATH --database-sha256 SHA256]" >&2
}

reuse=false
database_override=""
database_sha256_override=""
while (($#)); do
  case "$1" in
    --reuse)
      reuse=true
      ;;
    --database)
      shift
      if (($# == 0)); then
        usage
        exit 2
      fi
      database_override="$1"
      ;;
    --database-sha256)
      shift
      if (($# == 0)); then
        usage
        exit 2
      fi
      database_sha256_override="$1"
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
srp_worktree="${SRP_HIT_DEMO_WORKTREE:-${repo_dir}/.work/stk2-content-srp-worktree}"
base_runtime_dir="${repo_dir}/.work/checkpoints/canonical-stk2-a01-v2-20260730/bin"
runtime_dir="${repo_dir}/.work/srp-a01-hit-demo-runtime-bin"
patch_source_dir="${srp_worktree}/client-patches/hit-report"
patch_build_dir="${patch_source_dir}/build"
database="${repo_dir}/.work/checkpoints/canonical-stk2-a01-v2-masonry-nearfix-v3-20260730/resources.db"
database_sha256="4a51b1a80540e93766851674c9c4e9041960a66ef15f06139a6183f2e0795174"
if [[ -n "${database_override}" || -n "${database_sha256_override}" ]]; then
  if [[ -z "${database_override}" || ! "${database_sha256_override}" =~ ^[0-9a-f]{64}$ ]]; then
    echo "--database and a lowercase 64-digit --database-sha256 are required together" >&2
    exit 2
  fi
  database="$(realpath -- "${database_override}")"
  database_sha256="${database_sha256_override}"
fi

for required in \
  "${database}" \
  "${srp_worktree}/Cargo.toml" \
  "${patch_source_dir}/build-linux.sh" \
  "${base_runtime_dir}/login-server" \
  "${base_runtime_dir}/browser-server" \
  "${base_runtime_dir}/lobby-server"
do
  if [[ ! -e "${required}" ]]; then
    echo "missing hit-demo input: ${required}" >&2
    exit 1
  fi
done

actual_database_sha256="$(sha256sum "${database}" | cut -d' ' -f1)"
if [[ "${actual_database_sha256}" != "${database_sha256}" ]]; then
  echo "masonry v3 checkpoint hash mismatch" >&2
  echo "expected=${database_sha256}" >&2
  echo "actual=${actual_database_sha256}" >&2
  exit 1
fi

if [[ "${reuse}" == false ]]; then
  cargo_bin="${CARGO:-cargo}"
  if ! command -v "${cargo_bin}" >/dev/null 2>&1; then
    echo "missing Cargo; set CARGO to the Rust toolchain's cargo executable" >&2
    exit 1
  fi
  (
    cd -- "${srp_worktree}"
    "${cargo_bin}" build --offline -p match-server
  )

  mingw_cxx="${MINGW_CXX:-}"
  mingw_library_dir="${MINGW_LIBRARY_DIR:-}"
  if [[ -z "${mingw_cxx}" || -z "${mingw_library_dir}" ]]; then
    mapfile -t mingw_paths < <(
      nix build \
        nixpkgs#pkgsCross.mingw32.stdenv.cc \
        nixpkgs#pkgsCross.mingw32.windows.mcfgthreads \
        --no-link \
        --print-out-paths
    )
    for path in "${mingw_paths[@]}"; do
      if [[ -x "${path}/bin/i686-w64-mingw32-g++" ]]; then
        mingw_cxx="${path}/bin/i686-w64-mingw32-g++"
      fi
      if [[ -f "${path}/lib/libmcfgthread.a" ]]; then
        mingw_library_dir="${path}/lib"
      fi
    done
  fi
  if [[ ! -x "${mingw_cxx}" || ! -f "${mingw_library_dir}/libmcfgthread.a" ]]; then
    echo "could not resolve the 32-bit MinGW compiler/runtime" >&2
    exit 1
  fi
  env \
    MINGW_CXX="${mingw_cxx}" \
    MINGW_LIBRARY_DIR="${mingw_library_dir}" \
    "${patch_source_dir}/build-linux.sh"

  mkdir -p "${runtime_dir}"
  cp -f -- "${base_runtime_dir}/login-server" "${runtime_dir}/login-server"
  cp -f -- "${base_runtime_dir}/browser-server" "${runtime_dir}/browser-server"
  cp -f -- "${base_runtime_dir}/lobby-server" "${runtime_dir}/lobby-server"
  cp -f -- "${srp_worktree}/target/debug/match-server" "${runtime_dir}/match-server"
fi

for required in \
  "${runtime_dir}/login-server" \
  "${runtime_dir}/browser-server" \
  "${runtime_dir}/lobby-server" \
  "${runtime_dir}/match-server" \
  "${patch_build_dir}/srp_hit_report.dll" \
  "${patch_build_dir}/srp_hit_report_loader.exe"
do
  if [[ ! -x "${required}" && ! -f "${required}" ]]; then
    echo "missing built hit-demo artifact: ${required}" >&2
    exit 1
  fi
done

echo "hit_demo_db=${database}"
echo "hit_demo_srp=${srp_worktree}"
echo "hit_demo_runtime=${runtime_dir}"
echo "hit_demo_client_patch=${patch_build_dir}"
sha256sum \
  "${database}" \
  "${runtime_dir}/match-server" \
  "${patch_build_dir}/srp_hit_report.dll" \
  "${patch_build_dir}/srp_hit_report_loader.exe"

export SRP_BIN_DIR="${runtime_dir}"
export CLIENT_HIT_REPORT_PATCH_DIR="${patch_build_dir}"
exec env \
  CANONICAL_OUTPUT_DB="${database}" \
  "${script_dir}/run-canonical-stk2-a01-mixed-roster-demo.sh" --reuse
