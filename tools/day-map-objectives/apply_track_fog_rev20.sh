#!/usr/bin/env bash
# Revision 20: replace the growing track fog of Chemical Plant (v0.11), maps 206/207, in a built
# maps-v011e pack, in place. Only the client objects under logic/fill/track_anomaly/ of
# projects/v011e_level_04/{day_2,day_4}/client_project change: the old set (maps-v011e-4's, checked
# by count and hash) goes, ports/gameplay-fill/chemical-011e-track-anomaly.json comes in (checked by
# hash before and after). Anything else outside those objects changing fails the run; nothing is
# written unless every check passes. It refuses to run twice on the same pack.
#
# Usage: tools/day-map-objectives/apply_track_fog_rev20.sh PACK_DIR [STOCK_RESOURCES_DB]
#   PACK_DIR: a built pack directory (pack.json, files/), e.g. maps-v011e-5 before its spawn edits.
#   STOCK_RESOURCES_DB: the stock 0.26g resources.db (the fill's native level_02 template is read from it).
# Builds the editor with cargo (through `nix develop` when no C compiler is on PATH); CARGO_TARGET_DIR is honoured.
set -euo pipefail
die() { echo "apply_track_fog_rev20: $*" >&2; exit 1; }
[ $# -ge 1 ] && [ $# -le 2 ] || die "usage: $0 PACK_DIR [STOCK_RESOURCES_DB]"
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
pack=$(realpath -e "$1") || die "no pack directory $1"
stock=$(realpath -e "${2:-/home/sheep/Projects/survarium/srp/0.26g/game/resources.db}") || die "no stock resources.db"
plan="$repo/tools/day-map-objectives/track-fog-rev20.plan.json"
fill="$repo/ports/gameplay-fill/chemical-011e-track-anomaly.json"
fill_sha256=b3cf4638f4b66fafca29795db2e1ca11d9ef1890358edc74030cfa56e5c13119
[ -f "$pack/pack.json" ] && [ -d "$pack/files" ] || die "$pack is not a built pack (pack.json, files/)"
id=$(jq -r .id "$pack/pack.json") || die "unreadable $pack/pack.json"
[ "$id" = maps-v011e ] || die "$pack is pack '$id', not maps-v011e"
for d in day_2 day_4; do
  f="$pack/files/projects/v011e_level_04/$d/client_project"
  [ -f "$f" ] || die "missing $f"
  jq -e --arg p "projects/v011e_level_04/$d/client_project" '[.operations[] | select(.path == $p)] | length == 1' "$pack/pack.json" >/dev/null \
    || die "pack.json does not carry projects/v011e_level_04/$d/client_project exactly once"
done
echo "$fill_sha256  $fill" | sha256sum -c --quiet - || die "$fill is not the reviewed track pack"
cd "$repo"
build=(cargo build --quiet --release --locked --example edit-built-pack)
if command -v cc >/dev/null; then "${build[@]}"; else nix develop "$repo" --command "${build[@]}"; fi || die "cannot build edit-built-pack"
exe="${CARGO_TARGET_DIR:-$repo/target}/release/examples/edit-built-pack"
echo "pack: $pack (version $(jq -r .version "$pack/pack.json"))"
for d in day_2 day_4; do sha256sum "$pack/files/projects/v011e_level_04/$d/client_project"; done | sed 's/^/before: /'
"$exe" "$pack" "$plan" "$stock" || die "edit failed; the pack is unchanged"
for d in day_2 day_4; do sha256sum "$pack/files/projects/v011e_level_04/$d/client_project"; done | sed 's/^/after:  /'
jq -r '
  [.clones[] | select(.full_name | test("/stage_[0-9]+/controller$"))] as $s
  | [.clones[] | select(.full_name | test("/stage_[0-9]+/fog_zone$")) | .set.position[2]] as $z
  | [.clones[] | select(.full_name == "logic/fill/track_anomaly/damage/controller")][0].set.states as $d
  | "track fog now: \($s | length) stages, z \($z | min) .. \($z | max), from \($s[0].set.states[0].active_time_sec) s every \($s[1].set.states[0].active_time_sec - $s[0].set.states[0].active_time_sec) s (full at \($s[-1].set.states[0].active_time_sec) s); damage steps at "
    + ([foreach $d[] as $st (0; . + $st.active_time_sec; .)] | .[:-1] | map(tostring + " s") | join(", "))
    + "; \([.clones[] | select(.full_name | test("/damage/segment_"))] | length) capped segments"' "$fill"
