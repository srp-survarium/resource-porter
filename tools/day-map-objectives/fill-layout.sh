#!/usr/bin/env bash
# Usage: fill-layout.sh A|B|status
# Switches this package's Chemical 194 gameplay-fill layout while its session is stopped:
#   A  native chargers at the two bases (default), resupply crates, toxic-moss anomaly
#   B  native chargers at the central train (contested), resupply crates, toxic-moss anomaly
# Both layouts carry the footstep bindings. The inactive database waits in layouts/<X>.db;
# switching renames files on the same disk and updates manifest.json, so it is instant.
set -eu
T=$(cd -- "$(dirname -- "$0")" && pwd); R=${RUNTIME:-/tmp/day-map-fill-runtime}
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
cur=$(cat "$T/LAYOUT.txt")
case "${1:-status}" in
  status) echo "layout $cur"; exit 0;;
  A|B) ;;
  *) echo "usage: $0 A|B|status"; exit 2;;
esac
if [ -f "$R/state.json" ] && grep -Eq '"status": "(preparing|running)"' "$R/state.json"; then
  echo "stop the session first: $T/play-maps stop --runtime $R"; exit 1
fi
[ "$cur" = "$1" ] && { echo "layout $1 already active"; exit 0; }
mv "$T/resources.db" "$T/layouts/$cur.db"
mv "$T/layouts/$1.db" "$T/resources.db"
echo "$1" > "$T/LAYOUT.txt"
"$P" - "$T" "$1" <<'PY'
import json, os, sys
t, layout = sys.argv[1], sys.argv[2]
m = json.load(open(f'{t}/manifest.json'))
m['files']['resources.db'] = {'size': os.path.getsize(f'{t}/resources.db'), 'sha256': open(f'{t}/layouts/{layout}.sha256').read().split()[0]}
json.dump(m, open(f'{t}/manifest.json', 'w'), indent=2)
PY
echo "layout $1 active"
