#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "${script_dir}/.." && pwd)"
outer_work_dir="$(cd -- "${repo_dir}/.." && pwd)"

export CLIENT_DIR="${repo_dir}/.work/client-v0100b-stk2-weapons-stock-lobby"
export WINE_PREFIX="${repo_dir}/.work/wineprefix-stk2-content"
export SRP_REPO_DIR="${outer_work_dir}/stk2-content-srp-worktree"
export SRP_BIN_DIR="${SRP_REPO_DIR}/target/debug"
export SERVER_LOG_DIR="${repo_dir}/.work/srp-stk2-weapons-stock-lobby-logs"
export CLIENT_LOG="${repo_dir}/.work/client-v0100b-stk2-weapons-stock-lobby.log"

exec "${script_dir}/run-akmn-demo.sh"
