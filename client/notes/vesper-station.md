# Vesper Station

Zombies map, night. Plain-shapes blockout built by `tools/blender/vesper_station.py`
(`blender --background --python tools/blender/vesper_station.py`), which writes
`client/assets/models/maps/vesper_station_map.glb`. Edit the script and re-run it to change
the map, then rebuild the client and the server (the server embeds the `.glb`).

```
          x=-42          x=-14          x=14            x=42
  z=-69  +--------------+--------------+ - - rock - - - +
         |  POWER       |  LAB (PaP)   |                |
         |  (plateau,   |  (plateau,   +----------------+ z=-51
         |   y=3)  [F]  |   y=3)   [E] <-ramp  RAIL     |
  z=-19  +-----[C]------+--------------+       YARD     |
         |  BARRACKS    |  SPAWN       |      (open,    |
         |  (2 floors)  [A]  (start)  [B]   training)   |
  z=19   +--------------+--------------+----------------+
```

## Doorways for buyable doors (2.6 m wide, 3 m tall)

| Door | Between                        | Middle (x, y, z)    | Facing |
|------|--------------------------------|---------------------|--------|
| A    | spawn → barracks               | (-14, 0, 8)         | along x |
| B    | spawn → rail yard              | (14, 0, 8)          | along x |
| C    | barracks yard → power (top of the outdoor stairs) | (-40.3, 3, -19) | along z |
| E    | rail yard → lab (top of the ramp) | (14, 3, -28)     | along x |
| F    | power ↔ lab                    | (-14, 3, -40)       | along x |

- Spawn → power: A then C. Spawn → Pack-a-Punch: B then E. All five open = one big loop.
- What's where: Quick Revive, ammo crate and sniper wall buy in spawn. Mystery Box, crafting
  table and Juggernog (upstairs) in the barracks. Speed Cola, Stamin-Up, armor, AK and the
  exfil in the rail yard. Power switch, Double Tap and the Rampage Inducer in the power zone.
  Pack-a-Punch, Deadshot and Der Wunderfizz (on the mezzanine) in the lab.

## Windows

Outer-wall windows (sill 1 m, 1.6 × 1.4 m) look out onto a 4 m "zombie closet" strip behind
the map's outer wall. That's where zombies would climb in once barriers exist. The other
windows (between zones, and down from the lab over spawn) are just for seeing through. Every
doorway and window has an empty named `Door_<wall>_<n>` / `Window_<wall>_<n>` at its middle in
the `.glb`.
