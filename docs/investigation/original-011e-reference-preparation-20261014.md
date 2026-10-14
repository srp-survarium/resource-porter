# Original 0.11e reference preparation

Validated in one coordinated original session: twelve Chemical Plant and seventeen
Radar views, followed by the same twenty-nine cameras on the unchanged 0.26
candidate. Both sessions stopped cleanly. Original references are frozen; no
additional original launch is needed for these poses.

## Guarded camera and bootstrap

Original executable:
`/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011e/app/binaries/win32/survarium.exe`.
SHA256 `83d6cd0728e3ab71acfa8507b9458c23373ac25ebace9ca4bf1539e53f9f2396`.

PDB-backed `base_game_scene::apply_camera` is actual VA `0x5d42c0`, RVA
`0x5c42c0`, length 220. Its guarded entry bytes are
`83 ec 40 53 8b 5c 24 48`. At `0x5d42d0` it copies the view from EDX+4;
at `0x5d42dd` it copies the projection from EDX+0x44. This independently confirms
the existing EDX camera observer ABI; no nearby-build address was assumed.

The generic PE import/bootstrap preparer now accepts the pinned 011e source.
Its disposable output SHA256 is
`b0c5c6054e547d9260fdad1d651f8c7e9b2c195a1b9f1fc4981bb167bc16692d`.
Original `.text` bytes remain unchanged. The camera hook patches only the guarded
apply-camera entry at runtime and observes DXGI Present. The 011e initialization
path skips all shader capture/replacement, native lighting hooks and post hooks.

Cross-compiled observer and viewport helper passed `-Wall -Wextra -Werror`.
The viewport helper uses the GUI subsystem to avoid creating a console that
steals game focus. It executes only inside this recipe's dedicated Wine prefix.

## Prepared session

`/tmp/resource-porter-chemical-011/.work/reference-011e/recipe.json` pins the
staged executable/DLL, server, configuration and pose files. A resource database
symlink points directly at the original 011e archive; no data is changed or
copied. Auxiliary resources contain only the existing SSL files and splash image.

- Chemical target 194 corresponds to original `level_04`. The existing spectator
  server profile is named `chemical-011a`; that name selects the project path,
  not an asset donor. All assets come from the linked **011e** database.
- Radar target 195 corresponds to original `level_03`; server profile
  `level03-native-0100b` likewise selects only the original project path.
- Chemical gets twelve and Radar seventeen 1280×720 screenshots: each has eight
  45-degree panorama headings plus four distant views. Radar additionally includes
  both exact remains poses and three lamp poses supplied by the material/lamp
  workers; their source JSON provenance is recorded per pose. The original captured frames and exact camera
  metadata will remain separate from any assembled panoramas.
- Chemical uses east-hall panorama plus west/north/south/central locations;
  Radar uses yard panorama plus hangar/upper/hospital/officer-roof locations.
- The stopped original 011a Wine prefix is reused, avoiding a large prefix copy.
  Its `user.cfg` is saved/restored in `finally`; no other prefix is stopped.
- The old comparison profile is retained as an explicit fixture, with credentials
  stripped and `r_debug_clip_texture_quality off`. Texture quality remains 2.
  This disables the early renderer's VRAM-dependent texture downgrade without
  changing detected VRAM. It is not the later 020/021 pool-offset workaround.
  Actual saved profile is preserved after each map and confirms quality 2 and
  debug clipping disabled. Both builds use gamma 1, lighting/shadow/shading 1;
  source post quality 3 versus target 2 and texture 2 versus target 1 remain
  explicit differences. Numeric quality enums are not assumed equivalent.

The runner refuses an active dedicated prefix or occupied port 2254. It waits for
`join_match` and `sync_response`, then sustained camera/Present advancement.
A separate five-second warmup image precedes final views. All final images were reviewed on contact sheets: geometry is present, UI absent,
and resolution 1280×720. Separate warmup frames remain outside the final set.
Counter advancement alone is not a texture-streaming guarantee.

Run the non-launching validation first:

```sh
python3 tools/chemical-plant/run_reference_011e.py .work/reference-011e/recipe.json
```

After an explicit coordinated GPU handoff, the same command with `--launch`
runs Chemical then Radar sequentially. `--maps chemical` permits the first
bounded compatibility check. Initialization has a 180-second deadline and each
pose has a 30-second deadline; failures preserve logs and stop owned processes.
The original 011e spectator protocol/UI bootstrap worked without memory-floor,
menu no-op, or nearby-version protocol patches.


## Matched candidate replay and frozen outputs

Artifact root: `/tmp/resource-porter-chemical-011/.work/reference-011e`.
`capture-{chemical,radar}/poses` holds originals and `target-{chemical,radar}/poses`
holds current candidate images, each with requested and actual camera metadata.
`comparison/capture-sha256.json` hashes the captured inputs; `metrics.json` lists
per-view metrics. Contact sheets are `comparison/{chemical,radar}-{capture,target}-contact.png`.
Five full-resolution paired lamp/remains PNGs put original on the left.
Each map/engine also has a `panoramas` directory, with individual frames retained.
Panorama stations include nearby walls; the four separate distant poses provide
broader coverage. Assembly does not adjust exposure or blend seams.

`replay_reference_011e.py` uses the managed candidate launcher, verifies database
size/mtime and preset SHA256 remain unchanged, and stops its owned runtime in
`finally`. After map startup acknowledgments it writes one **disabled** camera
request to clear the observer's stale lobby-director selection. This does not
move the character. Sustained camera/Present advancement and a five-second warmup
then precede final capture. The initial run performed this reset manually; the
reusable runner now includes it. It does not run without `--launch`.

```sh
python3 tools/chemical-plant/replay_reference_011e.py .work/reference-011e \
  --package /tmp/three-day-map-candidate --runtime /tmp/three-day-map-runtime \
  --port 2266 --launch
python3 tools/chemical-plant/compare_reference_011e.py .work/reference-011e
```

Both commands refuse existing output directories. Preserve existing references
and use a fresh artifact directory for any later target variant.

The unchanged candidate is substantially darker: Chemical mean RGB MAE 0.1260
across twelve views; Radar 0.1578 across seventeen. Chemical east-hall source
mean luminance is 0.177–0.209 versus target 0.054–0.096. This includes walls,
floors and metal, so it is not solely a pipe-specular problem. Original distant
vegetation already shows strong grain/colored edge noise. Single-frame errors
also include unsynchronized grain, wind and simulation; they are not temporal
shimmer measurements. No brightness correction was applied during this audit.
