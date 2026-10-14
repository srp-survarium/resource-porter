# Factory day water, 2026-10-14

Scope: Factory 0.10b day, packaged as `ported/v010w`. The user screenshot
`~/screenshots/2026-10-13-20-39-12.png` shows dense white streaks across water
when looking down. This investigation does not change atmosphere or native maps.

## Cause and correction

The source `material_instances/water/water_green.material` uses
`forward_default_simpe_water` and has no `foam_intensity` parameter. The original
`forward_simple_water.ps` has no foam texture or foam blend. The 0.26 template
adds `foam_intensity = 1`; the converter copies source parameters into that
template but previously retained this newly introduced layer at full strength.

The saved native shader disassembly at
`.work/early-to-026/shaders/026-water.asm` binds `t_foam` to t5 and
`foam_intensity` to cb0[23].x (byte 368). Near the final surface composition,
its foam weight is multiplied by this scalar and 30, then used to blend the
existing reflected/refracted water toward foam texture times diffuse lighting.
Zeroing the scalar removes exactly that extra layer. Wave normals, reflection,
refraction, fog, geometry, and soft-intersection parameters remain unchanged.

`MaterialCatalog::convert` now sets target foam to zero only when both effects
are simple water and the donor has no foam parameter. Authored foam in newer
sources remains intact. One focused regression test covers this distinction,
unrelated effects, and older templates without foam. The four material
conversion tests pass using the edited module against cached dependencies.

## Reproducing the archive experiment

`tools/render-compare/prepare_factory_water.py` prepares exact baseline/fixed
bytes plus hashes and offsets from the supplied archive. It requires the
original donor's foam absence and the current port's intensity of one. It
changes one byte in the Factory day water material. The existing
`patch_archive_entries.py` validates bytes and CRC before applying/reverting.
Stop clients using the archive before mutation. No database copy is needed.

```sh
python3 tools/render-compare/prepare_factory_water.py \
  --exe /path/to/vostok-port --archive /path/to/resources.db \
  --source-db /path/to/010b/resources.db --output /path/to/water-evidence/patch
python3 tools/render-compare/patch_archive_entries.py \
  /path/to/resources.db /path/to/water-evidence/patch/patch.json apply
# Same command with revert restores the exact saved bytes and CRC.
```

Water is surface `sdsaaa777sg1` of `terrain_p_08`, bounds
`[10.74,0.427,-53.05]..[40.6,0.496,-25]`. The packed surface binds precisely this
water material. Runtime evidence is recorded separately below; offline shader
analysis alone does not prove that the user-visible defect is resolved.

The shader's angle/depth dependence also explains why this is strongest in
some downward views. Writing `d` for the difference of sampled scene depth and
water's view-space depth, its foam calculation is:

```
u = abs(view_direction.y)^0.4 * saturate(2 * abs(d))
weight = 30 * foam_intensity * saturate(u - u*u)
       * foam.r * saturate(foam.r - u * (2 - soft_intersection_factor))
```

Both the viewing direction and projected shallow-water depth affect the mask;
it is not a constant white overlay. Removing inherited foam leaves these
native water reflection/refraction calculations intact rather than flattening
wave normals or reducing scene brightness.

## Runtime validation

The packaged Factory day hook and shader preset were kept unchanged for both
runs, with `ui off` and screenshot-only mode. No original client was relaunched.
The near-downward view at `[26.39,4.5,-38.382]`, forward `[0,-0.995,-0.1]`,
reproduced the user's dense white pattern. Disabling only foam removes that
pattern and reveals the submerged terrain. The first oblique control was
partially blocked by a tree; an additional clear oblique view and a view within
0.6 degrees of perpendicular were captured separately.

The overhead water crop changes from mean RGB `[94.66,102.17,102.44]` to
`[42.93,48.09,44.12]` in 8-bit display values. Dry-ground and truck controls
change by about 2 levels per channel across the separate launches. These are
image diagnostics, not an exact source-parity score; animated surfaces and
history differ between runs. Source normals, reflection, refraction and fog
remain untouched in both the archive diff and converter output.

An integration check with the real source material and native target template
produced exactly the decoded one-byte candidate config. The shared database
was then restored to its saved baseline bytes and CRC for other workers. The
release must apply the prepared patch or regenerate through the corrected
converter; screenshots of the candidate alone do not modify the release.

Camera caveat: unfocused clients can stop camera/Present callbacks while their
status worker continues updating the status file timestamp. Focus the owned
client and check counters, rather than trusting the timestamp. Do not request
captures while the initial map is still loading; blank captures are not valid
evidence. The successful images and exact pose/status metadata are preserved
with the patch plan and material contracts in the evidence directory.

Evidence is stored at
`/home/sheep/Projects/survarium/resource-porter/.work/factory-day-water-20261014/`:

- `baseline-final/` versus `fixed/`: initial matched overhead/oblique views and
  lamp controls, with original frozen hook throughout.
- `baseline-normal-ready/` versus `fixed-normal/`: settled 89.4-degree downward
  and clear oblique controls. The restored baseline reproduces the foam again.
- `patch/`: exact rollback/candidate material bytes, source contracts, hashes
  and archive offsets; final inspection reports `baseline`.
- `overhead-metrics.json`, unit and real-material integration results.

The archive is baseline after the experiment. No release fix is silently left
applied for the subsequent material and lamp investigations.
