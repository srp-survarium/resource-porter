# Chemical Plant 0.11e donor inventory

The local version catalogue has the 0.11d update note but no mapped client
binary. Its note lists Chemical Plant/Radar lighting and cylindrical collision
fixes. The 0.11e note lists only an installer hotfix, suggesting it retains
those changes; the note alone does not prove runtime fidelity.

Available donor: `/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011e/app/resources.db`
(build 884). The installer `survarium_setup_v011e.exe` is also retained in
`/mnt/hdd/Projects/Survarium Builds`. PDB-derived structure and shader assembly
are catalogued under `vostok-versions/versions/0.11e/builds/884`.

Reading the database confirms only Factory projects (`level_03`, evening and
rain), Chemical Plant (`level_04`) and lobby. There is no Radar project in
this donor, so it cannot supply both requested maps.

Chemical client project comparison with 0.11a: collision object count changes
600 → 654, rendered visuals 442 → 451, scene objects 104 → 103; respawn points
remain 44. Both client and server project data differ. Index-wise field diffs
are not semantic object matching and do not establish which lighting fix moved.
Decoded projects and raw binaries are saved under the battery gallery's
`donor-011e-audit` directory. No 0.11e map has replaced the playable package yet.

`tools/chemical-plant/compare_donor_lighting.py` now matches lighting objects by
unique `full_name` and extracts both referenced post-process materials. Run it
with porter, old database, new database and output-directory arguments. The
saved report is `donor-011e-audit/lighting/comparison.json` in the gallery.
It identifies 28 changed or added lighting objects and 19 post-process changes:
three added box probes, one added spotlight, weaker local lights, changed sun
rotation, adjusted probe coverage/multipliers, tonemapping, atmospheric
scattering, AO and the cloud texture. This is a broader visual revision than a
single intensity adjustment. The sphere falloff initially looked incompatible,
but binary tracing found the same outer-radius contraction in all eight source
sphere shader variants. Source 0.11e itself has an inverted interval for this
authored sphere. Preserve the donor width rather than silently correcting its
appearance; native perturbations and the full lighting pipeline still require
runtime comparison.

The collision comparison in `chemical-collision-successor-20261014.md` finds a
specific authored wall split beside the reported blocked corridor. Nearby
boiler primitives are unchanged. A general cylinder-dimension converter patch
is therefore not supported by the current evidence.
