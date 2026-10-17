#!/usr/bin/env bash
# Apply the reviewed spawn fixes to a built map pack, in place, then check every spawn point of the pack's
# variants (spawn_check.py audit: inside collision, ground under it, its zones) and fail loudly if any point
# fails or a team has fewer than 10 passing points. Edits only respawn points and their zones.
#
#   apply-spawns.sh 021d|v011e PACK_DIR [--plan PLAN.json] [--check-only] [--auto-fix]
#
# --plan       the plan to apply (default: plans/rev20-<kind>.json next to this script);
# --check-only apply nothing, only check;
# --auto-fix   when the check fails after the plan (new collision since the plan was made, e.g. new crates),
#              propose moves for the failing points (nearest passing spot, its zone along), apply them and
#              check again; the proposal is kept in the report directory. Without it a failure only writes
#              the proposal there and stops.
# Report: PACK_DIR.spawn-check/ (audit.csv, summary.md, points.md, <label>.json, applied.log, renders/).
# Environment: SPAWN_BASE (default /mnt/hdd/Projects/release-20/base-patches: resources.db.patch_19, 17, 14,
# 12, newest first, then SPAWN_STOCK_DB), SPAWN_STOCK_DB (default the 0.26g stock resources.db),
# SPAWN_CACHE (default /mnt/hdd/Projects/release-20/spawns/cache), CARGO_TARGET_DIR (default
# /mnt/hdd/Projects/release-20/spawns/target).
set -euo pipefail
die() { echo "SPAWN FIXES FAILED: $*" >&2; exit 1; }
[ $# -ge 2 ] || die "usage: $0 021d|v011e PACK_DIR [--plan PLAN.json] [--check-only] [--auto-fix]"
kind=$1; pack=$(realpath "$2"); shift 2
here=$(cd "$(dirname "$0")" && pwd); porter=$(cd "$here/../.." && pwd)
plan="$here/plans/rev20-$kind.json"; check_only=0; auto=0
while [ $# -gt 0 ]; do case $1 in
  --plan) plan=$(realpath "$2"); shift 2;; --check-only) check_only=1; shift;; --auto-fix) auto=1; shift;;
  *) die "unknown option $1";; esac; done
case $kind in 021d|v011e) ;; *) die "kind is 021d or v011e, not $kind";; esac
[ -f "$pack/pack.json" ] && [ -d "$pack/files" ] || die "$pack is not a built pack (pack.json, files/)"
base=${SPAWN_BASE:-/mnt/hdd/Projects/release-20/base-patches}
stock=${SPAWN_STOCK_DB:-/home/sheep/Projects/survarium/srp/0.26g/game/resources.db}
cache=${SPAWN_CACHE:-/mnt/hdd/Projects/release-20/spawns/cache}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/mnt/hdd/Projects/release-20/spawns/target}
for n in 19 17 14 12; do [ -f "$base/resources.db.patch_$n" ] || die "no $base/resources.db.patch_$n"; done
[ -f "$stock" ] || die "no stock database $stock"
out="$pack.spawn-check"; mkdir -p "$out"

echo "== tools (resource-porter $(git -C "$porter" rev-parse --short HEAD))"
(cd "$porter" && nix develop --command cargo build --release --quiet --bin vostok-port --example edit-built-pack) || die "cannot build the porter tools"
py=$(nix build --no-link --print-out-paths --impure --expr \
  'let p = import (builtins.getFlake "nixpkgs") {}; in p.python3.withPackages (ps: [ps.numpy ps.pillow])')/bin/python3
[ -x "$py" ] || die "no python with numpy and pillow"
run() { (cd "$porter" && nix develop --command "$@"); }
vp="$CARGO_TARGET_DIR/release/vostok-port"; edit="$CARGO_TARGET_DIR/release/examples/edit-built-pack"

cfg="$out/config.json"
"$py" -I - "$here/variants-$kind.json" "$cfg" "$vp" "$cache" "$pack" "$base" "$stock" <<'PY'
import json, sys
variants, cfg, vp, cache, pack, base, stock = sys.argv[1:]
json.dump(dict(porter=vp, cache=cache, pack=pack, variants=json.load(open(variants)),
               base=[f'{base}/resources.db.patch_{n}' for n in (19, 17, 14, 12)] + [stock]), open(cfg, 'w'), indent=1)
PY
for v in $("$py" -I -c 'import json,sys; print(" ".join(v["project"] for v in json.load(open(sys.argv[1]))))' "$here/variants-$kind.json"); do
  [ -f "$pack/files/$v" ] || die "$pack has no $v"
done

if [ $check_only = 0 ]; then
  echo "== apply $plan"
  [ -f "$plan" ] || die "no plan $plan"
  run "$edit" "$pack" "$plan" "$stock" | tee "$out/applied.log" || die "the plan does not apply to $pack (see above): it was made for another state of the pack; re-run with --check-only, then propose a new plan"
fi

audit() { run "$py" -I "$here/spawn_check.py" audit "$cfg" "$out" --fail | tee "$out/audit.log"; }
echo "== check"
if ! audit; then
  run "$py" -I "$here/spawn_check.py" propose "$cfg" "$out" "$out/proposal.json" > "$out/proposal.log" || true
  if [ $auto = 1 ]; then
    echo "== auto-fix: $out/proposal.json"
    run "$edit" "$pack" "$out/proposal.json" "$stock" | tee -a "$out/applied.log" || die "the proposal does not apply"
    audit || die "points still fail after the auto-fix: $out/summary.md"
  else
    die "failing points or teams under 10 after the plan: $out/summary.md; a proposal for them is in $out/proposal.json (apply with --plan, or re-run with --auto-fix)"
  fi
fi
planarg=(); [ $check_only = 0 ] && planarg=(--plan "$plan")
run "$py" -I "$here/spawn_check.py" render "$cfg" "$out" "$out/renders" "${planarg[@]}" > "$out/render.log" || die "renders failed: $out/render.log"
echo "== spawn check passed: every point of the $kind variants passes; report $out/summary.md, renders $out/renders/"
