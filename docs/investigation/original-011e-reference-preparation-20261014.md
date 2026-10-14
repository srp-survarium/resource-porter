# Original 0.11e reference preparation

Prepared, not yet runtime validated. No game or Wine process was started during
this work. The first coordinated session will capture Chemical Plant and Radar
from the original build 884 database, then reuse those frozen images.

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
  Actual saved profile is preserved after each map; source render defaults and
  accepted quality commands still need checking on the first run.

The runner refuses an active dedicated prefix or occupied port 2254. It waits for
`join_match` and `sync_response`, then sustained camera/Present advancement.
A separate five-second warmup image precedes final views. Contact-sheet review
must still reject incomplete geometry, incorrect UI, wrong resolution or startup
frames: counter advancement alone cannot prove that every texture has settled.

Run the non-launching validation first:

```sh
python3 tools/chemical-plant/run_reference_011e.py .work/reference-011e/recipe.json
```

After an explicit coordinated GPU handoff, the same command with `--launch`
runs Chemical then Radar sequentially. `--maps chemical` permits the first
bounded compatibility check. Initialization has a 180-second deadline and each
pose has a 30-second deadline; failures preserve logs and stop owned processes.
The original 011e spectator protocol/UI bootstrap remains unproven. No speculative
memory-floor, menu no-op, or nearby-version protocol patch is included.
