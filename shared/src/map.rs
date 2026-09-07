//! The small surface the server needs from your map. You own the geometry
//! (meshes, BVH, nav data); the server only calls through these traits.
//!
//! Nothing here is wired into the server loop yet — [`crate::ballistics::resolve_shot`]
//! currently takes a `|from, to| false` closure. Swap that for a
//! [`CollisionWorld::segment_blocked`] call once you have real geometry.

use bevy::math::Vec3;

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
