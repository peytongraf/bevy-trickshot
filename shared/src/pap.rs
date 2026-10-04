//! `Zombies` Pack-a-Punch: pay at the machine to upgrade the weapon in your
//! hands, up to [`MAX_LEVEL`]. Every level doubles its damage
//! ([`damage_mult`]) and the sniper carries more ammo (the client's
//! `weapons::weapon`). Each member's levels are replicated as
//! [`crate::LobbyMember::pap`]; the server (`server::zombies`) checks and
//! takes every purchase ([`crate::BuyPap`]).

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

use crate::perks::{PERK_USE_HEIGHT, PERK_USE_RADIUS};
use crate::weapon::{SlotWeapon, WeaponId};
use crate::MapId;

/// Highest level a weapon can be packed to.
pub const MAX_LEVEL: u8 = 3;

/// Which weapon is being packed — whichever the player is holding. A
/// player carries at most one of each ([`crate::LobbyMember::weapons`]), so
/// the level goes with the kind.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PapWeapon {
    Sniper,
    Ak74,
    Knife,
}

impl PapWeapon {
    pub fn label(self) -> &'static str {
        match self {
            PapWeapon::Sniper => "Sniper",
            PapWeapon::Ak74 => "AK-74",
            PapWeapon::Knife => "Knife",
        }
    }

    /// The level `weapon` is packed by.
    pub fn of(weapon: SlotWeapon) -> PapWeapon {
        match weapon {
            SlotWeapon::Gun(gun) => PapWeapon::gun(gun),
            SlotWeapon::Knife => PapWeapon::Knife,
        }
    }

    pub fn gun(gun: WeaponId) -> PapWeapon {
        match gun {
            WeaponId::Ak74 => PapWeapon::Ak74,
            _ => PapWeapon::Sniper,
        }
    }
}

/// A member's Pack-a-Punch level (0 = not packed) for each weapon. Reset
/// whenever a game starts; a weapon dropped takes its level with it (a
/// [`crate::WeaponDrop`]), and one picked up brings its own.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PapLevels {
    pub sniper: u8,
    pub ak: u8,
    pub knife: u8,
}

impl PapLevels {
    pub fn get(self, weapon: PapWeapon) -> u8 {
        match weapon {
            PapWeapon::Sniper => self.sniper,
            PapWeapon::Ak74 => self.ak,
            PapWeapon::Knife => self.knife,
        }
    }

    pub fn set(&mut self, weapon: PapWeapon, level: u8) {
        match weapon {
            PapWeapon::Sniper => self.sniper = level,
            PapWeapon::Ak74 => self.ak = level,
            PapWeapon::Knife => self.knife = level,
        }
    }
}

/// Points to pack a weapon up to `level` (1..=[`MAX_LEVEL`]) from the level
/// below — Cold War's prices.
pub fn cost(level: u8) -> u32 {
    match level {
        0 | 1 => 5_000,
        2 => 15_000,
        _ => 30_000,
    }
}

/// Points to pack a weapon at `current` straight up to `level`, skipping
/// any in between — every level on the way, paid for at once.
pub fn cost_to(current: u8, level: u8) -> u32 {
    (current + 1..=level.min(MAX_LEVEL)).map(cost).sum()
}

/// Damage multiplier at `level`: doubles every level (1, 2, 4, 8).
pub fn damage_mult(level: u8) -> f32 {
    (1u32 << level.min(MAX_LEVEL)) as f32
}

/// Roman numeral for `level` (1..=3) — the menu's tiles and header.
pub fn numeral(level: u8) -> &'static str {
    match level {
        1 => "I",
        2 => "II",
        3 => "III",
        _ => "",
    }
}

/// Where the Pack-a-Punch machine stands on `map` (the ground under its
/// middle), if the map has one — from its layout ([`crate::level`]).
pub fn machine_pos(map: MapId) -> Option<Vec3> {
    crate::level::layout(map).pack_a_punch.map(|p| p.pos)
}

/// Which way the machine faces on `map` (degrees about y).
pub fn machine_yaw_deg(map: MapId) -> f32 {
    crate::level::layout(map).pack_a_punch.map_or(0.0, |p| p.yaw_deg)
}

/// Half the machine's width, height and depth (m) — the client's model at
/// its scale (0.5 of the 2.5 × 4 × 1.2 m model).
pub const MACHINE_HALF_EXTENTS: Vec3 = Vec3::new(0.625, 1.0, 0.3);

/// The machine's solid box on `map`, if it has one (only there in
/// `Zombies`): `(centre, rotation, half extents)`.
pub fn machine_box(map: MapId) -> Option<(Vec3, bevy::math::Quat, Vec3)> {
    crate::level::layout(map).pack_a_punch.map(|at| {
        let half = MACHINE_HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}

/// Whether feet at `feet` are close enough to a Pack-a-Punch machine
/// standing at `machine` to use it — the same reach as a perk machine, plus
/// `slack`.
pub fn in_range_of(machine: Vec3, feet: Vec3, slack: f32) -> bool {
    Vec3::new(feet.x - machine.x, 0.0, feet.z - machine.z).length() <= PERK_USE_RADIUS + 0.5 + slack
        && (feet.y - machine.y).abs() <= PERK_USE_HEIGHT + slack
}

/// [`in_range_of`] `map`'s machine.
pub fn in_range(map: MapId, feet: Vec3, slack: f32) -> bool {
    machine_pos(map).is_some_and(|m| in_range_of(m, feet, slack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_doubles_each_level() {
        assert_eq!(damage_mult(0), 1.0);
        assert_eq!(damage_mult(1), 2.0);
        assert_eq!(damage_mult(2), 4.0);
        assert_eq!(damage_mult(3), 8.0);
    }

    #[test]
    fn each_level_costs_more() {
        assert!(cost(1) < cost(2) && cost(2) < cost(3));
    }

    #[test]
    fn skipping_levels_pays_for_every_one_on_the_way() {
        assert_eq!(cost_to(0, 1), cost(1));
        assert_eq!(cost_to(0, 3), cost(1) + cost(2) + cost(3));
        assert_eq!(cost_to(1, 3), cost(2) + cost(3));
        assert_eq!(cost_to(3, 3), 0);
    }

    #[test]
    fn levels_are_per_weapon() {
        let mut l = PapLevels::default();
        l.set(PapWeapon::Knife, 2);
        assert_eq!(l.get(PapWeapon::Knife), 2);
        assert_eq!(l.get(PapWeapon::Sniper), 0);
        assert_eq!(l.get(PapWeapon::Ak74), 0);
        assert_eq!(PapWeapon::of(SlotWeapon::Gun(WeaponId::Ak74)), PapWeapon::Ak74);
    }

    #[test]
    fn range_is_only_where_there_is_a_machine() {
        for map in MapId::PLACES {
            match machine_pos(map) {
                Some(m) => {
                    assert!(in_range(map, m, 0.0));
                    assert!(!in_range(map, m + Vec3::X * 10.0, 0.0));
                    assert!(in_range(map.with_night(!map.is_night()), m, 0.0), "day and night alike");
                }
                None => assert!(!in_range(map, Vec3::ZERO, 0.0)),
            }
        }
    }
}
