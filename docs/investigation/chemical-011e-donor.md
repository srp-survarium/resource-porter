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

Next comparison should match scene objects by identity, inspect environment and
light settings, and compare collision primitive definitions. Reuse existing
rendering fixes only after checking their source contracts against this donor.
