#!/usr/bin/env bash
# Revision 20: replace the growing track fog of Chemical Plant (v0.11), maps 206/207, in a built
# maps-v011e pack, in place. Only the client objects under logic/fill/track_anomaly/ of
# projects/v011e_level_04/{day_2,day_4}/client_project change: the old set (maps-v011e-4's, or the
# first or second revision 20 fog; checked by count and hash) goes,
# ports/gameplay-fill/chemical-011e-track-anomaly.json comes in (checked by hash before and after).
# The plan's add_files (the fog's particle, derived from native rust_smoke) are written under files/
# and get an add_file operation in pack.json (kept in path order). Anything else changing fails the
# run; nothing is written unless every check passes. It refuses to run twice on the same pack.
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
fill_sha256=71085a68c1de080d2a5a908abf9c6e8e11061efc8b1b586ea43b813edb1b6c6f
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
added=$(jq -r '.add_files[]?.path' "$plan")
newpack=$(mktemp); trap 'rm -f "$newpack"' EXIT
cp "$pack/pack.json" "$newpack"
for p in $added; do
  n=$(jq --arg p "$p" '[.operations[] | select(.path == $p)] | length' "$newpack")
  if [ -e "$pack/files/$p" ] || [ "$n" != 0 ]; then
    # Already there from an earlier run of this fog: the same file (checked by the editor) and one add_file.
    [ -e "$pack/files/$p" ] && [ "$n" = 1 ] && jq -e --arg p "$p" '[.operations[] | select(.path == $p and .op == "add_file")] | length == 1' "$newpack" >/dev/null \
      || die "$p: the file and pack.json's add_file must both be there or both be absent"
    continue
  fi
  jq --arg p "$p" '.operations |= ([(map(select(.op == "add_file")) + [{op: "add_file", path: $p}] | sort_by(.path))[], (map(select(.op != "add_file")))[]])' "$newpack" > "$newpack.next"
  mv "$newpack.next" "$newpack"
done
cd "$repo"
build=(cargo build --quiet --release --locked --example edit-built-pack)
if command -v cc >/dev/null; then "${build[@]}"; else nix develop "$repo" --command "${build[@]}"; fi || die "cannot build edit-built-pack"
exe="${CARGO_TARGET_DIR:-$repo/target}/release/examples/edit-built-pack"
echo "pack: $pack (version $(jq -r .version "$pack/pack.json"))"
for d in day_2 day_4; do sha256sum "$pack/files/projects/v011e_level_04/$d/client_project"; done | sed 's/^/before: /'
"$exe" "$pack" "$plan" "$stock" || die "edit failed; the pack is unchanged"
cmp -s "$newpack" "$pack/pack.json" || { cp "$newpack" "$pack/pack.json"; echo "pack.json: add_file for $added"; }
for d in day_2 day_4; do sha256sum "$pack/files/projects/v011e_level_04/$d/client_project"; done | sed 's/^/after:  /'
for p in $added; do sha256sum "$pack/files/$p" | sed 's/^/added:  /'; done
jq -r '
  [.clones[] | select(.full_name | test("/stage_[0-9]+/controller$"))] as $s
  | [.clones[] | select(.full_name | test("/stage_[0-9]+/fog_zone$")) | .set.position[2]] as $z
  | [.clones[] | select(.full_name == "logic/fill/track_anomaly/damage/controller")][0].set.states as $d
  | "track fog now: \($s | length) stages, z \($z | min) .. \($z | max), from \($s[0].set.states[0].active_time_sec) s every \($s[1].set.states[0].active_time_sec - $s[0].set.states[0].active_time_sec) s (full at \($s[-1].set.states[0].active_time_sec) s); damage steps at "
    + ([foreach $d[] as $st (0; . + $st.active_time_sec; .)] | .[:-1] | map(tostring + " s") | join(", "))
    + "; \([.clones[] | select(.full_name | test("/damage/segment_"))] | length) capped segments"' "$fill"
