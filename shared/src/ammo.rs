//! The `Zombies` ammo crates: buy a full refill of sniper ammo for points at
//! any of them.
//!
//! Where they stand comes from the map's layout ([`crate::level`]). The range
//! check is the client's; the server only checks the game, that the buyer is
//! alive, and the points (`server::zombies`). Their solid boxes (for players
//! and zombies alike) come from here.

use bevy::math::{Quat, Vec3};

use crate::MapId;

/// Points a full sniper ammo refill costs.
pub const AMMO_COST: u32 = 500;

/// Where `map`'s ammo crates stand (each the ground under its middle) —
/// none on a map without any.
pub fn crates(map: MapId) -> &'static [crate::level::Placement] {
    &crate::level::layout(map).ammo_crates
}

/// Half the crate's length, height and depth (m) — `ammo_crate.glb` at scale
/// 1, handles included.
pub const CRATE_HALF_EXTENTS: Vec3 = Vec3::new(0.9, 0.42, 0.43);

/// Each of `map`'s crates' solid box (only there in `Zombies`):
/// `(centre, rotation, half extents)`.
pub fn crate_boxes(map: MapId) -> impl Iterator<Item = (Vec3, Quat, Vec3)> {
    crates(map).iter().map(|at| {
        let half = CRATE_HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}
