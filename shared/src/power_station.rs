//! The power station (`models/props/power_station.glb`) — a big piece of
//! `Zombies` scenery placed in the level editor
//! ([`crate::level::ZombiesLayout::power_station`]): solid, but it does
//! nothing itself. The power lever (`crate::power`) is placed on its front
//! separately, and works as ever.

use bevy::math::{Quat, Vec3};

use crate::level::Placement;
use crate::MapId;

/// Its solid box (half extents) as made — `power_station.glb` at scale 1
/// is 4.04 m tall, 1.42 m across its own X and 2.76 m along its Z (its
/// pillars), standing on the ground with its middle over the origin.
pub const HALF_EXTENTS: Vec3 = Vec3::new(0.71, 2.02, 1.38);

/// Where `map`'s station stands, if it has one.
pub fn placement(map: MapId) -> Option<Placement> {
    crate::level::layout(map).power_station
}

/// Its solid box on `map`, if it has one: `(centre, rotation, half extents)`.
pub fn solid_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    placement(map).map(|at| {
        let half = HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}
