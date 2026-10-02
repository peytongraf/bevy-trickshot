//! `Zombies` power: a switch on the map a player pays to throw, which turns
//! on the map's lights for the whole lobby (Call of Duty zombies' power
//! switch). The server owns it (`server::zombies::on_turn_on_power`); whether
//! it's on is replicated in `Lobby::power_on`.

use bevy::math::Vec3;

use crate::perks::{PERK_USE_HEIGHT, PERK_USE_RADIUS};
use crate::MapId;

/// Points it costs to turn the power on — affordable by round 2 or 3 (see
/// `perks::Perk::cost` for the points pace).
pub const POWER_COST: u32 = 750;

/// Where the power switch is on `map` (the ground under its middle — the
/// lever itself is mounted on the wall above, see the client's
/// `power::PowerLeverSettings`), if that map has one. Only Break Point does
/// for now (day or night — the same in every way but the look).
pub fn switch_pos(map: MapId) -> Option<Vec3> {
    if map.is_break_point() {
        Some(Vec3::new(-7.0, 0.0, -56.35))
    } else {
        None
    }
}

/// Whether the power's on in a game on `map` (`Lobby::power_on`) — always,
/// on a map with no switch to throw. Perk machines only sell (and light up)
/// with it on.
pub fn has_power(map: MapId, power_on: bool) -> bool {
    power_on || switch_pos(map).is_none()
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
    fn a_map_without_a_switch_always_has_power() {
        assert!(has_power(MapId::BasicMap, false));
        for map in [MapId::BreakPoint, MapId::BreakPointNight] {
            assert!(!has_power(map, false));
            assert!(has_power(map, true));
        }
    }

    #[test]
    fn only_break_point_has_a_switch_day_or_night_and_its_reach_is_a_couple_of_metres() {
        assert!(switch_pos(MapId::BasicMap).is_none());
        assert!(!in_range(MapId::BasicMap, Vec3::ZERO, 0.0));
        assert_eq!(switch_pos(MapId::BreakPoint), switch_pos(MapId::BreakPointNight));
        let s = switch_pos(MapId::BreakPointNight).unwrap();
        assert!(in_range(MapId::BreakPointNight, s + Vec3::X * 1.5, 0.0));
        assert!(!in_range(MapId::BreakPointNight, s + Vec3::X * 3.0, 0.0));
    }
}
