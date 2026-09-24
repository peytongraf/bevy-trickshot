//! `Zombies` perks: machines on the map a player walks up to and buys a perk
//! from with their points (Call of Duty zombies' perk-a-colas). The server
//! owns purchases (`server::zombies::on_buy_perk`); each member's owned perks
//! are replicated in `LobbyMember::perks`.

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

use crate::MapId;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Perk {
    /// Shroom Tea — for now just the shroom screen effect; more to come.
    ShroomTea,
}

impl Perk {
    pub fn label(self) -> &'static str {
        match self {
            Perk::ShroomTea => "Shroom Tea",
        }
    }

    /// Points it costs (low for now, for testing).
    pub fn cost(self) -> u32 {
        match self {
            Perk::ShroomTea => 100,
        }
    }

    /// Where the perk's machine stands on `map` — the ground under it (feet
    /// level, like `Bot::pos`). Maps without a spot yet use the origin.
    pub fn machine_pos(self, map: MapId) -> Vec3 {
        match (self, map) {
            // Logged with the client's `P` key standing at the spot, which
            // reports eye height (6.5); the ground is 1.7 m below that.
            (Perk::ShroomTea, MapId::BreakPoint) => Vec3::new(-2.66, 6.5 - 1.7, -59.73),
            (Perk::ShroomTea, _) => Vec3::ZERO,
        }
    }
}

/// How close (m, horizontally) a player's feet must be to a machine to buy
/// from it...
pub const PERK_USE_RADIUS: f32 = 2.0;
/// ...and within this much height of its base (not a floor above / below).
pub const PERK_USE_HEIGHT: f32 = 1.5;

/// Whether feet position `feet` is close enough to `perk`'s machine on `map`
/// to buy it. `slack` widens the radius (the server's check allows for the
/// client's pose being a moment old).
pub fn in_range(perk: Perk, map: MapId, feet: Vec3, slack: f32) -> bool {
    let m = perk.machine_pos(map);
    Vec3::new(feet.x - m.x, 0.0, feet.z - m.z).length() <= PERK_USE_RADIUS + slack
        && (feet.y - m.y).abs() <= PERK_USE_HEIGHT + slack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_at_the_logged_spot_is_in_range_and_a_few_metres_away_is_not() {
        let m = Perk::ShroomTea.machine_pos(MapId::BreakPoint);
        assert!(in_range(Perk::ShroomTea, MapId::BreakPoint, m, 0.0));
        assert!(in_range(Perk::ShroomTea, MapId::BreakPoint, m + Vec3::X * 1.5, 0.0));
        assert!(!in_range(Perk::ShroomTea, MapId::BreakPoint, m + Vec3::X * 3.0, 0.0));
        // A floor below doesn't count.
        assert!(!in_range(Perk::ShroomTea, MapId::BreakPoint, m - Vec3::Y * 4.0, 0.0));
    }
}
