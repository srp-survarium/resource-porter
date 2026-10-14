#!/usr/bin/env python3
"""Validate native batteries against a diagnostic batch and update gallery poses.

Requires initial exact positions and two stable later checkpoints. A failed
validation writes nothing, so candidate points cannot masquerade as live items.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess
import shutil

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument('manifest', type=Path)
ap.add_argument('map')
ap.add_argument('batch', type=int)
ap.add_argument('snapshots', type=Path)
ap.add_argument('--decoder', type=Path, default=Path('/tmp/decode-battery-snapshot-json'))
ap.add_argument('--players', type=int, default=1)
ap.add_argument('--output', type=Path, required=True)
ap.add_argument('--evidence-dir', type=Path, help='Defaults to OUTPUT parent/checkpoints/MAP/batch-NN')
ap.add_argument('--settle-ms', type=int, default=5000)
ap.add_argument('--stability-metres', type=float, default=.03)
a = ap.parse_args()
manifest = json.loads(a.manifest.read_text())
item = next(m for m in manifest['maps'] if m['key'] == a.map)
spawns = [s for s in item['spawns'] if s['diagnostic_batch'] == a.batch]
assert 1 <= len(spawns) <= 10, 'Missing or oversized diagnostic batch'
files = sorted(a.snapshots.glob('match026g_*_battery.bin'), key=lambda p: int(p.name.split('_')[2]))
assert files, 'No complete snapshots'
peers = {p.name.split('_')[1] for p in files}
assert len(peers) == 1, 'Use checkpoints from exactly one client/run'
records = []
for file in files:
    time = int(file.name.split('_')[2])
    record = json.loads(subprocess.check_output([str(a.decoder), str(file), str(time), str(a.players), str(len(spawns))], text=True))
    record['file'] = str(file.resolve())
    record['sha256'] = hashlib.sha256(file.read_bytes()).hexdigest()
    assert all(b['state'] == 'free' for b in record['batteries']), 'A gallery item is carried or stored'
    records.append(record)
initial = records[0]
matched = {}
for battery in initial['batteries']:
    candidates = [s for s in spawns if math.dist(s['position'], battery['position']) < .0001]
    assert len(candidates) == 1, f"Battery {battery['id']} did not start at one authored batch candidate"
    spawn = candidates[0]
    assert spawn['index'] not in matched, 'Two batteries mapped to one candidate'
    matched[spawn['index']] = battery['id']
assert len(matched) == len(spawns), 'Not every candidate has a live battery'
late = [r for r in records if r['time_ms'] >= initial['time_ms'] + a.settle_ms]
assert len(late) >= 2, 'Need two later checkpoints to establish settled positions'
previous, settled = late[-2:]
assert settled['time_ms'] - previous['time_ms'] >= 1000, 'Checkpoints too close to establish stability'
max_motion = max(math.dist(p['position'], q['position']) for p, q in zip(previous['batteries'], settled['batteries']))
assert max_motion <= a.stability_metres, f'Batteries still moving: maximum change {max_motion:.3f}m'
rows = []
for spawn in spawns:
    battery_id = matched[spawn['index']]
    position = settled['batteries'][battery_id]['position']
    displacement = math.dist(spawn['position'], position)
    spawn.update(active_runtime_battery_id=battery_id, runtime_position=position,
                 settled_position=position, runtime_validated=True,
                 runtime_initial_time_ms=initial['time_ms'], runtime_settled_time_ms=settled['time_ms'],
                 displacement_from_authored_metres=displacement)
    rows.append({'index': spawn['index'], 'battery_id': battery_id, 'authored_position': spawn['position'],
                 'runtime_position': position, 'displacement_metres': displacement})
evidence = a.evidence_dir or a.output.parent / 'checkpoints' / a.map / f'batch-{a.batch:02d}'
evidence.mkdir(parents=True, exist_ok=True)
for record in records:
    source_file = Path(record['file'])
    destination = evidence / source_file.name
    if source_file.resolve() != destination.resolve():
        shutil.copy2(source_file, destination)
    assert hashlib.sha256(destination.read_bytes()).hexdigest() == record['sha256']
    record['file'] = str(destination.resolve())
(evidence / 'decoded-checkpoints.json').write_text(json.dumps(records, indent=2)+'\n')
report = {'map': a.map, 'batch': a.batch, 'battery_count': len(spawns), 'initial_time_ms': initial['time_ms'],
          'settled_time_ms': settled['time_ms'], 'max_motion_between_late_checkpoints_metres': max_motion,
          'initial_checkpoint': {'file': initial['file'], 'sha256': initial['sha256']},
          'settled_checkpoint': {'file': settled['file'], 'sha256': settled['sha256']}, 'batteries': rows}
for batch in item.get('batches', []):
    if batch['index'] == a.batch:
        batch['runtime_validation'] = report
        break
else:
    item.setdefault('runtime_validation', {})[str(a.batch)] = report
# Atomic replacement protects the browseable manifest from partial writes.
a.output.parent.mkdir(parents=True, exist_ok=True)
temporary = a.output.with_suffix(a.output.suffix + '.tmp')
temporary.write_text(json.dumps(manifest, indent=2)+'\n')
temporary.replace(a.output)
print(json.dumps(report, indent=2))
