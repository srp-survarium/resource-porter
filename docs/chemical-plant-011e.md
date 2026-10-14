# Chemical Plant 0.11e candidate

This isolated DAY candidate uses build 884 under its own `v011e` namespace and
map ID **194**. It does not replace the accepted 0.11a map 193. The source database
SHA-256 is `694e1772a0595b6686cc9810afa438ed25b21360c591c0e26aa8e35dc03a4955`;
the untouched 0.26g target remains
`c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c`.

Build only the additive bundle, without allocating another full database:

```sh
python3 tools/chemical-plant/rebuild.py \
  --porter /path/to/vostok-port \
  --manifest ports/chemical-plant-011e-to-026g.json \
  --donor-db /path/to/011e/resources.db \
  --target-db /path/to/original-026g/resources.db \
  --output /new/chemical-011e-candidate --bundle-only
```

FFmpeg must be on PATH for the sound caches. This reuses the early geometry and
material conversion, moss UV precision correction, authored lamp transforms,
collector mesh binding, and native battery body-origin correction. It imports no
donor shaders. Native material templates retain their shader switches; using
native shaders does not establish matching appearance.

The 33 battery editor positions remain authored mesh origins. Their nested
`spawned_item.position` is the mesh origin plus the rotated native collision
centre offset, as established in `investigation/day-map-objectives-20261014.md`.
The source's two collectors already name different, correct west/east interaction
zones. The source-specific 0.11a duplicate-zone repair is not run on 0.11e.

## Probe mapping is not verified

All 16 probes explicitly store `probe_side_width`. Fifteen are boxes, with widths
0.5–1.4 metres. The global sphere at `[41.456, 23.794, 25.365]` has radius
340.901 and width 0.5. Its texture is
`environment_probes/level_04/probe_586386534696113540`.

The investigation candidate preserves these explicit donor fields and emits an
`UNVERIFIED` compatibility note. This is not a validly established target mapping:
the observed 0.26 upload uses `outer=.98*radius` and
`inner=radius-width-.05`, which inverts the sphere's transition interval here.
The older adapter's synthetic `1+.02*radius` width must not silently replace a
newly explicit authored setting. Source 0.11e upload and shader evidence must
settle the mapping before runtime promotion. Existing 0.10b/0.11a behavior is
unchanged.

## Collision provenance

The candidate preserves donor collision payloads. The collision audit worker
found a meaningful authored opening in
`old_factory_base.model/walker_collision`: a former broad wall spanning
Z26.43–30.83 becomes a narrower main wall at Z29.11–30.84 and an upper small
piece at Z26.40–27.07. This is near the reported blocked corridor. It is a donor
change, not a generated relocation.

Boiler geometry itself is unchanged from 0.11a. Legacy cylinder creation forwards
XYZ dimensions while native 0.26 constructs XYX. Copying the payload therefore
does not prove collision parity; no blanket cylinder rewrite is included.

## CPU validation

`audit_011e_candidate.py` hashes every emitted file, checks the explicit source
identity, verifies every authored battery position and its native body offset,
checks distinct collector bindings, and records the probe interval issue. Supply
the original binary client/server configs and native battery options as inputs.
The build refuses unresolved dependency closure and imports only map assets.
The existing 21 map conversion and four material conversion tests pass; no test
functions were added. Runtime load, appearance, and collision playthrough remain
unverified for this separate candidate.
