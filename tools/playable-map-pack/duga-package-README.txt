Duga radar station (Survarium 0.69d level_11) for Survarium 0.26g
=================================================================

Map 205, mode 4 (team deathmatch, 100 tickets, 15 min), spawns from the
0.69d TDM layout (24 + 18). One local client, UI on, port 2291 by default.
Built by resource-porter branch duga-069d-to-026
(vostok-port build-survarium-069d-map, ports/survarium-069d-duga-to-026g.json).

Commands (run from this directory; nothing outside the package runtime is
touched, other Survarium sessions keep running):

  ./play-maps list
  ./play-maps verify                      # checks every packaged file's hash
  ./play-maps start duga-069d-day         # one client, UI on, port 2291
  ./play-maps status
  ./play-maps stop

Options: --port N (pick a free UDP port; avoid 2246, 2266/2268, 2276, 2280,
3234, 2300/2302), --runtime DIR (default /tmp/duga-069d-runtime; must be on
a Linux filesystem, not /mnt/hdd), --ui on|off, --clients 1|2.

The runtime keeps server.log, client.log and state.json. stop ends only this
package's server and Wine prefix.

Checklist (what to look at in game)
-----------------------------------
[ ] The map loads (map 205) and you spawn at a TDM spawn; the match starts.
[ ] Walk the stairs: silo loading-station stairs, the conveyor ramp
    (generic/silo/ladder_transition_02), the watchtowers. 0.69d "ladders"
    are stairs and should walk like stairs; there is no climbing.
[ ] Terrain: two-layer blends per 34 m tile from the 0.69d mask. Look for
    hard texture changes at tile borders, and that grass/mud/stone read right.
[ ] Lighting: native level_03 day sky and sun at the 0.69d sun direction.
    Tunnels and buildings use native indoor cubemaps in the 0.69d probe
    volumes; say if interiors are too bright/dark.
[ ] Materials: albedo/normal/smoothness from 0.69d on native templates.
    Metals look dielectric (no metalness in 0.26). Glass is cut out.
[ ] The Duga antenna and big structures at distance (LOD switching by
    distance 12/30 m; far LODs simplified). Look for popping or holes.
[ ] Batched props (repeated models baked per 64 m cell): misplaced or
    rotated copies would be a batching error.
[ ] Resupply zones (5 places, each usable by both teams).
[ ] Collision: invisible walls only where 0.69d had them (map border).
[ ] Frame rate compared with native maps (see performance notes below).

Known issues
------------
* Minimap: 0.26 has no level_11 minimap; the HUD falls back to another map's.
* The loading screen is level_03's; the map name shows the key st_level_11_name.
* 0.69d anomalies, artefacts, battle-royale/capture-point objects, ambient
  lights, sound environments and particles (cave fires are disabled in 0.69d
  TDM anyway) are not ported. Ambience is native (wind, insects, dogs).
* Performance is at the upper end of native maps; see below.
