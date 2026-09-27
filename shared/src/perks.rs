//! `Zombies` perks: machines on the map a player walks up to and buys a perk
//! from with their points (Call of Duty zombies' perk-a-colas). The server
//! owns purchases (`server::zombies::on_buy_perk`); each member's owned perks
//! are replicated in `LobbyMember::perks`.

use bevy::math::{Quat, Vec3};
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
    /// Bomb Shot — a trickshot kill (a 360 no-scope, [`is_trickshot`])
    /// explodes, killing zombies close by and hurting ones a little further
    /// out ([`bomb_shot_damage`]).
    BombShot,
}

/// Damage a Liquid Courage owner takes, as a fraction of the normal amount
/// (0.6 ≈ two thirds more health).
pub const LIQUID_COURAGE_DAMAGE_MULT: f32 = 0.6;

/// Bomb Shot's blast: everything within this many metres of the dead
/// zombie's feet dies — about the size of the explosion's fireball and dust
/// ring (the client's `ExplosionSettings` defaults)...
pub const BOMB_SHOT_KILL_RADIUS: f32 = 4.0;
/// ...and zombies out to this far still get hurt, less the further out.
pub const BOMB_SHOT_DAMAGE_RADIUS: f32 = 8.0;
/// Damage just outside the kill radius, fading linearly to...
pub const BOMB_SHOT_EDGE_DAMAGE_MAX: f32 = 65.0;
/// ...this at the damage radius (nothing beyond it).
pub const BOMB_SHOT_EDGE_DAMAGE_MIN: f32 = 15.0;
/// Enough to kill anything inside the kill radius outright.
pub const BOMB_SHOT_LETHAL_DAMAGE: f32 = 10_000.0;

/// Whether a shot fired with this trick metadata sets off Bomb Shot: a
/// 360 no-scope (a full turn, any amount airborne or not).
pub fn is_trickshot(spin_deg: f32, noscope: bool) -> bool {
    noscope && spin_deg >= 360.0
}

/// Bomb Shot damage to a zombie standing `distance` metres from the blast
/// (feet to feet): lethal inside [`BOMB_SHOT_KILL_RADIUS`], then fading from
/// [`BOMB_SHOT_EDGE_DAMAGE_MAX`] to [`BOMB_SHOT_EDGE_DAMAGE_MIN`] out to
/// [`BOMB_SHOT_DAMAGE_RADIUS`], and nothing past that.
pub fn bomb_shot_damage(distance: f32) -> f32 {
    if distance <= BOMB_SHOT_KILL_RADIUS {
        BOMB_SHOT_LETHAL_DAMAGE
    } else if distance <= BOMB_SHOT_DAMAGE_RADIUS {
        let t = (distance - BOMB_SHOT_KILL_RADIUS) / (BOMB_SHOT_DAMAGE_RADIUS - BOMB_SHOT_KILL_RADIUS);
        BOMB_SHOT_EDGE_DAMAGE_MAX + (BOMB_SHOT_EDGE_DAMAGE_MIN - BOMB_SHOT_EDGE_DAMAGE_MAX) * t
    } else {
        0.0
    }
}

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
    pub const ALL: [Perk; 4] = [Perk::ShroomTea, Perk::NitroBrew, Perk::LiquidCourage, Perk::BombShot];

    pub fn label(self) -> &'static str {
        match self {
            Perk::ShroomTea => "Shroom Tea",
            Perk::NitroBrew => "Nitro Brew",
            Perk::LiquidCourage => "Liquid Courage",
            Perk::BombShot => "Bomb Shot",
        }
    }

    /// One-line blurb for the machine's card (CoD style).
    pub fn description(self) -> &'static str {
        match self {
            Perk::ShroomTea => "See enemies through walls and gain aim assist.",
            Perk::NitroBrew => "Move, aim, reload, rechamber and swap weapons faster.",
            Perk::LiquidCourage => "Take less damage from everything.",
            Perk::BombShot => "360 no-scope kills explode, blowing up nearby zombies.",
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
            Perk::BombShot => &[
                "A fistful of black powder",
                "Orange soda, violently shaken",
                "Ghost pepper extract",
                "Shaved fireworks, the illegal kind",
                "Leftover nitroglycerin from Nitro Brew",
                "A lit fuse, still burning",
                "Dizziness, bottled mid-spin",
                "Tears of a camping sniper",
            ],
        }
    }

    /// Points it costs (low for now, for testing).
    pub fn cost(self) -> u32 {
        match self {
            Perk::ShroomTea => 100,
            Perk::NitroBrew => 100,
            Perk::LiquidCourage => 100,
            Perk::BombShot => 100,
        }
    }

    /// Where the perk's machine stands on `map` — the centre of the ground
    /// under it (feet level, like `Bot::pos`). The buy range, the machine's
    /// collision box, its model, light and jingle all go from here. Maps
    /// without a spot yet use somewhere near the origin.
    pub fn machine_pos(self, map: MapId) -> Vec3 {
        match (self, map) {
            // Tuned in the client's debug panel ("Machine placement").
            (Perk::ShroomTea, MapId::BreakPoint | MapId::BreakPointNight) => Vec3::new(-2.76, 4.8, -57.43),
            (Perk::ShroomTea, _) => Vec3::ZERO,
            (Perk::NitroBrew, MapId::BreakPoint | MapId::BreakPointNight) => Vec3::new(-35.62, 6.0, 13.54),
            // Clear of Shroom Tea's origin spot.
            (Perk::NitroBrew, _) => Vec3::new(6.0, 0.0, 0.0),
            (Perk::LiquidCourage, MapId::BreakPoint | MapId::BreakPointNight) => Vec3::new(28.92, 6.0, 59.62),
            (Perk::LiquidCourage, _) => Vec3::new(-6.0, 0.0, 0.0),
            // Ground floor, under the upper walkway.
            (Perk::BombShot, MapId::BreakPoint | MapId::BreakPointNight) => Vec3::new(-30.6, 0.0, 0.12),
            (Perk::BombShot, _) => Vec3::new(0.0, 0.0, 6.0),
        }
    }

    /// Which way the perk's machine faces on `map`: its turn around the
    /// vertical axis (degrees).
    pub fn machine_yaw_deg(self, map: MapId) -> f32 {
        match (self, map) {
            (Perk::ShroomTea, MapId::BreakPoint | MapId::BreakPointNight) => -90.0,
            (Perk::NitroBrew, MapId::BreakPoint | MapId::BreakPointNight) => 90.0,
            (Perk::LiquidCourage, MapId::BreakPoint | MapId::BreakPointNight) => 180.0,
            _ => 0.0,
        }
    }

    /// The machine's solid box on `map` (only there in `Zombies`):
    /// `(centre, rotation, half extents)`.
    pub fn machine_box(self, map: MapId) -> (Vec3, Quat, Vec3) {
        (
            self.machine_pos(map) + Vec3::Y * MACHINE_HALF_EXTENTS.y,
            Quat::from_rotation_y(self.machine_yaw_deg(map).to_radians()),
            MACHINE_HALF_EXTENTS,
        )
    }
}

/// Every perk machine is the same box (m): half its width, height and depth
/// before it's turned. The models (`client/assets/models/*_perk_machine.glb`)
/// are this box at the client's machine scale.
pub const MACHINE_HALF_EXTENTS: Vec3 = Vec3::new(0.75, 1.2, 0.36);

/// How close (m, horizontally) a player's feet must be to a machine to buy
/// from it...
pub const PERK_USE_RADIUS: f32 = 2.0;
/// ...and within this much height of its base (not a floor above / below).
pub const PERK_USE_HEIGHT: f32 = 1.5;

/// Whether feet position `feet` is close enough to `perk`'s machine on `map`
/// to buy it. `slack` widens the radius (the server's check allows for the
/// client's pose being a moment old).
pub fn in_range(perk: Perk, map: MapId, feet: Vec3, slack: f32) -> bool {
    in_range_of(perk.machine_pos(map), feet, slack)
}

/// [`in_range`] for a machine standing at `machine` (the client's debug
/// panel can move one away from its [`Perk::machine_pos`]).
pub fn in_range_of(machine: Vec3, feet: Vec3, slack: f32) -> bool {
    Vec3::new(feet.x - machine.x, 0.0, feet.z - machine.z).length() <= PERK_USE_RADIUS + slack
        && (feet.y - machine.y).abs() <= PERK_USE_HEIGHT + slack
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
    fn bomb_shot_kills_close_hurts_further_out_and_spares_the_rest() {
        assert!(bomb_shot_damage(0.0) >= crate::health::FULL_HEALTH);
        assert!(bomb_shot_damage(BOMB_SHOT_KILL_RADIUS) >= crate::health::FULL_HEALTH);
        let just_out = bomb_shot_damage(BOMB_SHOT_KILL_RADIUS + 0.1);
        let further = bomb_shot_damage(BOMB_SHOT_DAMAGE_RADIUS - 0.1);
        assert!(just_out < crate::health::FULL_HEALTH && just_out > further && further > 0.0);
        assert_eq!(bomb_shot_damage(BOMB_SHOT_DAMAGE_RADIUS + 0.1), 0.0);
    }

    #[test]
    fn only_a_360_no_scope_is_a_trickshot() {
        assert!(is_trickshot(360.0, true));
        assert!(is_trickshot(720.0, true));
        assert!(!is_trickshot(359.0, true));
        assert!(!is_trickshot(720.0, false));
    }

    #[test]
    fn only_liquid_courage_cuts_damage() {
        assert_eq!(damage_taken(&[], 50.0), 50.0);
        assert_eq!(damage_taken(&[Perk::ShroomTea, Perk::NitroBrew], 50.0), 50.0);
        assert_eq!(damage_taken(&[Perk::LiquidCourage], 50.0), 50.0 * LIQUID_COURAGE_DAMAGE_MULT);
    }

    #[test]
    fn no_two_machines_can_be_bought_from_the_same_spot() {
        for map in [MapId::BasicMap, MapId::Shipment, MapId::ShipmentDay, MapId::BreakPoint, MapId::BreakPointNight] {
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
