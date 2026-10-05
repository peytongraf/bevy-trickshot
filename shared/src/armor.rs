//! `Zombies` armor, Call of Duty: Cold War's: up to [`MAX_LEVEL`] plates,
//! bought at the armor station (placed in the level editor,
//! [`crate::level::ZombiesLayout::armor_station`]) — any level above the one
//! owned can be bought straight away, paying for every one on the way
//! ([`cost_to`]), and each buy fills every plate. Every bit of damage a
//! player takes comes off their armor first ([`Armor::absorb`]), the top
//! plate emptying before the one under it; only once it's all gone does
//! their health go down. At the station they can refill what they own for
//! a flat [`REFILL_COST`].
//!
//! A member's armor is [`crate::LobbyMember::armor`] — server-owned
//! (`server::armor`), replicated for everyone's HUD, cleared whenever a
//! game starts and when they bleed out.

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::level::Placement;
use crate::perks::PERK_USE_HEIGHT;
use crate::MapId;

/// The most plates a player can own.
pub const MAX_LEVEL: u8 = 3;
/// How much damage one plate soaks up.
pub const PLATE_POINTS: f32 = 60.0;
/// What each level costs on its own ([`cost_to`] adds them up).
const LEVEL_COSTS: [u32; MAX_LEVEL as usize] = [1500, 3000, 5000];
/// What filling every owned plate back up costs, whatever the level.
pub const REFILL_COST: u32 = 500;

/// What buying up from `current` to `level` costs: every level on the way.
pub fn cost_to(current: u8, level: u8) -> u32 {
    (current.min(MAX_LEVEL)..level.min(MAX_LEVEL))
        .map(|l| LEVEL_COSTS[l as usize])
        .sum()
}

/// A level as a roman numeral (the station's tiles).
pub fn numeral(level: u8) -> &'static str {
    match level {
        1 => "I",
        2 => "II",
        _ => "III",
    }
}

/// A player's armor: the plates they own, and how much is left of them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Armor {
    /// Plates owned (0 = no armor).
    pub level: u8,
    /// What's left of them (`0..=level × PLATE_POINTS`).
    pub points: f32,
}

impl Armor {
    /// All `level` plates, full.
    pub fn full(level: u8) -> Self {
        let level = level.min(MAX_LEVEL);
        Self {
            level,
            points: level as f32 * PLATE_POINTS,
        }
    }

    pub fn max_points(&self) -> f32 {
        self.level as f32 * PLATE_POINTS
    }

    /// Whether every owned plate is full (or none's owned).
    pub fn is_full(&self) -> bool {
        self.points >= self.max_points() - 1e-3
    }

    /// How full plate `i` (0 = the bottom one) is, `0..=1` — the HUD's
    /// bars. The top plate empties first.
    pub fn plate_fill(&self, i: u8) -> f32 {
        if i >= self.level {
            return 0.0;
        }
        ((self.points - i as f32 * PLATE_POINTS) / PLATE_POINTS).clamp(0.0, 1.0)
    }

    /// Take `damage` on the armor first: what it soaks up comes off, and the
    /// rest — what gets through to health — is returned.
    pub fn absorb(&mut self, damage: f32) -> f32 {
        if damage <= 0.0 || self.points <= 0.0 {
            return damage.max(0.0);
        }
        let soaked = damage.min(self.points);
        self.points -= soaked;
        if self.points < 1e-3 {
            self.points = 0.0;
        }
        damage - soaked
    }
}

/// The station's footprint (half its width, height and depth, m) as placed
/// at scale 1 — `client::armor` fits the model to it.
pub const HALF_EXTENTS: Vec3 = Vec3::new(0.57, 1.1, 0.62);

/// How far (m, across the ground) from the station's middle a player can
/// use it.
pub const USE_RADIUS: f32 = 2.0;

/// Where `map`'s station stands, if it has one.
pub fn placement(map: MapId) -> Option<Placement> {
    crate::level::layout(map).armor_station
}

/// Whether feet at `feet` can use a station standing at `at` (`slack`
/// widens it — the server allows for the pose being a moment old).
pub fn in_range_of(at: Placement, feet: Vec3, slack: f32) -> bool {
    let d = feet - at.pos;
    // (A bigger station reaches further out.)
    let reach = USE_RADIUS + (at.scale - 1.0).max(0.0) * HALF_EXTENTS.x;
    Vec3::new(d.x, 0.0, d.z).length() <= reach + slack && d.y.abs() <= PERK_USE_HEIGHT + slack
}

/// Whether feet at `feet` can use `map`'s station.
pub fn in_range(map: MapId, feet: Vec3, slack: f32) -> bool {
    placement(map).is_some_and(|at| in_range_of(at, feet, slack))
}

/// The station's solid box on `map`, if it has one: `(centre, rotation,
/// half extents)`.
pub fn solid_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    placement(map).map(|at| {
        let half = HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buying_up_pays_for_every_level_on_the_way() {
        assert_eq!(cost_to(0, 1), 1500);
        assert_eq!(cost_to(0, 3), 9500);
        assert_eq!(cost_to(1, 3), 8000);
        assert_eq!(cost_to(3, 3), 0);
        assert!(REFILL_COST < cost_to(0, 1));
    }

    #[test]
    fn armor_soaks_damage_before_health_and_the_top_plate_goes_first() {
        let mut a = Armor::full(2);
        assert_eq!(a.absorb(34.0), 0.0);
        assert!((a.plate_fill(1) - (26.0 / 60.0)).abs() < 1e-4);
        assert_eq!(a.plate_fill(0), 1.0);
        // The rest of the top plate and the whole bottom one go, and what's
        // left gets through.
        assert!((a.absorb(100.0) - 14.0).abs() < 1e-4);
        assert_eq!(a.points, 0.0);
        assert_eq!(a.plate_fill(0), 0.0);
        // None left: it all gets through.
        assert_eq!(a.absorb(20.0), 20.0);
        assert_eq!(Armor::default().absorb(5.0), 5.0);
    }

    #[test]
    fn unowned_plates_are_empty() {
        let a = Armor::full(1);
        assert_eq!(a.plate_fill(0), 1.0);
        assert_eq!(a.plate_fill(1), 0.0);
        assert!(a.is_full());
        assert_eq!(Armor::full(9).level, MAX_LEVEL);
    }
}
