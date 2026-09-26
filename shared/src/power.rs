//! `Zombies` power: a switch on the map a player pays to throw, which turns
//! on the map's lights for the whole lobby (Call of Duty zombies' power
//! switch). The server owns it (`server::zombies::on_turn_on_power`); whether
//! it's on is replicated in `Lobby::power_on`.

use bevy::math::Vec3;

use crate::perks::{PERK_USE_HEIGHT, PERK_USE_RADIUS};
use crate::MapId;

/// Points it costs to turn the power on.
pub const POWER_COST: u32 = 100;

/// Where the power switch stands on `map` (the ground under its middle), if
/// that map has one. Only Break Point Night does for now.
pub fn switch_pos(map: MapId) -> Option<Vec3> {
    match map {
        MapId::BreakPointNight => Some(Vec3::ZERO),
        _ => None,
    }
}

/// Whether feet position `feet` is close enough to `map`'s power switch to
/// throw it — the same reach as a perk machine. `slack` widens it (the
/// server allows for the client's pose being a moment old).
pub fn in_range(map: MapId, feet: Vec3, slack: f32) -> bool {
    switch_pos(map).is_some_and(|s| {
        Vec3::new(feet.x - s.x, 0.0, feet.z - s.z).length() <= PERK_USE_RADIUS + slack
            && (feet.y - s.y).abs() <= PERK_USE_HEIGHT + slack
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_break_point_night_has_a_switch_and_its_reach_is_a_couple_of_metres() {
        assert!(switch_pos(MapId::BreakPoint).is_none());
        assert!(!in_range(MapId::BreakPoint, Vec3::ZERO, 0.0));
        let s = switch_pos(MapId::BreakPointNight).unwrap();
        assert!(in_range(MapId::BreakPointNight, s + Vec3::X * 1.5, 0.0));
        assert!(!in_range(MapId::BreakPointNight, s + Vec3::X * 3.0, 0.0));
    }
}
