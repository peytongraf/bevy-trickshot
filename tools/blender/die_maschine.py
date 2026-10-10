"""Builds `client/assets/models/maps/die_maschine_map.glb` — a plain-shapes
blockout of Black Ops Cold War's Die Maschine, at this game's scale (metres;
the player is 1.8 m: doors 2.6 x 3 m, storeys 3.6 m, stair risers 0.2 m).

Run from the repo root (Blender 4+):

    blender --background --python tools/blender/die_maschine.py
    blender --background --python tools/blender/die_maschine.py -- --previews /some/dir

Numbers are in the GAME's frame — x right (east), y up, z toward the camera
(south); north is -z — and converted to Blender's z-up frame when the mesh is
built. The Yard (the spawn) is at the origin; placement scale 1.

THE SURFACE (y = 0), looking down, north at the top:

          x=-84        x=-64          x=-40     x=-16     x=16
   z=-64  +-------------+--------------+-------------------+
          |  TUNNEL     [3]   CRASH SITE (plane, Jugg)     |
          |  ENTRANCE   |              |                   |
          |  (ramp down)|       ramp from the Penthouse    |
   z=-24  +----[2]------+-[P]----------+  bedroom stairs   |
          |                            | NACHT     +-------+ z=-16
          |  POND (QR)   mezz stairs ->| (Omega    | YARD  |
          |                            | Outpost)  [1]  (spawn)
          |                            +-----------+  [PA] -> stairs down
   z=30   +----------------------------+           +-------+ z=16

   Nacht: ground floor = Omega Outpost (north) + Living Room (south), both
   off the Yard; upstairs = Bedroom (north, outside stairs down to the Crash
   Site) + Mezzanine (south, outside stairs down to the Pond); the roof is
   the Penthouse (Der Wunderfizz), with a debris ramp down to the Crash Site.

UNDERGROUND (floor y = -6; the two-floor rooms go down to y = -10):

   Tunnel (from the ramp) -> Control Room -> Medical Bay (south, two
   floors) -> Particle Accelerator (lower floor)
                          -> Weapons Lab (east, two floors round a pit)
                             -> Particle Accelerator (upper ring)
   Particle Accelerator, under the Yard: an upper ring walkway round a bowl
   with Pack-a-Punch in the middle, and the power switch; stairs up to the
   Yard's power door.

Every doorway is left open for buyable doors later; each gets an empty named
`Door_*` (windows `Window_*`) at its middle, listed in
`client/notes/die-maschine.md`.
"""

import math
import os
import sys
from collections import defaultdict

import bmesh
import bpy
from mathutils import Vector

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(REPO, "client", "assets", "models", "maps", "die_maschine_map.glb")

T = 0.4  # wall thickness
DOOR_W, DOOR_H = 2.6, 3.0
WIN_W, WIN_SILL, WIN_TOP = 1.6, 1.0, 2.4
BOUND_H = 12.0  # outer / zone walls: out of mantle reach from anything near them
U = -6.0  # underground floor
U2 = -10.0  # underground lower floor
CEIL = -1.0  # underground ceiling (the surface slab's underside)

MATERIALS = {
    "snow": ((0.80, 0.82, 0.85), 0.9, 0.0),
    "mud": ((0.28, 0.24, 0.20), 1.0, 0.0),
    "concrete": ((0.48, 0.48, 0.47), 0.9, 0.0),
    "bunker": ((0.36, 0.37, 0.36), 0.9, 0.0),
    "plaster": ((0.55, 0.50, 0.42), 0.9, 0.0),
    "wood": ((0.40, 0.28, 0.17), 0.85, 0.0),
    "rock": ((0.30, 0.29, 0.28), 1.0, 0.0),
    "roof": ((0.17, 0.16, 0.16), 0.9, 0.0),
    "ice": ((0.55, 0.68, 0.75), 0.15, 0.0),
    "metal": ((0.34, 0.36, 0.37), 0.5, 0.8),
    "lab": ((0.62, 0.64, 0.62), 0.6, 0.1),
    "lab_floor": ((0.30, 0.32, 0.32), 0.7, 0.2),
    "rust": ((0.42, 0.24, 0.13), 0.8, 0.4),
    "olive": ((0.27, 0.30, 0.18), 0.8, 0.2),
    "plane": ((0.45, 0.47, 0.42), 0.6, 0.5),
    "tree": ((0.24, 0.17, 0.11), 1.0, 0.0),
    "crate": ((0.50, 0.38, 0.22), 0.9, 0.0),
    "aether": ((0.45, 0.20, 0.70), 0.3, 0.0),
    "lamp": ((1.0, 0.9, 0.7), 0.4, 0.0),
}

PARTS = defaultdict(lambda: ([], []))
MARKERS = []


def g2b(x, y, z):
    return (x, -z, y)


def solid(zone, mat, corners, faces):
    verts, fs = PARTS[(zone, mat)]
    base = len(verts)
    verts.extend(g2b(*c) for c in corners)
    fs.extend([tuple(base + i for i in f) for f in faces])


HEX_FACES = [(0, 1, 2, 3), (4, 7, 6, 5), (0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0)]


def box(zone, mat, x0, x1, y0, y1, z0, z1):
    if x1 - x0 < 1e-4 or y1 - y0 < 1e-4 or z1 - z0 < 1e-4:
        return
    c = [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1),
         (x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)]
    solid(zone, mat, c, HEX_FACES)


def obox(zone, mat, cx, cz, sx, sz, y0, y1, yaw_deg):
    """A box `sx` (x) by `sz` (z) round (cx, cz), turned `yaw_deg` about y."""
    a = math.radians(yaw_deg)
    ca, sa = math.cos(a), math.sin(a)

    def at(dx, dz):
        return (cx + dx * ca + dz * sa, cz - dx * sa + dz * ca)

    hx, hz = sx / 2, sz / 2
    ring = [at(-hx, -hz), at(hx, -hz), at(hx, hz), at(-hx, hz)]
    c = [(x, y0, z) for x, z in ring] + [(x, y1, z) for x, z in ring]
    solid(zone, mat, c, HEX_FACES)


def slab(zone, mat, x0, x1, z0, z1, y0, y1, holes=()):
    """A floor / ceiling slab with rectangular holes (x0, x1, z0, z1)."""
    xs = sorted({x0, x1, *(c for h in holes for c in (h[0], h[1]) if x0 < c < x1)})
    zs = sorted({z0, z1, *(c for h in holes for c in (h[2], h[3]) if z0 < c < z1)})
    for a0, a1 in zip(xs, xs[1:]):
        for b0, b1 in zip(zs, zs[1:]):
            mx, mz = (a0 + a1) / 2, (b0 + b1) / 2
            if any(h[0] < mx < h[1] and h[2] < mz < h[3] for h in holes):
                continue
            box(zone, mat, a0, a1, y0, y1, b0, b1)


def ramp_z(zone, mat, x0, x1, za, ya, zb, yb, base):
    c = [(x0, base, za), (x1, base, za), (x1, base, zb), (x0, base, zb),
         (x0, ya, za), (x1, ya, za), (x1, yb, zb), (x0, yb, zb)]
    solid(zone, mat, c, HEX_FACES)


def ramp_x(zone, mat, z0, z1, xa, ya, xb, yb, base):
    c = [(xa, base, z0), (xb, base, z0), (xb, base, z1), (xa, base, z1),
         (xa, ya, z0), (xb, yb, z0), (xb, yb, z1), (xa, ya, z1)]
    solid(zone, mat, c, HEX_FACES)


def stairs(zone, mat, axis, a_start, direction, lo, hi, y_base, rise, run, count):
    """`count` steps along `axis` from `a_start`, climbing `rise` each as
    they go `direction` (+1 / -1); `lo..hi` is their width."""
    for i in range(count):
        a0 = a_start + direction * run * i
        a1 = a_start + direction * run * (i + 1)
        a0, a1 = min(a0, a1), max(a0, a1)
        top = y_base + rise * (i + 1)
        if axis == "x":
            box(zone, mat, a0, a1, y_base - 0.5, top, lo, hi)
        else:
            box(zone, mat, lo, hi, y_base - 0.5, top, a0, a1)


def cylinder(zone, mat, cx, cz, r, y0, y1, seg=20):
    c = []
    for y in (y0, y1):
        for i in range(seg):
            a = 2 * math.pi * i / seg
            c.append((cx + r * math.cos(a), y, cz + r * math.sin(a)))
    faces = [tuple(range(seg - 1, -1, -1)), tuple(range(seg, 2 * seg))]
    for i in range(seg):
        j = (i + 1) % seg
        faces.append((i, j, seg + j, seg + i))
    solid(zone, mat, c, faces)


def wall(zone, mat, axis, fixed, a0, a1, y0, height, openings=(), thick=T, name=""):
    """A wall along `axis` ('x' at z = `fixed`, 'z' at x = `fixed`) from a0 to
    a1, y0 up `height`; openings (centre, width, bottom, top, kind) above y0
    may stack or overlap."""
    half = thick / 2

    def piece(b0, b1, ya, yb):
        if axis == "x":
            box(zone, mat, b0, b1, ya, yb, fixed - half, fixed + half)
        else:
            box(zone, mat, fixed - half, fixed + half, ya, yb, b0, b1)

    holes = []
    for n, (centre, width, bottom, top, kind) in enumerate(openings):
        o0, o1 = centre - width / 2, centre + width / 2
        assert a0 < o0 and o1 < a1, f"{name}: opening at {centre} outside the wall"
        holes.append((o0, o1, y0 + bottom, y0 + top))
        mid_y = y0 + (bottom + top) / 2
        at = (centre, mid_y, fixed) if axis == "x" else (fixed, mid_y, centre)
        MARKERS.append((f"{'Door' if kind == 'door' else 'Window'}_{name}_{n + 1}", at))
    cuts = sorted({a0, a1, *(h[0] for h in holes), *(h[1] for h in holes)})
    for c0, c1 in zip(cuts, cuts[1:]):
        mid = (c0 + c1) / 2
        y = y0
        for _, _, yb, yt in sorted((h for h in holes if h[0] < mid < h[1]), key=lambda h: h[2]):
            piece(c0, c1, y, yb)
            y = max(y, yt)
        piece(c0, c1, y, y0 + height)


def door(centre, width=DOOR_W, height=DOOR_H, bottom=0.0):
    return (centre, width, bottom, bottom + height, "door")


def window(centre, sill=WIN_SILL, top=WIN_TOP, width=WIN_W):
    return (centre, width, sill, top, "window")


def crate(zone, x, z, y=0.0, size=1.2):
    h = size / 2
    box(zone, "crate", x - h, x + h, y, y + size, z - h, z + h)


def lamp_post(zone, x, z, y=0.0, height=6.0):
    cylinder(zone, "metal", x, z, 0.12, y, y + height, seg=10)
    box(zone, "lamp", x - 0.3, x + 0.3, y + height, y + height + 0.25, z - 0.3, z + 0.3)


def tree(zone, x, z, height=11.0):
    cylinder(zone, "tree", x, z, 0.45, -0.5, height, seg=10)


def railing(zone, x0, x1, z0, z1, y):
    box(zone, "metal", x0, x1, y, y + 1.0, z0, z1)


# --- the surface -----------------------------------------------------------------


def yard():
    z = "yard"
    slab(z, "snow", -16, 16, -16, 16, -1, 0)
    # Rock walls round it — the Nacht building is its west side.
    wall(z, "rock", "x", -16, -16.2, 16.2, -1, BOUND_H + 1, name="YardNorth")
    wall(z, "rock", "x", 16, -16.2, 16.2, -1, BOUND_H + 1, name="YardSouth")
    wall(z, "rock", "z", 16, -16, 16, -1, BOUND_H + 1, [door(13.5, bottom=1.0)], name="YardEast")  # power door: down to the PA
    wall(z, "rock", "z", -16, -16, -10, -1, BOUND_H + 1, name="YardWestN")
    wall(z, "rock", "z", -16, 10, 16, -1, BOUND_H + 1, name="YardWestS")
    # Rocks cutting the corners (the yard's rough octagon).
    obox(z, "rock", -13.0, 13.0, 7, 2.5, 0, 4, -45)
    # The stone steps up to a ledge in the north-east, the broken tank.
    box(z, "concrete", 5, 15.8, -1, 1.2, -15.8, -10)
    stairs(z, "concrete", "z", -7.6, -1, 5, 15.8, 0, 0.2, 0.4, 6)
    obox(z, "olive", 3, 6, 3.4, 6.6, 0, 2.0, 30)
    obox(z, "olive", 3, 6, 2.4, 2.6, 2.0, 2.9, 30)
    obox(z, "metal", 1.4, 3.2, 0.3, 3.2, 2.3, 2.6, 30)
    crate(z, -6, 12)
    crate(z, -4.8, 12.6)
    lamp_post(z, -8, 10)
    lamp_post(z, 10, -5)


def nacht():
    z = "nacht"
    x0, x1, z0, z1 = -40, -16, -10, 10
    slab(z, "wood", x0, x1, z0, z1, -1, 0)
    up, roof, top = 3.6, 7.2, 11.2
    # Exterior walls, ground floor to the top of the Penthouse's walls.
    wall(z, "plaster", "x", z0, x0 - 0.2, x1 + 0.2, 0, top, [
        window(-34), window(-28), window(-22),
        door(-22.5, bottom=up),  # Bedroom -> Crash Site stairs
        window(-34, up + 1.0, up + 2.4),
        door(-38.1, bottom=roof),  # Penthouse -> debris ramp
        window(-28, roof + 1.0, roof + 2.4), window(-20, roof + 1.0, roof + 2.4),
    ], name="NachtNorth")
    wall(z, "plaster", "x", z1, x0 - 0.2, x1 + 0.2, 0, top, [
        window(-34), window(-22),
        window(-34, up + 1.0, up + 2.4),
        window(-28, roof + 1.0, roof + 2.4), window(-20, roof + 1.0, roof + 2.4),
    ], name="NachtSouth")
    wall(z, "plaster", "z", x1, z0, z1, 0, top, [
        door(-5), door(5),  # Yard -> Omega Outpost, Yard -> Living Room
        window(-5, up + 1.0, up + 2.4), window(5, up + 1.0, up + 2.4),
        window(-5, roof + 1.0, roof + 2.4), window(5, roof + 1.0, roof + 2.4),
    ], name="NachtEast")
    wall(z, "plaster", "z", x0, z0, z1, 0, top, [
        window(-5), window(5),
        window(-5, up + 1.0, up + 2.4),
        door(5, bottom=up),  # Mezzanine -> Pond stairs
        window(-5, roof + 1.0, roof + 2.4), window(2, roof + 1.0, roof + 2.4),
    ], name="NachtWest")
    # Omega Outpost | Living Room, Bedroom | Mezzanine (power doors).
    wall(z, "plaster", "x", 0, x0 + 0.2, x1 - 0.2, 0, up - 0.3, [door(-28)], name="NachtGroundSplit")
    wall(z, "plaster", "x", 0, x0 + 0.2, x1 - 0.2, up, roof - up - 0.3, [door(-28)], name="NachtUpperSplit")

    # Upstairs floor, holes for the two staircases up to it.
    main_hole = (-39.8, -37.4, -8.0, -0.2)
    living_hole = (-26.1, -18.4, 7.6, 9.8)
    slab(z, "wood", x0 + 0.2, x1 - 0.2, z0 + 0.2, z1 - 0.2, up - 0.3, up, [main_hole, living_hole])
    stairs(z, "wood", "z", -0.4, -1, -39.6, -37.6, 0, 0.2, 0.42, 18)  # Outpost -> Bedroom
    stairs(z, "wood", "x", -26.0, 1, 7.6, 9.6, 0, 0.2, 0.42, 18)  # Living Room -> Mezzanine
    railing(z, -37.4, -37.2, -8.0, -0.2, up)
    railing(z, -26.1, -18.4, 7.4, 7.6, up)
    railing(z, -26.3, -26.1, 7.6, 9.8, up)
    # The roof (the Penthouse) and its stairs up from the Mezzanine.
    roof_hole = (-37.6, -29.9, 7.6, 9.8)
    slab(z, "roof", x0 + 0.2, x1 - 0.2, z0 + 0.2, z1 - 0.2, roof - 0.3, roof, [roof_hole])
    stairs(z, "wood", "x", -30.0, -1, 7.6, 9.6, up, 0.2, 0.42, 18)
    railing(z, -37.6, -29.9, 7.4, 7.6, roof)
    railing(z, -29.9, -29.7, 7.6, 9.8, roof)

    # Outside stairs: Bedroom down to the Crash Site, Mezzanine down to the
    # Pond; the Penthouse's debris ramp down to the Crash Site.
    stairs(z, "wood", "z", -19.2, 1, -24, -21, 0, 0.2, 0.5, 18)
    stairs(z, "wood", "x", -49.2, 1, 3.5, 6.5, 0, 0.2, 0.5, 18)
    ramp_z(z, "rock", -39.6, -36.6, -10.2, roof, -28, 0.0, -0.5)

    # Furniture.
    box(z, "wood", -32, -29, 0, 0.9, -6, -4)  # Outpost table
    box(z, "wood", -24, -20, 0, 0.8, 2, 3.5)  # Living Room sofa
    box(z, "wood", -36, -34, up, up + 0.6, -8.5, -6)  # beds
    box(z, "wood", -20, -18, up, up + 0.6, -8.5, -6)
    crate(z, -20, 3, up)

    # The strip behind it, between the Pond and the Yard (zombie closet).
    slab(z, "mud", x0, x1, z1, 14, -1, 0)
    wall(z, "rock", "x", 14, x0 - 0.2, x1 + 0.2, -1, BOUND_H + 1, name="NachtBack")


def crash_site():
    z = "crash"
    slab(z, "snow", -64, 16, -64, -24, -1, 0)
    slab(z, "snow", -40, 16, -24, -16, -1, 0)
    slab(z, "snow", -40, -16, -16, -10, -1, 0)
    wall(z, "rock", "x", -64, -84.2, 16.2, -1, BOUND_H + 1, name="North")
    wall(z, "rock", "z", 16, -64, -16, -1, BOUND_H + 1, name="CrashEast")
    wall(z, "rock", "z", -40, -24, -10, -1, BOUND_H + 1, name="CrashPond")
    wall(z, "rock", "z", -64, -64, -24, -1, BOUND_H + 1, [door(-44, bottom=1.0)], name="CrashTunnel")

    # The downed plane: fuselage, tail, wing, and the cockpit broken off.
    obox(z, "plane", -10, -42, 4.2, 22, 0, 4.0, 25)
    obox(z, "plane", -15.3, -53.4, 0.5, 4, 4.0, 7.0, 25)  # tail fin
    obox(z, "plane", -10, -42, 24, 3.2, 0, 1.2, 25)  # wing
    obox(z, "plane", -4.5, -30.0, 4.0, 5.0, 0, 3.2, 40)  # cockpit
    obox(z, "aether", -4.5, -30.0, 3.0, 3.0, 0, 0.05, 40)  # the portal under it
    # Two rises with ramps up ("UP" on the guides).
    box(z, "rock", -36, -28, -1, 1.5, -40, -34)
    ramp_z(z, "rock", -36, -28, -30, 0.0, -34, 1.5, -0.5)
    box(z, "rock", 4, 12, -1, 1.5, -26, -20)
    ramp_x(z, "rock", -26, -20, -1, 0.0, 4, 1.5, -0.5)
    for tx, tz in ((-58, -58), (-50, -30), (12, -60), (12, -30), (-30, -60), (-30, -20)):
        tree(z, tx, tz)
    crate(z, -46, -58)
    crate(z, -44.8, -58.6)
    crate(z, 10, -36)
    lamp_post(z, -30, -50)
    lamp_post(z, 0, -30)


def pond():
    z = "pond"
    slab(z, "mud", -84, -40, -24, 30, -1, 0)
    wall(z, "rock", "z", -84, -64, 30.2, -1, BOUND_H + 1, name="West")
    wall(z, "rock", "x", 30, -84.2, -39.8, -1, BOUND_H + 1, name="PondSouth")
    wall(z, "rock", "z", -40, 14, 30, -1, BOUND_H + 1, name="PondEast")
    wall(z, "rock", "z", -40, 10, 14, -1, BOUND_H + 1, name="PondEastCloset")
    wall(z, "rock", "x", -24, -84, -40, -1, BOUND_H + 1, [
        door(-74, bottom=1.0),  # -> Tunnel entrance
        door(-52, bottom=1.0),  # -> Crash Site (power door)
    ], name="PondNorth")
    box(z, "ice", -72, -54, 0, 0.03, -4, 14)  # the frozen pond
    obox(z, "rust", -50, -14, 2.6, 5.5, 0, 2.2, 70)  # the old truck
    obox(z, "rust", -47.2, -13.0, 2.4, 2.2, 0, 2.8, 70)
    for tx, tz in ((-78, 24), (-60, 25), (-80, -10), (-46, 26), (-62, -18)):
        tree(z, tx, tz)
    obox(z, "rock", -76, 6, 4, 6, 0, 2.5, 20)
    crate(z, -56, 22)
    lamp_post(z, -60, 20)


def tunnel_entrance():
    z = "tunnel"
    # Plaza round the trench the tunnel ramp runs down.
    slab(z, "concrete", -84, -64, -64, -24, -1, 0, [(-78, -72, -50, -30)])
    ramp_z(z, "concrete", -78, -72, -30, 0.0, -56, U, U - 0.5)
    box(z, "bunker", -78.2, -78, U - 0.5, 0, -56, -30)  # trench sides
    box(z, "bunker", -72, -71.8, U - 0.5, 0, -56, -30)
    box(z, "bunker", -78.6, -71.4, -1, 1.5, -50.4, -50)  # bunker lintel
    lamp_post(z, -68, -30)


# --- underground -----------------------------------------------------------------


def room(zone, x0, x1, z0, z1, floor_y, walls, wall_mat="bunker", floor_mat="lab_floor", ceiling=True):
    """An underground room's floor, ceiling and walls. `walls` maps
    'N'/'S'/'E'/'W' to that wall's openings (None = no wall)."""
    box(zone, floor_mat, x0, x1, floor_y - 1, floor_y, z0, z1)
    if ceiling:
        box(zone, "bunker", x0 - 0.2, x1 + 0.2, CEIL - 0.4, CEIL, z0 - 0.2, z1 + 0.2)
    h = CEIL - floor_y
    for side, openings in walls.items():
        if openings is None:
            continue
        name = f"{zone}{side}"
        if side == "N":
            wall(zone, wall_mat, "x", z0, x0 - 0.2, x1 + 0.2, floor_y, h, openings, name=name)
        elif side == "S":
            wall(zone, wall_mat, "x", z1, x0 - 0.2, x1 + 0.2, floor_y, h, openings, name=name)
        elif side == "W":
            wall(zone, wall_mat, "z", x0, z0, z1, floor_y, h, openings, name=name)
        else:
            wall(zone, wall_mat, "z", x1, z0, z1, floor_y, h, openings, name=name)


def labs():
    # Tunnel: from the ramp's foot east to the Control Room.
    z = "Tunnel"
    box(z, "lab_floor", -78, -44, U - 1, U, -62, -56)
    box(z, "bunker", -78.2, -44, CEIL - 0.4, CEIL, -62.2, -55.8)
    wall(z, "bunker", "x", -62, -78.2, -44, U, CEIL - U, name="TunnelN")
    wall(z, "bunker", "x", -56, -72, -44, U, CEIL - U, name="TunnelS")
    wall(z, "bunker", "z", -78, -62, -56, U, CEIL - U, name="TunnelW")
    crate(z, -50, -57.2, U)
    lamp = (-60, -59)
    box(z, "lamp", lamp[0] - 0.4, lamp[0] + 0.4, CEIL - 0.6, CEIL - 0.4, lamp[1] - 0.4, lamp[1] + 0.4)

    # Control Room: trial pedestals in the middle.
    z = "Control"
    room(z, -44, -20, -72, -46, U, {
        "N": [], "W": [door(-59, 5.6, 4.0)], "E": [door(-60)], "S": [door(-32)],
    })
    for px in (-35, -29):
        for pz in (-64, -58):
            box(z, "metal", px - 0.6, px + 0.6, U, U + 1.0, pz - 0.6, pz + 0.6)
    box(z, "metal", -40, -36, U, U + 1.2, -71.6, -70.6)  # consoles
    box(z, "metal", -32, -28, U, U + 1.2, -71.6, -70.6)

    # Control Room -> Weapons Lab corridor, and -> Medical Bay corridor.
    z = "Corridors"
    box(z, "lab_floor", -20, -12, U - 1, U, -63, -57)
    box(z, "bunker", -20, -12, CEIL - 0.4, CEIL, -63.2, -56.8)
    wall(z, "bunker", "x", -63, -20, -12, U, CEIL - U, name="CorrCWN")
    wall(z, "bunker", "x", -57, -20, -12, U, CEIL - U, name="CorrCWS")
    box(z, "lab_floor", -33.5, -30.5, U - 1, U, -46, -40)
    box(z, "bunker", -33.7, -30.3, CEIL - 0.4, CEIL, -46, -40)
    wall(z, "bunker", "z", -33.5, -46, -40, U, CEIL - U, name="CorrCMW")
    wall(z, "bunker", "z", -30.5, -46, -40, U, CEIL - U, name="CorrCME")

    # Medical Bay: a walkway (y -6) round three sides of a lower floor
    # (y -10), stairs down at both ends; Dr Vogel's machine in the middle.
    z = "Medical"
    x0, x1, z0, z1 = -46, -18, -40, -14
    box(z, "lab_floor", x0, x1, U2 - 1, U2, z0, z1)
    box(z, "bunker", x0 - 0.2, x1 + 0.2, CEIL - 0.4, CEIL, z0 - 0.2, z1 + 0.2)
    h = CEIL - U2
    wall(z, "lab", "x", z0, x0 - 0.2, x1 + 0.2, U2, h, [door(-32, bottom=U - U2)], name="MedN")
    wall(z, "lab", "x", z1, x0 - 0.2, x1 + 0.2, U2, h, name="MedS")
    wall(z, "lab", "z", x0, z0, z1, U2, h, name="MedW")
    wall(z, "lab", "z", x1, z0, z1, U2, h, [door(-18)], name="MedE")  # -> Particle Accelerator
    box(z, "lab", x0 + 0.2, x1 - 0.2, U2, U, z0 + 0.2, -37)  # north walkway
    box(z, "lab", x0 + 0.2, -43, U2, U, -37, -27)  # west walkway
    box(z, "lab", -21, x1 - 0.2, U2, U, -37, -30)  # east walkway
    stairs(z, "lab", "z", -18.0, -1, -45.8, -43, U2, 0.2, 0.45, 20)
    stairs(z, "lab", "z", -21.0, -1, -21, -18.2, U2, 0.2, 0.45, 20)
    railing(z, -43, -21, -37.2, -37, U)
    railing(z, -43.2, -43, -37, -27, U)
    railing(z, -21.2, -21, -37, -30, U)
    cylinder(z, "metal", -32, -26, 2.4, U2, U2 + 2.5, seg=18)
    cylinder(z, "aether", -32, -26, 1.4, U2 + 2.5, U2 + 4.0, seg=14)
    box(z, "metal", -40, -36, U2, U2 + 0.9, -15.4, -14.4)  # operating tables
    box(z, "metal", -27, -24, U2, U2 + 0.9, -21, -19)

    # Medical Bay -> Particle Accelerator (lower floors).
    z = "Corridors"
    box(z, "lab_floor", -18, -10, U2 - 1, U2, -21, -15)
    box(z, "bunker", -18, -10, -6.6, -6.2, -21.2, -14.8)
    wall(z, "bunker", "x", -21, -18, -10, U2, 3.8, name="CorrMPN")
    wall(z, "bunker", "x", -15, -18, -10, U2, 3.8, name="CorrMPS")

    # Weapons Lab: a walkway ring (y -6) round a pit down to y -10, the bomb
    # hanging over it, stairs down on two sides.
    z = "Weapons"
    x0, x1, z0, z1 = -12, 16, -72, -44
    box(z, "lab_floor", x0, x1, U2 - 1, U2, z0, z1)
    box(z, "bunker", x0 - 0.2, x1 + 0.2, CEIL - 0.4, CEIL, z0 - 0.2, z1 + 0.2)
    h = CEIL - U2
    wall(z, "bunker", "z", x0, z0, z1, U2, h, [door(-60, bottom=U - U2)], name="WeapW")
    wall(z, "bunker", "z", x1, z0, z1, U2, h, name="WeapE")
    wall(z, "bunker", "x", z0, x0 - 0.2, x1 + 0.2, U2, h, name="WeapN")
    wall(z, "bunker", "x", z1, x0 - 0.2, x1 + 0.2, U2, h, [door(8, bottom=U - U2)], name="WeapS")  # -> PA
    box(z, "metal", x0 + 0.2, x1 - 0.2, U2, U, z0 + 0.2, -68)
    box(z, "metal", x0 + 0.2, x1 - 0.2, U2, U, -48, z1 - 0.2)
    box(z, "metal", x0 + 0.2, -8, U2, U, -68, -48)
    box(z, "metal", 12, x1 - 0.2, U2, U, -68, -48)
    stairs(z, "metal", "x", 1.0, -1, -66, -63, U2, 0.2, 0.45, 20)
    stairs(z, "metal", "x", 3.0, 1, -53, -50, U2, 0.2, 0.45, 20)
    railing(z, -8, 12, -68.2, -68, U)
    railing(z, -8, 12, -48, -47.8, U)
    cylinder(z, "olive", 2, -58, 1.3, -7.5, -4.0, seg=16)  # the bomb
    for cx in (1.2, 2.8):
        cylinder(z, "metal", cx, -58, 0.06, -4.0, CEIL, seg=6)  # its chains
    box(z, "metal", -6, -2, U2, U2 + 1.0, -56, -54)  # benches in the pit
    box(z, "metal", 6, 10, U2, U2 + 1.0, -62, -60)

    # Weapons Lab -> Particle Accelerator (upper floors).
    z = "Corridors"
    box(z, "lab_floor", 6.5, 9.5, U - 1, U, -44, -30)
    box(z, "bunker", 6.3, 9.7, CEIL - 0.4, CEIL, -44, -30)
    wall(z, "bunker", "z", 6.5, -44, -30, U, CEIL - U, name="CorrWPW")
    wall(z, "bunker", "z", 9.5, -44, -30, U, CEIL - U, name="CorrWPE")

    particle_accelerator()


def particle_accelerator():
    z = "Accelerator"
    x0, x1, z0, z1 = -10, 36, -30, 16
    box(z, "lab_floor", x0, x1, U2 - 1, U2, z0, z1)
    shaft = (16, 29.9, 11.8, 15.2)
    slab(z, "bunker", x0 - 0.2, x1 + 0.2, z0 - 0.2, z1 + 0.2, CEIL - 0.4, CEIL, [shaft])
    h = CEIL - U2
    wall(z, "bunker", "z", x0, z0, z1, U2, h, [door(-18)], name="AccW")  # <- Medical Bay
    wall(z, "bunker", "z", x1, z0, z1, U2, h, name="AccE")
    wall(z, "bunker", "x", z0, x0 - 0.2, x1 + 0.2, U2, h, [door(8, bottom=U - U2)], name="AccN")  # <- Weapons Lab
    wall(z, "bunker", "x", z1, x0 - 0.2, x1 + 0.2, U2, h, name="AccS")
    # The upper ring walkway (solid down to the lower floor); the west side
    # is cut where the Medical Bay corridor comes in on the lower floor.
    box(z, "metal", x0 + 0.2, x1 - 0.2, U2, U, z0 + 0.2, -25)
    box(z, "metal", x0 + 0.2, x1 - 0.2, U2, U, 11, z1 - 0.2)
    box(z, "metal", x0 + 0.2, -5, U2, U, -25, -21)
    box(z, "metal", x0 + 0.2, -5, U2, U, -15, 11)
    box(z, "metal", 29, x1 - 0.2, U2, U, -25, 11)
    # Stairs down into the bowl on all four sides.
    stairs(z, "metal", "z", -16, -1, 2, 5, U2, 0.2, 0.45, 20)
    stairs(z, "metal", "z", 2, 1, 2, 5, U2, 0.2, 0.45, 20)
    stairs(z, "metal", "x", 4, -1, -2, 1, U2, 0.2, 0.45, 20)
    stairs(z, "metal", "x", 20, 1, -12, -9, U2, 0.2, 0.45, 20)
    # Pack-a-Punch's platform in the middle, the accelerator's ring of
    # magnets round it.
    for i, inset in enumerate((0.0, 0.6, 1.2)):
        box(z, "metal", 8 + inset, 16 - inset, U2, U2 + 0.2 * (i + 1), -11 + inset, -3 - inset)
    for i in range(10):
        a = 2 * math.pi * i / 10
        cylinder(z, "aether" if i % 2 else "metal", 12 + 9 * math.cos(a), -7 + 9 * math.sin(a), 0.6, U2, CEIL, seg=10)
    # The power room alcove: the switch's housing on the north wall.
    box(z, "metal", -2, 2, U, U + 2.4, -29.8, -29)
    # Stairs up to the Yard (its power door), in a shaft on the south ring.
    stairs(z, "concrete", "x", 29.9, -1, 12, 15, U, 0.2, (29.9 - 16.2) / 30, 30)
    box(z, "bunker", 16.2, 29.9, U, 3.0, 11.6, 11.8)
    box(z, "bunker", 16.2, 29.9, U, 3.0, 15.2, 15.4)
    box(z, "bunker", 16.2, 30.1, 3.0, 3.3, 11.6, 15.4)
    box(z, "bunker", 29.9, 30.1, CEIL, 3.0, 11.6, 15.4)
    crate(z, -7, -27.4, U)
    crate(z, 33, -22, U)


def build():
    yard()
    nacht()
    crash_site()
    pond()
    tunnel_entrance()
    labs()


# --- Blender -----------------------------------------------------------------------


def srgb_to_linear(rgb):
    return tuple(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in rgb)


def make_materials():
    mats = {}
    for name, (rgb, rough, metal) in MATERIALS.items():
        m = bpy.data.materials.new(name)
        m.use_nodes = True
        bsdf = m.node_tree.nodes["Principled BSDF"]
        bsdf.inputs["Base Color"].default_value = (*srgb_to_linear(rgb), 1.0)
        bsdf.inputs["Roughness"].default_value = rough
        bsdf.inputs["Metallic"].default_value = metal
        mats[name] = m
    return mats


def make_objects(mats):
    for (zone, mat), (verts, faces) in sorted(PARTS.items()):
        mesh = bpy.data.meshes.new(f"{zone}_{mat}")
        mesh.from_pydata(verts, [], faces)
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(mesh)
        bm.free()
        mesh.materials.append(mats[mat])
        obj = bpy.data.objects.new(f"{zone.capitalize()}_{mat}", mesh)
        bpy.context.collection.objects.link(obj)
    for name, (x, y, z) in MARKERS:
        empty = bpy.data.objects.new(name, None)
        empty.empty_display_size = 0.5
        empty.location = g2b(x, y, z)
        bpy.context.collection.objects.link(empty)


def render_previews(folder):
    os.makedirs(folder, exist_ok=True)
    scene = bpy.context.scene
    engines = [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items]
    scene.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in engines else "BLENDER_EEVEE"
    scene.render.resolution_x, scene.render.resolution_y = 1400, 1100
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.5, 0.55, 0.6, 1)
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.8
    scene.world = world
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 3.0
    sun.rotation_euler = (math.radians(30), math.radians(15), math.radians(30))
    scene.collection.objects.link(sun)
    cam_data = bpy.data.cameras.new("cam")
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam

    def shoot(name, hide=()):
        for obj in bpy.data.objects:
            if obj.type == "MESH":
                obj.hide_render = any(obj.name.startswith(h) or obj.name.endswith(h) for h in hide)
        scene.render.filepath = os.path.join(folder, f"{name}.png")
        bpy.ops.render.render(write_still=True)

    cam_data.type = "ORTHO"
    cam_data.ortho_scale = 110
    cam.location = g2b(-34, 150, -17)
    cam.rotation_euler = (0, 0, 0)
    shoot("top")
    shoot("top_no_roofs", hide=("_roof",))
    # Underground: everything at the surface hidden.
    surface = ("Yard_", "Nacht_", "Crash_", "Pond_", "Tunnel_")
    shoot("underground", hide=surface + ("_bunker",))
    cam_data.type = "PERSP"
    cam_data.lens = 22
    for name, loc, look in (
        ("angle_se", (45, 60, 55), (-30, 0, -20)),
        ("angle_nw", (-110, 55, -95), (-25, 0, -15)),
    ):
        cam.location = g2b(*loc)
        cam.rotation_euler = (Vector(g2b(*look)) - cam.location).to_track_quat("-Z", "Y").to_euler()
        shoot(name)


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    bpy.ops.wm.read_factory_settings(use_empty=True)
    build()
    make_objects(make_materials())
    bpy.ops.export_scene.gltf(filepath=OUT, export_format="GLB", export_yup=True, export_apply=True)
    print(f"wrote {OUT}: {sum(len(f) for _, f in PARTS.values())} faces, {len(MARKERS)} markers")
    if "--previews" in args:
        render_previews(args[args.index("--previews") + 1])


main()
