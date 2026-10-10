"""Builds `client/assets/models/maps/vesper_station_map.glb` — VESPER STATION,
a Zombies blockout map — from plain boxes, ramps, stairs and cylinders.

Run from the repo root (Blender 4+):

    blender --background --python tools/blender/vesper_station.py

and, to also render overhead / angled previews into a folder:

    blender --background --python tools/blender/vesper_station.py -- --previews /some/dir

Everything below is written in the GAME's frame — metres, x right, y up, z
toward the camera (Bevy / glTF) — and converted to Blender's z-up frame only
when the mesh is built, so the numbers here are the numbers in the game
(`shared/levels/vesper_station.ron`, the level editor, `shared::map`).
The map sits at the origin at scale 1 (`VESPER_STATION_PLACEMENT`).

THE LAYOUT (looking down, north = -z at the top):

            x=-42          x=-14          x=14            x=42
    z=-69  +--------------+--------------+ - - rock - - - +
           |  POWER       |  LAB (PaP)   |                |
           |  (plateau,   |  (plateau,   +----------------+ z=-51
           |   y=3)  [F]  |   y=3)   [E] <-ramp  RAIL     |
    z=-19  +-----[C]------+--------------+       YARD     |
           |  BARRACKS    |  SPAWN       |      (open,    |
           |  (2 floors)  [A]  (start)  [B]   training)   |
    z=19   +--------------+--------------+----------------+

    Doorways (left open — buyable doors go here later):
      A  spawn  -> barracks      B  spawn -> rail yard
      C  barracks yard -> power (top of the outdoor stairs)
      E  rail yard -> lab (top of the ramp)
      F  power <-> lab
    Spawn -> power: A, C (2).  Spawn -> Pack-a-Punch: B, E (2).
    Opening all five makes one big loop: spawn, barracks, power, lab, rail
    yard and back to spawn.

The outer walls have window openings (sill 1 m, 1.6 x 1.4 m) backed by a
4 m "zombie closet" strip and a tall outer wall — where zombies will climb in
from once barriers exist. Windows between zones are just for looking through.
Every doorway and window also gets an empty named `Door_*` / `Window_*` at
its middle (no mesh, so it doesn't collide), to find them by later.
"""

import math
import os
import sys
from collections import defaultdict

import bmesh
import bpy
from mathutils import Vector

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(REPO, "client", "assets", "models", "maps", "vesper_station_map.glb")

T = 0.4  # wall thickness
DOOR_W, DOOR_H = 2.6, 3.0
WIN_W, WIN_SILL, WIN_TOP = 1.6, 1.0, 2.4

# --- materials ---------------------------------------------------------------------

MATERIALS = {
    # name: (sRGB colour, roughness, metallic)
    "asphalt": ((0.16, 0.16, 0.17), 0.95, 0.0),
    "paving": ((0.42, 0.40, 0.37), 0.9, 0.0),
    "dirt": ((0.30, 0.25, 0.19), 1.0, 0.0),
    "gravel": ((0.33, 0.31, 0.29), 1.0, 0.0),
    "concrete": ((0.50, 0.50, 0.49), 0.9, 0.0),
    "lab_floor": ((0.58, 0.60, 0.62), 0.6, 0.0),
    "rock": ((0.24, 0.22, 0.21), 1.0, 0.0),
    "brick": ((0.45, 0.20, 0.15), 0.9, 0.0),
    "plaster": ((0.66, 0.62, 0.55), 0.9, 0.0),
    "lab_wall": ((0.72, 0.74, 0.76), 0.7, 0.0),
    "boundary": ((0.20, 0.20, 0.21), 1.0, 0.0),
    "wood": ((0.42, 0.29, 0.17), 0.85, 0.0),
    "roof": ((0.14, 0.13, 0.13), 0.9, 0.0),
    "metal": ((0.35, 0.37, 0.38), 0.5, 0.8),
    "rust": ((0.45, 0.24, 0.12), 0.8, 0.4),
    "train_green": ((0.16, 0.28, 0.20), 0.7, 0.3),
    "crate": ((0.50, 0.38, 0.22), 0.9, 0.0),
    "glass": ((0.35, 0.55, 0.50), 0.2, 0.0),
    "stone": ((0.55, 0.54, 0.52), 0.85, 0.0),
    "lamp": ((1.0, 0.9, 0.7), 0.4, 0.0),
}

# (zone, material) -> [verts, faces]
PARTS = defaultdict(lambda: ([], []))
MARKERS = []  # (name, (x, y, z))


def g2b(x, y, z):
    """Game (x, y-up, z) -> Blender (x, y, z-up). glTF export undoes it."""
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


def ramp_z(zone, mat, x0, x1, za, ya, zb, yb, base):
    """A solid ramp across x0..x1, its top rising from (za, ya) to (zb, yb)."""
    c = [(x0, base, za), (x1, base, za), (x1, base, zb), (x0, base, zb),
         (x0, ya, za), (x1, ya, za), (x1, yb, zb), (x0, yb, zb)]
    solid(zone, mat, c, HEX_FACES)


def stairs(zone, mat, axis, a_start, direction, lo, hi, y_base, rise, run, count):
    """`count` steps along `axis` ('x' or 'z') from `a_start`, climbing as
    they go `direction` (+1 / -1); `lo..hi` is their width on the other axis."""
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
    """A wall along `axis` ('x': runs along x at z = `fixed`; 'z': runs along
    z at x = `fixed`) from a0 to a1, y0 up `height`. `openings` are
    (centre, width, bottom, top, kind) with bottom / top above y0 — they may
    stack (a ground-floor and an upstairs window) or overlap along the wall;
    kind is 'door' or 'window' (for the marker's name)."""
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
    # Cut the wall into columns at every opening's edge; each column is solid
    # but for the openings that span it.
    cuts = sorted({a0, a1, *(h[0] for h in holes), *(h[1] for h in holes)})
    for c0, c1 in zip(cuts, cuts[1:]):
        mid = (c0 + c1) / 2
        y = y0
        for _, _, yb, yt in sorted((h for h in holes if h[0] < mid < h[1]), key=lambda h: h[2]):
            piece(c0, c1, y, yb)
            y = max(y, yt)
        piece(c0, c1, y, y0 + height)


def door(centre, width=DOOR_W, height=DOOR_H):
    return (centre, width, 0.0, height, "door")


def window(centre, sill=WIN_SILL, top=WIN_TOP, width=WIN_W):
    return (centre, width, sill, top, "window")


def crate(zone, x, z, y=0.0, size=1.2):
    h = size / 2
    box(zone, "crate", x - h, x + h, y, y + size, z - h, z + h)


def lamp_post(zone, x, z, y=0.0, height=6.0):
    """Tall enough that its head can't be climbed onto (out of mantle reach
    from the crates and benches)."""
    cylinder(zone, "metal", x, z, 0.12, y, y + height, seg=10)
    box(zone, "lamp", x - 0.3, x + 0.3, y + height, y + height + 0.25, z - 0.3, z + 0.3)


# --- the map -----------------------------------------------------------------------


def build():
    # Ground, by zone (they don't overlap — no z-fighting).
    box("ground", "paving", -14, 14, -1, 0, -19, 19)  # spawn square
    box("ground", "dirt", -46.4, -14, -1, 0, -19, 23.4)  # barracks + its closets
    box("ground", "dirt", -14, 14, -1, 0, 19, 23.4)  # closet behind spawn
    box("ground", "gravel", 14, 46.4, -1, 0, -51, 23.4)  # rail yard + closets
    # The plateau (y = 3): power (west) and the lab (middle).
    box("ground", "concrete", -46.4, -14, -1, 3, -73.4, -19)
    box("ground", "lab_floor", -14, 14, -1, 3, -73.4, -19)
    # Rock filling the north-east corner.
    box("ground", "rock", 14, 46.4, -1, 10, -73.4, -51)

    # Outer boundary (the far side of the zombie closets).
    box("boundary", "boundary", -46.8, 46.8, -1, 14, 23.4, 23.8)
    box("boundary", "boundary", -46.8, 46.8, -1, 14, -73.8, -73.4)
    box("boundary", "boundary", -46.8, -46.4, -1, 14, -73.8, 23.8)
    box("boundary", "boundary", 46.4, 46.8, -1, 14, -73.8, 23.8)

    spawn()
    barracks()
    power()
    lab()
    rail_yard()


def spawn():
    z = "spawn"
    # South edge of the map (shared with the barracks and rail yard behind
    # their own buildings) — windows onto the closet.
    wall(z, "brick", "x", 19, -42.2, 42.2, 0, 10, [
        window(-38), window(-28),  # barracks, ground floor
        window(-38, 5.0, 6.4), window(-28, 5.0, 6.4),  # barracks, upstairs
        window(-11), window(-4), window(4), window(11),  # spawn
        window(20), window(30), window(38),  # rail yard
    ], name="South")
    # The square's sides: doorways A (barracks) and B (rail yard).
    wall(z, "brick", "z", -14, -19, 19, 0, 9, [door(8), window(-8)], name="SpawnWest")
    wall(z, "brick", "z", 14, -19, 19, 0, 9, [door(8), window(-8)], name="SpawnEast")

    # Ticket office along the back: QR + ammo inside, wide doorway in front.
    wall(z, "plaster", "x", 11, -8.2, 8.2, 0, 4.5, [door(0, 3.0), window(-5), window(5)], name="Office")
    wall(z, "plaster", "z", -8, 11.2, 18.8, 0, 4.5, [window(15)], name="OfficeWest")
    wall(z, "plaster", "z", 8, 11.2, 18.8, 0, 4.5, [door(15, 2.2)], name="OfficeEast")
    box(z, "roof", -8.4, 8.4, 4.5, 4.8, 10.6, 19.2)
    box(z, "wood", -6, -2, 0, 1.0, 16.5, 17.5)  # ticket counter

    # Fountain in the middle (clear of the bots' centre at (0, -5)).
    cylinder(z, "stone", 0, 4, 2.6, 0, 0.7, seg=28)
    cylinder(z, "stone", 0, 4, 0.5, 0.7, 2.4, seg=14)
    cylinder(z, "stone", 0, 4, 1.0, 2.1, 2.3, seg=18)

    # Raised station platform down the west side, steps at its south end.
    box(z, "concrete", -13.8, -8, -1, 1.0, -16, -1)
    stairs(z, "concrete", "z", 1.0, -1, -13.8, -8, 0, 0.2, 0.4, 5)
    for bz in (-13, -7):
        box(z, "wood", -12.6, -11.4, 1.0, 1.45, bz - 1.2, bz + 1.2)  # benches

    # Some low cover (nothing tall enough to climb out from).
    crate(z, 7, -12)
    crate(z, 8.3, -11.4)
    crate(z, 7.6, -11.6, 1.2, 1.0)
    crate(z, -4, -15)
    box(z, "wood", 3, 6.5, 0, 1.0, -3.5, -2.5)  # planter
    lamp_post(z, -6, -12)
    lamp_post(z, 6, -6)
    lamp_post(z, 10, 6)


def barracks():
    z = "barracks"
    # West edge of the map, ground level (the barracks' own west wall).
    wall(z, "brick", "z", -42, -19, 19.2, 0, 10, [
        window(-13),  # north yard
        window(-2), window(12),
        window(-2, 5.0, 6.4), window(12, 5.0, 6.4),
    ], name="WestLow")

    # The building: x -42..-24, z -7..19, two floors (upstairs at y 4).
    wall(z, "brick", "z", -24, -7.2, 19, 0, 8, [
        door(-1, 2.4), door(13, 2.4), window(6),
        window(-2, 5.0, 6.4), window(6, 5.0, 6.4), window(14, 5.0, 6.4),
    ], name="BarracksEast")
    wall(z, "brick", "x", -7, -41.8, -23.8, 0, 8, [
        door(-33, 2.4), window(-38), window(-28),
        window(-38, 5.0, 6.4), window(-28, 5.0, 6.4),
    ], name="BarracksNorth")
    # Upstairs floor, with the stairwell open along the south wall.
    box(z, "wood", -41.8, -24.2, 3.7, 4.0, -6.8, 16.4)
    box(z, "wood", -41.8, -38.5, 3.7, 4.0, 16.4, 18.8)
    box(z, "wood", -28.5, -24.2, 3.7, 4.0, 16.4, 18.8)
    stairs(z, "wood", "x", -28.5, -1, 16.6, 18.8, 0, 0.2, 0.5, 20)
    box(z, "wood", -38.5, -28.5, 4.0, 5.0, 16.2, 16.4)  # stairwell railing
    box(z, "wood", -28.7, -28.5, 4.0, 5.0, 16.4, 18.8)
    box(z, "roof", -42.2, -23.8, 8.0, 8.3, -7.2, 19.2)
    # Downstairs tables, upstairs bunks.
    for tz in (-3, 4):
        box(z, "wood", -33, -29, 0, 0.9, tz - 0.6, tz + 0.6)
    for bx in (-40.6, -36.6, -32.6, -28.6):
        box(z, "metal", bx - 0.5, bx + 0.5, 4.0, 5.3, -5.6, -3.6)

    # North yard: the outdoor stairs up to the power plateau (doorway C at
    # the top), and an old truck for cover.
    stairs(z, "concrete", "z", -9.0, -1, -41.8, -38.8, 0, 0.2, 0.6, 15)
    box(z, "concrete", -41.8, -38.8, -1, 3.0, -19, -18)
    box(z, "rust", -31, -26, 0, 2.0, -15, -12.6)  # truck bed
    box(z, "rust", -26, -24.2, 0, 2.4, -15, -12.6)  # truck cab
    crate(z, -20, -15)
    crate(z, -18.8, -15.4)
    lamp_post(z, -18, -3)


def power():
    z = "power"
    y = 3.0
    # West and north edges of the map, up on the plateau.
    wall(z, "brick", "z", -42, -69.2, -19, y, 8, [window(-30), window(-60)], name="WestHigh")
    wall(z, "brick", "x", -69, -42.2, 14, y, 8, [
        window(-35), window(-28),  # generator hall
        window(-8), window(0), window(8),  # lab core
        window(-8, 4.6, 6.0), window(0, 4.6, 6.0), window(8, 4.6, 6.0),  # lab mezzanine
    ], name="North")
    # The plateau's edge over the barracks yard: doorway C.
    wall(z, "brick", "x", -19, -41.8, -14, y, 7, [door(-40.3), window(-32), window(-24)], name="PowerSouth")

    # Generator hall in the north-west corner.
    wall(z, "concrete", "x", -50, -41.8, -21.8, y, 7, [door(-30, 4.0, 3.5), window(-38), window(-25)], name="HallSouth")
    wall(z, "concrete", "z", -22, -68.8, -50, y, 7, [door(-60), window(-54)], name="HallEast")
    box(z, "roof", -42.2, -21.8, y + 7, y + 7.3, -69.2, -49.8)
    box(z, "metal", -36, -30, y, y + 2.5, -64, -60)  # the generator
    cylinder(z, "rust", -40, -64, 0.4, y, y + 6.8, seg=12)  # exhaust stack
    box(z, "metal", -27, -24.5, y, y + 1.8, -66, -63)  # switch gear

    # Yard: transformers and crates, kept off the walls.
    box(z, "metal", -32, -29.5, y, y + 2.0, -40, -37.5)
    box(z, "metal", -26, -23.5, y, y + 2.0, -40, -37.5)
    crate(z, -36, -27, y)
    crate(z, -34.8, -27.6, y)
    crate(z, -20, -33, y)
    lamp_post(z, -28, -30, y)


def lab():
    z = "lab"
    y = 3.0
    # Its walls: west (power, doorway F), east (the ramp up, doorway E) and
    # south (windows down over the spawn square). North is the map's edge.
    wall(z, "lab_wall", "z", -14, -69.2, -18.8, y, 7, [door(-40), window(-28), window(-56)], name="LabWest")
    wall(z, "lab_wall", "z", 14, -69.2, -18.8, y, 7, [door(-28), window(-38)], name="LabEast")
    wall(z, "lab_wall", "x", -19, -13.8, 13.8, y, 7, [window(-9), window(-3), window(3), window(9)], name="LabSouth")
    # Front lab / core divider.
    wall(z, "lab_wall", "x", -45, -13.8, 13.8, y, 7, [door(-7), door(7), window(0, 1.0, 2.6, 3.0)], name="LabCore")
    box(z, "roof", -14.2, 14.2, y + 7, y + 7.3, -69.2, -18.8)

    # Pack-a-Punch dais in the core.
    box(z, "metal", -4, 4, y - 0.5, y + 0.2, -61, -53)
    box(z, "metal", -3, 3, y - 0.5, y + 0.4, -60, -54)
    # Mezzanine along the north wall (Der Wunderfizz), stairs up its east end.
    box(z, "metal", -13.8, 13.8, y + 3.3, y + 3.6, -68.8, -63)
    stairs(z, "metal", "z", -54, -1, 10.8, 13.8, y, 0.2, 0.5, 18)
    box(z, "metal", -13.8, 10.8, y + 3.6, y + 4.6, -63, -62.8)  # railing

    # Front lab: benches and specimen tanks.
    for bx in (-8, 8):
        for bz in (-30, -38):
            box(z, "metal", bx - 1.5, bx + 1.5, y, y + 1.0, bz - 0.6, bz + 0.6)
    for tx in (-11, 11):
        cylinder(z, "glass", tx, -23.5, 1.0, y, y + 3.0, seg=16)
        cylinder(z, "metal", tx, -23.5, 1.1, y + 3.0, y + 3.3, seg=16)


def rail_yard():
    z = "rail"
    # East edge of the map (windows over the loading platform sit higher).
    wall(z, "brick", "z", 42, -51, 19.2, 0, 10, [
        window(-40), window(-30), window(-15, 2.2, 3.6), window(0, 2.2, 3.6), window(15),
    ], name="East")

    # Ramp up to the lab's doorway E, and the landing at the top.
    ramp_z(z, "concrete", 14.0, 18.2, -9, 0.0, -25, 3.0, -0.5)
    box(z, "concrete", 14.0, 18.2, -1, 3.0, -31, -25)

    # Loading platform along the east wall, ramps at both ends.
    box(z, "concrete", 36, 41.8, -1, 1.2, -20, 10)
    ramp_z(z, "concrete", 36, 41.8, 10, 1.2, 15, 0.0, -0.5)
    ramp_z(z, "concrete", 36, 41.8, -20, 1.2, -25, 0.0, -0.5)

    # Track beds and parked cars — loops to run zombies around.
    for tx in (23.5, 30.5):
        box(z, "wood", tx - 1.6, tx + 1.6, 0, 0.05, -50, 18)
    box(z, "train_green", 22, 25, 0, 3.6, -14, 2)
    box(z, "rust", 29, 32, 0, 3.6, -2, 14)
    box(z, "train_green", 22, 25, 0, 3.6, -48, -36)

    # Open-fronted shed against the rock (Speed Cola, armor), roof on posts.
    for px in (26.2, 34, 41.6):
        cylinder(z, "metal", px, -37.8, 0.2, 0, 8, seg=10)
    box(z, "roof", 26, 42, 8, 8.3, -51, -37.6)

    crate(z, 19, -2)
    crate(z, 19.6, -0.8)
    crate(z, 27, 8)
    crate(z, 34, -10)
    crate(z, 34, -8.8, 0, 1.0)
    crate(z, 18, 12)
    # Floodlight mast.
    cylinder(z, "metal", 39, 17, 0.25, 0, 9, seg=12)
    box(z, "lamp", 38.4, 39.6, 9, 9.5, 16.4, 17.6)
    lamp_post(z, 20, -36)


# --- Blender -----------------------------------------------------------------------


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


def srgb_to_linear(rgb):
    return tuple(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in rgb)


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
    scene.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in [
        e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items
    ] else "BLENDER_EEVEE"
    scene.render.resolution_x, scene.render.resolution_y = 1400, 1400
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.5, 0.55, 0.6, 1)
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
    scene.world = world
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 3.0
    sun.rotation_euler = (math.radians(35), math.radians(15), math.radians(30))
    scene.collection.objects.link(sun)

    cam_data = bpy.data.cameras.new("cam")
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    shots = {
        # Straight down, roofs and all.
        "top": dict(ortho=100, loc=g2b(0, 120, -25), rot=(0, 0, 0)),
        # From the south-east, looking north-west.
        "angle_se": dict(lens=24, loc=g2b(70, 55, 60), look=g2b(-5, 0, -25)),
        # From the south-west.
        "angle_sw": dict(lens=24, loc=g2b(-75, 50, 55), look=g2b(0, 0, -25)),
    }
    for name, s in shots.items():
        if "ortho" in s:
            cam_data.type = "ORTHO"
            cam_data.ortho_scale = s["ortho"]
            cam.location = s["loc"]
            cam.rotation_euler = s["rot"]
        else:
            cam_data.type = "PERSP"
            cam_data.lens = s["lens"]
            cam.location = s["loc"]
            direction = Vector(s["look"]) - cam.location
            cam.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(folder, f"{name}.png")
        bpy.ops.render.render(write_still=True)
    # The same straight-down view with the roofs hidden, to see inside.
    for obj in bpy.data.objects:
        if obj.type == "MESH" and obj.name.endswith("_roof"):
            obj.hide_render = True
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = 100
    cam.location = shots["top"]["loc"]
    cam.rotation_euler = (0, 0, 0)
    scene.render.filepath = os.path.join(folder, "top_no_roofs.png")
    bpy.ops.render.render(write_still=True)


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
