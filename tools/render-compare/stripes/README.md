Offline checks from the parked "stripes on low mips" item
(docs/investigation/day-map-calibration-20261015.md, "Open item (parked by
the user): stripes on low mips"). All read a resources.db through
`target/release/vostok-port stat/dump-db-config` and the raw entry bytes; none
launch the game.

- `audit_dds_layout.py DB LIST`: header + mip-chain size against the file size.
- `audit_dds_trailing_byte.py DB LIST`: trailing byte against `.options` srgb/format.
- `emulate_dropped_mip_offsets.py DB LIST`: 0.26g's dropped-mip start offset
  (0x3E17D0/0x3E16B0) against the true offset, for every drop count.
- `dxt_mip_stats.py DDS...`: per-mip statistics and distance from a box filter.
- `moss_lift_clearance.py GEO_DIR`: ray casts for the moss lift (edge gaps,
  vertices reaching other geometry) against exported static-model geometry.

LIST is a file of `textures/...dds` paths. The numpy-based scripts need a
Python with numpy.
