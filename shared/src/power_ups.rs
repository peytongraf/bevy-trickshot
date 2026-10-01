//! `Zombies` power-ups, Call of Duty style: a killed zombie sometimes drops
//! one ([`roll`]), it floats where it fell for [`DROP_LIFETIME_SECS`]
//! (blinking for the last [`BLINK_SECS`]), and the first player to walk into
//! it ([`in_pickup_range`]) sets it off for the whole team:
//!
//! * **Max Ammo** — everyone's ammo (sniper, throwing knives and any molotovs) full.
//! * **Insta-Kill** — any hit kills a zombie, for [`TIMED_SECS`].
//! * **Double Points** — zombie kills score double, for [`TIMED_SECS`].
//! * **Nuke** — every zombie up dies, each at a random moment within
//!   [`NUKE_KILL_SECS`] (bursting into flames), everyone gets [`NUKE_POINTS`],
//!   and no more rise for [`NUKE_SPAWN_PAUSE_SECS`].
//! * **Bonus Points** — [`BONUS_POINTS`] for whoever grabbed it.
//!
//! The server owns all of it (`server::power_ups`); clients draw the drops
//! ([`crate::PowerUpDrop`]), play the sounds on [`crate::PowerUpGrabbed`], and
//! show the running timers from [`crate::Lobby::active_power_ups`].

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerUp {
    MaxAmmo,
    InstaKill,
    DoublePoints,
    Nuke,
    BonusPoints,
}

/// How long (s) Insta-Kill and Double Points last. Grabbing one while it's
/// already running starts it over.
pub const TIMED_SECS: f32 = 30.0;
/// How long (s) a drop floats there before it's gone.
pub const DROP_LIFETIME_SECS: f32 = 30.0;
/// It blinks for this long (s) before it goes.
pub const BLINK_SECS: f32 = 8.0;
/// Points every player gets from a Nuke (doubled by Double Points).
pub const NUKE_POINTS: u32 = 500;
/// Points Bonus Points gives whoever grabbed it.
pub const BONUS_POINTS: u32 = 500;
/// A Nuke's zombies die one by one, each at a random moment within this
/// long (s) of it going off.
pub const NUKE_KILL_SECS: f32 = 5.0;
/// After a Nuke, no zombie rises for this long (s) — the whole time its
/// zombies are still dropping — unless it ended the round, whose own break
/// then follows.
pub const NUKE_SPAWN_PAUSE_SECS: f32 = NUKE_KILL_SECS;
/// How close (m, across the ground) a player's feet must get to a drop.
pub const PICKUP_RADIUS: f32 = 1.4;
/// ...and how far (m) above or below it.
pub const PICKUP_HEIGHT: f32 = 2.0;

impl PowerUp {
    pub const ALL: [PowerUp; 5] = [
        PowerUp::MaxAmmo,
        PowerUp::InstaKill,
        PowerUp::DoublePoints,
        PowerUp::Nuke,
        PowerUp::BonusPoints,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PowerUp::MaxAmmo => "MAX AMMO",
            PowerUp::InstaKill => "INSTA-KILL",
            PowerUp::DoublePoints => "DOUBLE POINTS",
            PowerUp::Nuke => "NUKE",
            PowerUp::BonusPoints => "BONUS POINTS",
        }
    }

    /// The chance (0–1) a zombie kill drops this one — about 1 in 16 kills
    /// drops something, the common ones more often.
    pub fn drop_chance(self) -> f32 {
        match self {
            PowerUp::BonusPoints => 0.016,
            PowerUp::MaxAmmo => 0.014,
            PowerUp::DoublePoints => 0.012,
            PowerUp::InstaKill => 0.010,
            PowerUp::Nuke => 0.008,
        }
    }

    /// Runs for [`TIMED_SECS`] rather than happening at once.
    pub fn timed(self) -> bool {
        matches!(self, PowerUp::InstaKill | PowerUp::DoublePoints)
    }
}

/// What one zombie kill drops, from a uniform `r` in `[0, 1)`: at most one
/// power-up, each with its own [`PowerUp::drop_chance`]. `always` (the
/// debug "100% drops" toggle) makes every kill drop one, still weighted by
/// those chances.
pub fn roll(r: f32, always: bool) -> Option<PowerUp> {
    let total: f32 = PowerUp::ALL.iter().map(|p| p.drop_chance()).sum();
    let scale = if always { 1.0 / total } else { 1.0 };
    let mut acc = 0.0;
    for p in PowerUp::ALL {
        acc += p.drop_chance() * scale;
        if r < acc {
            return Some(p);
        }
    }
    // (Floating-point slack at the very top of the range.)
    always.then_some(PowerUp::ALL[PowerUp::ALL.len() - 1])
}

/// Whether feet at `feet` are close enough to grab a drop lying at `drop`
/// (its spot on the ground).
pub fn in_pickup_range(feet: Vec3, drop: Vec3) -> bool {
    let d = feet - drop;
    Vec3::new(d.x, 0.0, d.z).length() <= PICKUP_RADIUS && d.y.abs() <= PICKUP_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chances_differ_and_stay_small() {
        let mut seen = Vec::new();
        for p in PowerUp::ALL {
            let c = p.drop_chance();
            assert!(c > 0.0 && c < 0.05);
            assert!(!seen.contains(&c.to_bits()), "{p:?} shares a chance");
            seen.push(c.to_bits());
        }
    }

    #[test]
    fn a_roll_drops_at_most_one_at_about_the_set_rate() {
        let n = 100_000;
        let mut counts = [0u32; 5];
        let mut none = 0;
        for i in 0..n {
            let r = (i as f32 + 0.5) / n as f32;
            match roll(r, false) {
                Some(p) => counts[PowerUp::ALL.iter().position(|q| *q == p).unwrap()] += 1,
                None => none += 1,
            }
        }
        for (k, p) in PowerUp::ALL.iter().enumerate() {
            let rate = counts[k] as f32 / n as f32;
            assert!((rate - p.drop_chance()).abs() < 0.001, "{p:?}: {rate}");
        }
        assert!(none > n * 9 / 10);
    }

    #[test]
    fn the_test_toggle_always_drops_every_kind() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..1000 {
            let r = i as f32 / 1000.0;
            seen.insert(roll(r, true).expect("no drop with the test toggle on"));
        }
        assert_eq!(seen.len(), 5);
        assert!(roll(0.99999, true).is_some());
    }

    #[test]
    fn pickup_range() {
        let drop = Vec3::new(10.0, 0.0, 5.0);
        assert!(in_pickup_range(drop + Vec3::X, drop));
        assert!(!in_pickup_range(drop + Vec3::X * 2.0, drop));
        assert!(!in_pickup_range(drop + Vec3::Y * 3.0, drop));
    }
}
