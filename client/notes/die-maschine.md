# Die Maschine

Zombies map, night: a plain-shapes blockout of Black Ops Cold War's Die Maschine, at this game's
scale (doors 2.6 × 3 m, storeys 3.6 m). Built by `tools/blender/die_maschine.py`
(`blender --background --python tools/blender/die_maschine.py`, add `-- --previews <dir>` for
renders), which writes `client/assets/models/maps/die_maschine_map.glb`. Rebuild the client and
the server after changing it (the server embeds the `.glb`).

Layout researched from the COD Zombified area maps (codzombified.blogspot.com) — it follows
how the areas connect and what's in each, not exact measurements.

```
 TUNNEL ENTRANCE [3]  CRASH SITE (plane, Jugg)
  (ramp down)    |     ^ Penthouse ramp, Bedroom stairs
 ----[2]---[P]---+  NACHT (Omega Outpost / Living Room,      YARD (spawn)
 POND (QR) <- Mezzanine stairs    Bedroom / Mezzanine,  [1]  power door -> stairs down to
                                  Penthouse roof)               the Particle Accelerator
 underground: Tunnel -> Control Room -> Medical Bay (2 floors) -> Particle Accelerator
                                    -> Weapons Lab (2 floors) -> Particle Accelerator
```

## Doorways for buyable doors (Die Maschine's prices)

| Doorway                                         | Middle (x, y, z)     | Price |
|-------------------------------------------------|----------------------|-------|
| Yard → Omega Outpost                            | (-16, 0, -5)         | 500   |
| Yard → Living Room                              | (-16, 0, 5)          | 500   |
| Omega Outpost ↔ Living Room (power door)        | (-28, 0, 0)          | power |
| Outpost stairs up to Bedroom                    | (-38.6, 0, -1)       | 750   |
| Living Room stairs up to Mezzanine              | (-25.5, 0, 8.6)      | 750   |
| Bedroom ↔ Mezzanine (power door)                | (-28, 3.6, 0)        | power |
| Bedroom → Crash Site (outside stairs)           | (-22.5, 3.6, -10)    | 1000  |
| Mezzanine → Pond (outside stairs)               | (-40, 3.6, 5)        | 1250  |
| Penthouse → Crash Site (debris ramp)            | (-38.1, 7.2, -10)    | 1000  |
| Pond → Tunnel entrance                          | (-74, 0, -24)        | 1250  |
| Pond ↔ Crash Site (power door)                  | (-52, 0, -24)        | power |
| Crash Site → Tunnel entrance                    | (-64, 0, -44)        | 1250  |
| Control Room → Medical Bay                      | (-32, -6, -46)       | 1500  |
| Control Room → Weapons Lab                      | (-20, -6, -60)       | 1500  |
| Medical Bay → Particle Accelerator              | (-18, -10, -18)      | 1500  |
| Weapons Lab → Particle Accelerator              | (8, -6, -44)         | 1500  |
| Yard ↔ Particle Accelerator stairs (power door) | (16, 0, 13.5)        | power |

The Tunnel → Control Room doorway is the game's automatic door, so it has no price. Every
doorway also has an empty named `Door_<wall>_<n>` at its middle in the `.glb`.

## Where things are

- **Yard:** spawn, Mystery Box, ammo crate, Rampage Inducer, exfil radio.
- **Crash Site:** Juggernog, the exfil area.
- **Pond:** Quick Revive.
- **Bedroom:** Stamin-Up.
- **Penthouse:** Der Wunderfizz, standing in for Elemental Pop.
- **Tunnel:** crafting table.
- **Control Room:** Double Tap and the AK wall buy.
- **Medical Bay:** Speed Cola, on the lower floor.
- **Weapons Lab:** Deadshot, in the pit.
- **Particle Accelerator:** the power switch, Pack-a-Punch on the middle platform, the armor
  station and the sniper wall buy.

The Crash Site cockpit has a purple patch where the Aether portal is, but it doesn't take you
anywhere.
