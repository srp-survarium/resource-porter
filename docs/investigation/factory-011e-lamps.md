# Factory 0.11e lamp and beam comparison

Factory 0.11e contains revised general lighting, but no beam placement,
beam material, or cone pixel-shader fix was found relative to 0.10b/0.11a.
Do not infer beam parity in 0.26 merely from changing the donor release.

The donor is build 884 at
`/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011e/app/resources.db`.
The checked Factory project is `projects/level_03/client_project`.
The verified report and extracted inputs are in
`/tmp/resource-porter-factory-day-lights/.work/factory-011e-lamps-verified`
(6.4 MiB; no client launch or GPU capture).

| Source contract | 0.10b / 0.11a → 0.11e |
|---|---|
| 12 cone placements | Exact same model, position, rotation and scale, including multiplicity |
| 121 lamp placements | Exact same model, position, rotation and scale, including multiplicity |
| Cone mesh and model settings | Byte-identical |
| Wired lamp mesh and model settings | Byte-identical |
| Cone volume material | Byte-identical |
| Spot and sphere falloff DDS | Byte-identical |
| Cone pixel shaders | Same 30 executable variants, 20 distinct executable chunks; identical multiplicities |
| Lamp bulb material | Only emission tint changes: `[1,1,1,1]` → `[1,0.913355,0.849736,1]` |
| Existing 18 local lights and sun | No authored changes |
| Added local light | Animated point light at `[19.717,1.879,-39.035]` |
| Post-process material | 19 changed fields |

Shader resource keys differ between the releases. The audit extracts and hashes
DXBC `SHDR`/`SHEX` executable chunks rather than interpreting different filenames
as a shader change. Matching executable multisets prove unchanged instructions
across the stored permutations, not unchanged renderer state or constant uploads.
The cone shader family has no newly added executable implementation in 0.11e.

Lighting objects are matched by unique `full_name`, not array index. Forty
identity records differ: the added point light and 39 probe records, including
renames and new `probe_side_width` fields. Twenty-two records have changes beyond
that new field. These are not forty independently moved lights. Examples include
larger or repositioned probes, changed normal clipping, ambient multipliers, and
lower diffuse multipliers on previously intense interior probes.

Post changes include bloom scale 0.1 → 0.25, scattering multiplier 1 → 3,
inscatter distance 200 → 300, altered filmic tone mapping, SSAO saturation 2 → 1.5,
god-ray intensity 1 → 0.2, and lens flare multiplier 2 → 0.7. The full report
preserves all changed fields. The sun object itself is unchanged. Reusing the
0.10b weather shader constants would not establish 0.11e lighting fidelity.

Both bulb emission switches remain true in the original 0.11e material. The
narrow conversion repair in `MaterialCatalog::convert` preserves these switches
for the verified native lamp template; existing scalar/color merging retains
the warmer 0.11e tint. The native 0.26 cone-opacity difference remains a separate
compatibility question. No cone geometry should be moved on this evidence.

Reproduce with `tools/render-compare/compare_factory_lamp_releases.py`:

```sh
python tools/render-compare/compare_factory_lamp_releases.py \
  /path/to/vostok-port \
  /path/to/010b/resources.db \
  /path/to/011a/resources.db \
  /path/to/011e/resources.db \
  /path/to/new-evidence-directory
```

The tool uses the repository's binary-config decoder, extracts only the project,
selected lamp/cone resources and cone pixel shaders, rejects duplicate lighting
identities, and writes hashes and semantic differences. The archived verified run
matches the initial audit; formatting/lint checks pass. Separately, all four
existing material-conversion tests pass against the root branch with both the
water and lamp repairs integrated. No new test cases were added.
