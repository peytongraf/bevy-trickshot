//! The `Zombies` ammo crate: buy a full refill of sniper ammo for points.
//!
//! Where it stands comes from the map's layout ([`crate::level`]). The range
//! check is the client's; the server only checks the game, that the buyer is
//! alive, and the points (`server::zombies`). Its solid box (for players and
//! zombies alike) comes from here.

use bevy::math::{Quat, Vec3};

use crate::MapId;

/// Points a full sniper ammo refill costs.
pub const AMMO_COST: u32 = 500;

/// Where the crate stands on `map` (the ground under its middle), if the map
/// has one.
pub fn crate_pos(map: MapId) -> Option<Vec3> {
    crate::level::layout(map).ammo_crate.map(|p| p.pos)
}

/// Which way the crate faces on `map` (degrees about y).
pub fn crate_yaw_deg(map: MapId) -> f32 {
    crate::level::layout(map).ammo_crate.map_or(0.0, |p| p.yaw_deg)
}

/// Half the crate's length, height and depth (m) — `ammo_crate.glb` at scale
/// 1, handles included.
pub const CRATE_HALF_EXTENTS: Vec3 = Vec3::new(0.9, 0.42, 0.43);

/// The crate's solid box on `map`, if it has one (only there in `Zombies`):
/// `(centre, rotation, half extents)`.
pub fn crate_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    crate_pos(map).map(|pos| {
        (
            pos + Vec3::Y * CRATE_HALF_EXTENTS.y,
            Quat::from_rotation_y(crate_yaw_deg(map).to_radians()),
            CRATE_HALF_EXTENTS,
        )
    })
}
