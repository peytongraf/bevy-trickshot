//! `Zombies` perks: machines on the map a player walks up to and buys a perk
//! from with their points (Call of Duty zombies' perk-a-colas). The server
//! owns purchases (`server::zombies::on_buy_perk`); each member's owned perks
//! are replicated in `LobbyMember::perks`.

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

use crate::MapId;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Perk {
    /// Shroom Tea — the shroom screen effect, x-ray and a little aim assist.
    ShroomTea,
    /// Nitro Brew — faster movement, ADS, reload, rechamber and weapon swap
    /// (the multipliers are client-side: `zombies_hud::NitroBrew`).
    NitroBrew,
    /// Liquid Courage — takes less damage from everything
    /// ([`LIQUID_COURAGE_DAMAGE_MULT`]), with a drunk screen effect.
    LiquidCourage,
}

/// Damage a Liquid Courage owner takes, as a fraction of the normal amount
/// (0.6 ≈ two thirds more health).
pub const LIQUID_COURAGE_DAMAGE_MULT: f32 = 0.6;

/// `damage` scaled by what `perks` protect against.
pub fn damage_taken(perks: &[Perk], damage: f32) -> f32 {
    if perks.contains(&Perk::LiquidCourage) {
        damage * LIQUID_COURAGE_DAMAGE_MULT
    } else {
        damage
    }
}

impl Perk {
    /// Every perk, in the order their icons sit in the HUD.
    pub const ALL: [Perk; 3] = [Perk::ShroomTea, Perk::NitroBrew, Perk::LiquidCourage];

    pub fn label(self) -> &'static str {
        match self {
            Perk::ShroomTea => "Shroom Tea",
            Perk::NitroBrew => "Nitro Brew",
            Perk::LiquidCourage => "Liquid Courage",
        }
    }

    /// One-line blurb for the machine's card (CoD style).
    pub fn description(self) -> &'static str {
        match self {
            Perk::ShroomTea => "See enemies through walls and gain aim assist.",
            Perk::NitroBrew => "Move, aim, reload, rechamber and swap weapons faster.",
            Perk::LiquidCourage => "Take less damage from everything.",
        }
    }

    /// What's in it, for the machine's card (`client/notes/perk-ingredients.md`).
    pub fn ingredients(self) -> &'static [&'static str] {
        match self {
            Perk::ShroomTea => &[
                "Heroic dose of psilocybin mushrooms",
                "Fresh-squeezed lemon juice",
                "Bioluminescent foxfire fungus",
                "Lion's mane extract",
                "Cordyceps, harvested off a zombie",
                "Carrot concentrate and bilberry",
                "Owl eyeball garnish",
                "Ground-up x-ray film",
                "Third eye drops",
            ],
            Perk::NitroBrew => &[
                "Caffeine anhydrous",
                "Distilled rocket fuel",
                "Methamphetamine",
                "Dash of preworkout",
                "Crushed adderall",
                "Splash of nitroglycerin",
                "Twelve energy drinks, boiled down",
                "Freeze-dried hummingbird heartbeats",
                "Squirt of WD-40",
                "Cheetah sweat",
            ],
            Perk::LiquidCourage => &[
                "A whole handle of cheap vodka",
                "Smelling salts, to stay upright",
                "Crushed ibuprofen",
                "Bull's blood, freshly squeezed",
                "Rhino hide shavings",
                "Grandpa's war stories",
                "A splash of cough syrup",
                "Maraschino cherry juice",
                "Pickle brine for the morning after",
            ],
        }
    }

    /// Points it costs (low for now, for testing).
    pub fn cost(self) -> u32 {
        match self {
            Perk::ShroomTea => 100,
            Perk::NitroBrew => 100,
            Perk::LiquidCourage => 100,
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
            (Perk::NitroBrew, MapId::BreakPoint) => Vec3::new(-35.72, 7.7 - 1.7, 10.24),
            // Clear of Shroom Tea's origin spot.
            (Perk::NitroBrew, _) => Vec3::new(6.0, 0.0, 0.0),
            (Perk::LiquidCourage, MapId::BreakPoint) => Vec3::new(35.72, 7.7 - 1.7, 59.72),
            (Perk::LiquidCourage, _) => Vec3::new(-6.0, 0.0, 0.0),
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

    #[test]
    fn only_liquid_courage_cuts_damage() {
        assert_eq!(damage_taken(&[], 50.0), 50.0);
        assert_eq!(damage_taken(&[Perk::ShroomTea, Perk::NitroBrew], 50.0), 50.0);
        assert_eq!(damage_taken(&[Perk::LiquidCourage], 50.0), 50.0 * LIQUID_COURAGE_DAMAGE_MULT);
    }

    #[test]
    fn no_two_machines_can_be_bought_from_the_same_spot() {
        for map in [MapId::BasicMap, MapId::Shipment, MapId::ShipmentDay, MapId::BreakPoint] {
            for a in Perk::ALL {
                for b in Perk::ALL {
                    if a != b {
                        assert!(!in_range(b, map, a.machine_pos(map), 0.75), "{a:?}/{b:?} on {map:?}");
                    }
                }
            }
        }
    }
}
