//! The small surface the server needs from your map. You own the geometry
//! (meshes, BVH, nav data); the server only calls through these traits.
//!
//! Nothing here is wired into the server loop yet — [`crate::ballistics::resolve_shot`]
//! currently takes a `|from, to| false` closure. Swap that for a
//! [`CollisionWorld::segment_blocked`] call once you have real geometry.

use bevy::math::Vec3;

use crate::protocol::MapId;

/// An axis-aligned collision box in world-space X/Z — the Y axis is ignored
/// since every wall in this game is taller than a player, so it blocks at
/// every height a player could reach, not just at foot level. Used to keep
/// players out of `shipment.glb`'s shipping containers ([`client`]'s
/// `resolve_wall_collisions`) and to keep [`crate::spawns::spawn_point`] /
/// [`crate::bots::respawn_pose`] from placing someone inside one.
#[derive(Clone, Copy)]
pub struct WallBox {
    pub x: (f32, f32),
    pub z: (f32, f32),
}

/// `basic_map.glb` has no walls to block — only its walkable-surface height
/// field (client-only `MAP_SURFACES`) matters there.
const BASIC_MAP_WALLS: &[WallBox] = &[];

/// Hand-measured from `shipment.glb`'s own node transforms (translation ×
/// scale, swapping the local X/Z half-extents on the nodes with a 90° yaw):
/// the eight shipping containers (four near the edges, four interior) plus
/// the four thin walls now ringing the whole yard. These are the model's
/// **native** measurements — i.e. at [`WallBox`]'s implicit scale of `1.0` —
/// so every caller must run them through [`walls`]'s `scale` first; nothing
/// here is pre-scaled.
const SHIPMENT_WALLS: &[WallBox] = &[
    // Containers.
    WallBox { x: (-52.0, -36.0), z: (-9.0, 12.0) },
    WallBox { x: (34.5, 50.5), z: (-9.0, 12.0) },
    WallBox { x: (-10.5, 10.5), z: (-54.5, -38.5) },
    WallBox { x: (-10.5, 10.5), z: (42.0, 58.0) },
    WallBox { x: (5.5, 21.5), z: (-24.0, -3.0) },
    WallBox { x: (-21.5, -5.5), z: (-24.0, -3.0) },
    WallBox { x: (-20.5, -4.5), z: (6.5, 27.5) },
    WallBox { x: (4.0, 20.0), z: (7.5, 28.5) },
    // Outer boundary walls (+Z, -Z, +X, -X).
    WallBox { x: (-52.0, 55.0), z: (58.0, 60.0) },
    WallBox { x: (-52.0, 55.0), z: (-57.0, -55.0) },
    WallBox { x: (55.0, 57.0), z: (-57.0, 60.0) },
    WallBox { x: (-54.0, -52.0), z: (-57.0, 60.0) },
];

/// `shipment.glb` was authored about twice the size it should be relative to
/// the rest of the game (player height, weapon range, ...), so it's rendered
/// at this uniform scale rather than `1.0` — see `client::ShipmentSettings`,
/// whose debug-panel slider defaults to this and lets you dial in a different
/// number live. The server has no such panel, so its spawn/respawn placement
/// (`crate::spawns::spawn_point`, `crate::bots::respawn_pose`) always uses
/// this constant; if you land on a different number, update it here so the
/// server stays in agreement with what the client renders and collides with.
pub const SHIPMENT_SCALE: f32 = 0.5;

/// Every wall box for `map`, in **native** (unscaled) coordinates — scale
/// each by whatever factor `map` is actually being rendered at (`1.0` for
/// `BasicMap`, [`SHIPMENT_SCALE`] — or a live override of it — for
/// `Shipment`) before comparing against world-space positions; see
/// [`point_blocked`].
pub fn walls(map: MapId) -> &'static [WallBox] {
    match map {
        MapId::BasicMap => BASIC_MAP_WALLS,
        MapId::Shipment => SHIPMENT_WALLS,
    }
}

/// `true` if a circle of `radius` centred at world-space `(x, z)` overlaps
/// any of `map`'s walls once they're scaled by `scale` (see [`walls`]).
pub fn point_blocked(map: MapId, x: f32, z: f32, radius: f32, scale: f32) -> bool {
    walls(map).iter().any(|w| {
        let (x0, x1) = (w.x.0 * scale, w.x.1 * scale);
        let (z0, z1) = (w.z.0 * scale, w.z.1 * scale);
        x > x0 - radius && x < x1 + radius && z > z0 - radius && z < z1 + radius
    })
}

/// `Shipment`'s playable interior, in native (unscaled) coordinates — matches
/// the model's own `Ground` node footprint, which the outer walls in
/// [`SHIPMENT_WALLS`] now fully ring. `None` for `BasicMap`, an open field
/// with no real boundary to enforce.
fn bounds(map: MapId) -> Option<WallBox> {
    match map {
        MapId::BasicMap => None,
        MapId::Shipment => Some(WallBox { x: (-52.0, 55.0), z: (-55.0, 58.0) }),
    }
}

/// `true` if world-space `(x, z)` falls inside `map`'s playable interior (see
/// [`bounds`]) once that's scaled by `scale` — `crate::spawns::spawn_point`
/// and `crate::bots::respawn_pose` both reject candidates outside it, since a
/// point can clear every individual wall in [`point_blocked`] (they're thin)
/// while still landing beyond the map entirely. A map with no defined
/// interior (`BasicMap`) always passes.
pub fn in_bounds(map: MapId, x: f32, z: f32, scale: f32) -> bool {
    match bounds(map) {
        None => true,
        Some(b) => {
            let (x0, x1) = (b.x.0 * scale, b.x.1 * scale);
            let (z0, z1) = (b.z.0 * scale, b.z.1 * scale);
            x > x0 && x < x1 && z > z0 && z < z1
        }
    }
}

/// Static-geometry occlusion queries for shot resolution.
pub trait CollisionWorld: Send + Sync + 'static {
    /// `true` if a straight segment from `a` to `b` is stopped by solid map
    /// geometry (so a bullet along it should not reach `b`).
    fn segment_blocked(&self, a: Vec3, b: Vec3) -> bool;
}

/// Stand-in until a map is loaded: nothing blocks anything.
#[derive(Default, Clone, Copy, Debug)]
pub struct EmptyWorld;

impl CollisionWorld for EmptyWorld {
    fn segment_blocked(&self, _a: Vec3, _b: Vec3) -> bool {
        false
    }
}

/// Pathfinding for future AI bots: return waypoints from `start` to `goal`, or
/// `None` if unreachable. Back it with a navmesh (e.g. `oxidized_navigation`) or
/// your own grid/graph once the map exists.
pub trait NavProvider: Send + Sync + 'static {
    fn find_path(&self, start: Vec3, goal: Vec3) -> Option<Vec<Vec3>>;
}
