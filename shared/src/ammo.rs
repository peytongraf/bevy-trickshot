//! The `Zombies` ammo crate: buy a full refill of sniper ammo for points.
//!
//! Where the crate stands is still being tuned on the client (debug panel),
//! so the range check is the client's for now; the server only checks the
//! game, that the buyer is alive, and the points (`server::zombies`). Its
//! solid box (for players and zombies alike) comes from here.

use bevy::math::{Quat, Vec3};

use crate::MapId;

/// Points a full sniper ammo refill costs.
pub const AMMO_COST: u32 = 500;

/// Where the crate's model origin (the middle of the box) sits on `map`, if
/// the map has one. Only Break Point (day or night) so far.
pub fn crate_pos(map: MapId) -> Option<Vec3> {
    // Tuned in the client's debug panel ("Ammo crate").
    map.is_break_point().then_some(Vec3::new(9.6, 0.4, -35.0))
}

/// Which way the crate faces (degrees about y).
pub const CRATE_YAW_DEG: f32 = -90.0;

/// Half the crate's length, height and depth (m) — `ammo_crate.glb` at scale
/// 1, handles included.
pub const CRATE_HALF_EXTENTS: Vec3 = Vec3::new(0.9, 0.42, 0.43);

/// The crate's solid box on `map`, if it has one (only there in `Zombies`):
/// `(centre, rotation, half extents)`.
pub fn crate_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    crate_pos(map).map(|pos| {
        (
            pos,
            Quat::from_rotation_y(CRATE_YAW_DEG.to_radians()),
            CRATE_HALF_EXTENTS,
        )
    })
}
