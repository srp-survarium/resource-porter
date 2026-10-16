Duga radar station (Survarium 0.69d level_11) for Survarium 0.26g
=================================================================

Map 205, mode 4 (team deathmatch, 100 tickets, 15 min, no batteries), spawns
from the 0.69d TDM layout (42: 24 + 18). One local client, UI on, port 2317 by
default. Built by resource-porter branch duga-069d-to-026
(vostok-port build-survarium-069d-map with ports/survarium-069d-duga-to-026g.json,
then tools/playable-map-pack/build_069d_ui.py for the loading screen and minimap).

Commands (run from this directory; the runtime and Wine prefix are unpacked to
/tmp, nothing else is touched, other Survarium sessions keep running):

  ./play-maps list
  ./play-maps verify                      # checks every packaged file's hash
  ./play-maps start duga-069d-dm          # one client, UI on, port 2317
  ./play-maps status
  ./play-maps stop

Options: --port N (a free UDP port; avoid 2246, 2258, 2266/2268, 2272, 2276,
2280, 2290-2302, 2311, 2494-2498, 3234), --runtime DIR (default
/tmp/duga-069d-dm-runtime; must be on a Linux filesystem, not /mnt/hdd),
--ui on|off, --clients 1|2, --quality preset|low|medium|high,
--frame-rate N (0 = uncapped; default 60).

The runtime keeps server.log, client.log and state.json. stop ends only this
package's server and Wine prefix.

Checklist (what to look at in game)
-----------------------------------
[ ] The loading screen is Duga's own (0.69d ui_loading/level_11_day) and the
    map name reads "Duga radar station" (localized from 0.69d).
[ ] The map loads (map 205), you spawn at a TDM spawn and the match starts.
[ ] Minimap: 0.69d's map_11 minimap. Your marker should sit where you stand
    (walk to a building corner and compare).
[ ] Walk the stairs: silo loading-station stairs, the conveyor ramp
    (generic/silo/ladder_transition_02), the watchtowers. 0.69d "ladders"
    are stairs with walker ramps and should walk like stairs; there is no
    climbing in either build.
[ ] Terrain: two-layer blends per 34 m tile from the 0.69d mask; the render
    mesh is simplified with sealed tile borders (<= ~14 cm from collision).
    Look for cracks between tiles, feet sinking, hard texture seams.
[ ] Lighting: native level_03 day sky and sun at the 0.69d sun direction.
    Tunnels and buildings use native indoor cubemaps in the 0.69d probe
    volumes; say if interiors are too bright/dark.
[ ] Materials: albedo/normal/smoothness from 0.69d on native templates.
    Metals look dielectric (no metalness in 0.26). Glass is cut out.
[ ] LOD: imported models switch at 12/30 m; merged clusters (props, trees,
    repeated buildings) switch later by their size, so nothing next to you
    should look coarse. Far LODs are simplified (seams may smear at > 30 m).
    Look for popping, holes or missing/misplaced props (a cluster error).
[ ] Resupply zones (5 places, each usable by both teams).
[ ] Collision: invisible walls only where 0.69d had them (map border).
[ ] Frame rate compared with level_03 / level_07 (numbers below).

Performance (static, 42 spawns, omnidirectional upper bound; examples/view_budget)
------------------------------------------------------------------------------
                    mean tris   worst tris   mean draws   worst draws
  level_03 day_4      634 026      680 149        1 081         1 105
  level_07 day_4      790 954      905 423        1 457         1 606
  Duga (this)         814 014      965 991        1 717         1 724
Texture payload 651 MiB (level_03 638, level_07 720); particles 0 (10-11);
vegetation instances 48 286 (level_03 90 879). See validation/.

Known issues
------------
* Draw calls are ~7% above level_07's worst spawn: Duga's buildings use many
  more materials than native ones. Triangles are within ~7% of level_07.
* 0.26 has no data-driven loading screens or minimaps: the package's
  lobby_menu.swf shows Duga in loading slot 24 (Level8_night, which no 0.26
  mode uses) and hud.swf shows Duga in the level_09 minimap slot (used by no
  0.26 map; unknown HUD keys such as level_11 fall back to it).
* 0.69d anomalies, artefacts, battle-royale/capture-point objects, ambient
  lights, sound environments and particles are not ported. Ambience is
  native (wind, insects, dogs).
* Runtime status: see the report that came with this package.
