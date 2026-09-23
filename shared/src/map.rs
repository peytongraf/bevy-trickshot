//! What the client and the server both need to know about a map: where its
//! model sits in the world ([`placement`]), the hand-measured wall boxes used
//! for spawn placement, and the [`CollisionWorld`] trait the server's
//! collision meshes implement (see `server::collision`, which loads the very
//! same `.glb` files the client builds its own colliders from).
//!
//! The throwing knife ([`crate::throwing_knife`]) bounces off the mesh through
//! [`CollisionWorld::sweep_sphere`], and a bullet is stopped by the first
//! surface it meets ([`CollisionWorld::raycast`]) — it doesn't pass through
//! walls (no wallbangs yet).

use bevy::math::Vec3;

use crate::protocol::MapId;

/// Where a map's collision model sits in the world: a uniform `scale`, then a
/// yaw about +Y, then a `position` — applied to the whole `.glb` scene, the
/// same way the client's `MapModel` entity carries it as a `Transform`.
/// **The client's colliders and the server's are both built from the model
/// with exactly this placement**, so it lives here, shared, rather than in
/// either crate. (The client's debug panel can still nudge it live for
/// tuning — when you land on new numbers, update them here so the server
/// agrees.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapPlacement {
    pub position: Vec3,
    pub yaw_deg: f32,
    pub scale: f32,
}

/// `basic_map.glb`'s placement — see [`MapPlacement`].
pub const BASIC_MAP_PLACEMENT: MapPlacement = MapPlacement {
    position: Vec3::new(16.0, 0.0, 8.0),
    yaw_deg: 0.0,
    scale: 0.65,
};

/// `break_point_map.glb`'s placement: at the origin, at 0.6 scale — natively
/// its ground plane spans x ±60, z ±100 (ringed by boundary walls); at this
/// scale that's 72 × 120 m.
pub const BREAK_POINT_PLACEMENT: MapPlacement = MapPlacement {
    position: Vec3::ZERO,
    yaw_deg: 0.0,
    scale: 0.6,
};

/// The collision model file for `map`, relative to the client's assets
/// directory — `shipment.glb` for both Shipment variants. The server embeds
/// these same files at build time (`server::collision`).
pub fn collision_model_path(map: MapId) -> &'static str {
    match map {
        MapId::BasicMap => "models/basic_map.glb",
        MapId::Shipment | MapId::ShipmentDay => "models/shipment.glb",
        MapId::BreakPoint => "models/break_point_map.glb",
    }
}

/// Where `map`'s collision model sits — see [`MapPlacement`]. Shipment's is
/// just [`SHIPMENT_SCALE`] at the origin.
pub fn placement(map: MapId) -> MapPlacement {
    match map {
        MapId::BasicMap => BASIC_MAP_PLACEMENT,
        MapId::BreakPoint => BREAK_POINT_PLACEMENT,
        MapId::Shipment | MapId::ShipmentDay => MapPlacement {
            position: Vec3::ZERO,
            yaw_deg: 0.0,
            scale: SHIPMENT_SCALE,
        },
    }
}

/// An axis-aligned collision box in world-space X/Z — the Y axis is ignored
/// since every wall in this game is taller than a player, so it blocks at
/// every height a player could reach, not just at foot level. Used to keep
/// [`crate::spawns::spawn_point`] / [`crate::bots::respawn_pose`] from
/// placing someone inside `shipment.glb`'s shipping containers/crates — the
/// client's own movement collision collides against the model's mesh
/// directly instead (see [`SHIPMENT_WALLS`]'s doc comment).
#[derive(Clone, Copy)]
pub struct WallBox {
    pub x: (f32, f32),
    pub z: (f32, f32),
}

/// `basic_map.glb` has no walls to block — its walkable surface (like every
/// map's) instead comes from a `Collider` generated straight from the
/// model's own mesh data; see `sync_map_model` in the client.
const BASIC_MAP_WALLS: &[WallBox] = &[];

/// Hand-measured from `shipment.glb`'s own node transforms — translation and
/// rotated XZ half-extents, an axis-aligned box around each node's actual
/// footprint, exact for the yaw-only 0°/90° containers and a conservative
/// over-approximation for the crates at arbitrary yaw: the eight shipping
/// containers (four near the edges, four interior), the eight small crates
/// scattered around the yard, and the four thin walls ringing the whole
/// yard. These are the model's **native** measurements — i.e. at
/// [`WallBox`]'s implicit scale of `1.0` — so every caller must run them
/// through [`walls`]'s `scale` first; nothing here is pre-scaled.
///
/// Only used server-side, to keep `crate::spawns::spawn_point` /
/// `crate::bots::respawn_pose` from placing someone inside one of these — the
/// client's own movement collision (walking into a container, walking up a
/// ramp, ...) no longer uses this at all: it collides against a `Collider`
/// generated straight from `shipment.glb`'s mesh data (see `sync_map_model`
/// in the client), so it doesn't need this list kept in
/// sync with the model by hand. This one still does, because the server runs
/// headless and never loads the `.glb` itself.
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
    // Crates.
    WallBox { x: (48.7, 53.7), z: (-18.2, -12.2) },
    WallBox { x: (48.7, 53.7), z: (18.5, 29.1) },
    WallBox { x: (29.4, 45.8), z: (34.8, 47.4) },
    WallBox { x: (-39.2, -30.0), z: (21.9, 29.8) },
    WallBox { x: (-36.3, -30.9), z: (36.3, 41.4) },
    WallBox { x: (-41.7, -28.6), z: (-41.2, -25.0) },
    WallBox { x: (20.6, 30.1), z: (-46.0, -38.9) },
    WallBox { x: (35.5, 42.5), z: (-43.5, -34.0) },
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
        MapId::Shipment | MapId::ShipmentDay => SHIPMENT_WALLS,
        MapId::BreakPoint => BREAK_POINT_WALLS,
    }
}

/// `break_point_map.glb`'s solid blocks and walls that reach the ground, in
/// native units (generated from the model's node transforms: every box whose
/// underside is within 2 m of the floor; raised slabs and the outer boundary
/// walls — covered by [`bounds`] — are left out). Spawns and bot placement
/// keep clear of these.
const BREAK_POINT_WALLS: &[WallBox] = &[
    WallBox { x: (19.80, 20.20), z: (-40.00, -2.00) },
    WallBox { x: (-20.20, -19.80), z: (-40.00, 40.00) },
    WallBox { x: (-26.20, -20.20), z: (-40.00, -38.00) },
    WallBox { x: (19.80, 20.20), z: (-100.00, -50.00) },
    WallBox { x: (34.80, 35.20), z: (-40.00, 40.00) },
    WallBox { x: (19.80, 20.20), z: (40.00, 100.00) },
    WallBox { x: (-19.85, -3.85), z: (-38.00, -22.00) },
    WallBox { x: (3.85, 19.85), z: (3.00, 19.00) },
    WallBox { x: (15.85, 19.85), z: (19.00, 23.00) },
    WallBox { x: (-40.85, -3.85), z: (-80.20, -79.80) },
    WallBox { x: (-59.80, -43.80), z: (-80.20, -79.80) },
    WallBox { x: (-3.85, -3.45), z: (-100.00, -85.50) },
    WallBox { x: (-36.20, -20.20), z: (31.00, 35.00) },
    WallBox { x: (-20.20, -19.80), z: (60.00, 100.00) },
    WallBox { x: (-60.00, -54.00), z: (-4.00, 4.00) },
    WallBox { x: (-60.00, -48.00), z: (-10.00, -4.00) },
    WallBox { x: (19.80, 20.20), z: (3.00, 40.00) },
    WallBox { x: (5.75, 19.75), z: (74.80, 75.20) },
    WallBox { x: (38.00, 54.00), z: (94.00, 100.00) },
    WallBox { x: (35.00, 37.00), z: (60.00, 62.00) },
    WallBox { x: (35.00, 37.00), z: (92.00, 94.00) },
    WallBox { x: (58.00, 60.00), z: (60.00, 62.00) },
    WallBox { x: (35.00, 37.00), z: (76.00, 78.00) },
    WallBox { x: (33.00, 37.00), z: (45.00, 55.00) },
];

/// How much smaller than its native layout the area spawns and bots are spread
/// over is for `map` — spawn rings and bot distance tiers are in metres, sized
/// for maps of 100 m and up, so a compact map shrinks them to fit.
pub fn area_scale(map: MapId) -> f32 {
    match map {
        MapId::BreakPoint => BREAK_POINT_PLACEMENT.scale,
        _ => 1.0,
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
        MapId::Shipment | MapId::ShipmentDay => Some(WallBox { x: (-52.0, 55.0), z: (-55.0, 58.0) }),
        // The ground plane's x ±60, z ±100, with a margin off the boundary walls.
        MapId::BreakPoint => Some(WallBox { x: (-58.0, 58.0), z: (-98.0, 98.0) }),
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

/// Where a swept sphere first touched solid geometry — see
/// [`CollisionWorld::sweep_sphere`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHit {
    /// How far along `from → to` (`0.0..=1.0`) the sphere's centre was when it
    /// first touched. `0.0` if it started out already touching / overlapping.
    pub fraction: f32,
    /// Unit surface normal at the contact, pointing out of the solid toward
    /// the sphere (i.e. against the direction of travel).
    pub normal: Vec3,
}

/// Where a ray first met solid geometry — see [`CollisionWorld::raycast`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// Distance from the ray's origin to the surface.
    pub distance: f32,
    /// Unit surface normal at the hit, facing back toward the ray's origin.
    pub normal: Vec3,
}

/// Static-geometry queries against a map's collision mesh.
pub trait CollisionWorld: Send + Sync + 'static {
    /// `true` if a straight segment from `a` to `b` is stopped by solid map
    /// geometry (so a bullet along it should not reach `b`).
    fn segment_blocked(&self, a: Vec3, b: Vec3) -> bool;

    /// The first solid surface along the ray `origin + t * dir` (`dir` unit
    /// length) within `max_dist`, or `None` if the ray is clear.
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit>;

    /// Sweep a sphere of `radius` from `from` to `to` and report the first
    /// solid surface it touches, if any.
    fn sweep_sphere(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit>;
}

/// Stand-in until a map is loaded: nothing blocks anything.
#[derive(Default, Clone, Copy, Debug)]
pub struct EmptyWorld;

impl CollisionWorld for EmptyWorld {
    fn segment_blocked(&self, _a: Vec3, _b: Vec3) -> bool {
        false
    }

    fn raycast(&self, _origin: Vec3, _dir: Vec3, _max_dist: f32) -> Option<RayHit> {
        None
    }

    fn sweep_sphere(&self, _from: Vec3, _to: Vec3, _radius: f32) -> Option<WorldHit> {
        None
    }
}

/// Pathfinding for future AI bots: return waypoints from `start` to `goal`, or
/// `None` if unreachable. Back it with a navmesh (e.g. `oxidized_navigation`) or
/// your own grid/graph once the map exists.
pub trait NavProvider: Send + Sync + 'static {
    fn find_path(&self, start: Vec3, goal: Vec3) -> Option<Vec<Vec3>>;
}
