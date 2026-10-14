# Radar donor identity and local availability

Radar is present in the local 0.11e donor as `projects/level_03/client_project`,
with evening and rain siblings. The investigation's earlier “Factory” label for
that same `level_03` map is misleading. Chemical Plant is `level_04`.
This audit changes no package, map ID, donor choice, or release scope.

## Map identity

The original 0.11e project itself contains the radar dish
`level/03/antenna/antenna_tarelka`, antenna base, `barracks_x3`, and `hangar`.
Its portal groups are named `antenna`, `barracks`, and `hangar`. These four
landmarks persist with identical authored transforms in the independently read
0.11b, 0.11c, 0.20e, and 0.26g `level_03` projects.

Of 1,224 original 0.11e render instances, **1,202** have exactly the same model,
position, rotation, and scale as the frozen 0.10b project previously labelled
“Factory” (1,233 instances). This establishes physical map continuity; it does
not imply every model payload or gameplay layout is unchanged.

Direct native 0.26g localization reads:

| Key | English | Russian |
| --- | --- | --- |
| `st_level_03_name` | Vostok Radar Station | РЛС "Восток" |
| `st_level_04_name` | Chemical Plant | Химзавод |

Native resource provenance: `localization/english/localization` SHA256
`666332eeb7fedf67d7a7c8d344f8e374e58cf161f46b88f83e66b9185021a6dd`;
`localization/russian/localization` SHA256
`33b1b1680528ca1149097ff38918f271a25caf4eb6d502e8f1a425fa272f0f4e`.

Original `projects/level_03/client_project` SHA256:
0.10b `61406bd7b19c578e9351be1767be50be2534479663875ae9b485bec961a03947`;
0.11e `25a44a842ac1535b678b5e817a4e83bc4ea962dc69eda7d1b6559acc01a80671`.

The original 0.11e localization has no corresponding map-name label; none is
invented. Independent archived historical notes support the physical identity:
0.1 identifies its sole launch map as Vostok Radar Station; 0.11a names Radar's
automotive hangar and barracks; 0.11d names Chemical Plant and Radar in its
lighting correction. Together with the original project's radar structures
and cross-version continuity, this identifies the available early `level_03`.

## Local release availability

| Label | Local evidence |
| --- | --- |
| 0.11a | Extracted donor and installer; build 816 |
| 0.11b | Extracted donor and installer; build 826 |
| 0.11c | Extracted donor and installer; build 870 |
| 0.11d | Archived update note only; maintained catalogue maps no binary and its 2026-10-11 archive search found no full build |
| 0.11e | Extracted donor and installer; build 884, executable version `0.1.1e`, date 2013-05-28 |
| 0.11g | No matching label, installer, extracted donor, or entry in the inspected local release/archive catalogues |

Absence from those catalogues does not prove that an unlisted historical build
never existed. The user's “0.11g” remains a clarification point; no automatic
substitution is authorized by this audit. Public 0.11e is recorded as the
installer hotfix after 0.11d. Public-release labels and executable dates are
retained separately rather than treating the dates as identical.

Relevant paths:

- 0.11e database:
  `/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011e/app/resources.db`.
- 0.11e installer:
  `/mnt/hdd/Projects/Survarium Builds/survarium_setup_v011e.exe`.
- Local archive catalogue:
  `/mnt/hdd/Projects/Survarium Builds/archive-catalog-2026-10-11/{coverage,manifest}.json`.
- Maintained release notes:
  `/home/sheep/Projects/survarium/vostok-versions/versions/0.11{d,e}`.
- Frozen audit, including actual project resource hashes, landmark transforms,
  localization payload hashes, physical overlap count, catalogue extracts,
  reproduction script, and file hashes:
  `/tmp/resource-porter-chemical-011/.work/radar-donor-audit` (about 35 KB).

Only archive directories and small indexed resources were read. No downloads,
large copies, client launch, or database/package mutation were performed.
