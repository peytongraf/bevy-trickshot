//! `Zombies` field upgrades, Call of Duty: Cold War's — so far just the
//! Aether Shroud. It builds up on its own over [`CHARGE_SECS`] while the
//! player's up in a running game, up to [`MAX_CHARGES`] stored; using one
//! ([`crate::UseFieldUpgrade`], the field upgrade key) hides them for
//! [`DURATION_SECS`]: the zombies lose track of them as if they weren't
//! there, they take no damage, they move [`SPEED_MULT`]× as fast, and their
//! guns are reloaded on the spot.
//!
//! A member's state is [`crate::LobbyMember::field_upgrade`] — server-owned
//! (`server::field_upgrades`), published in whole seconds so it only
//! replicates once a second, and cleared whenever a game starts or ends.

use serde::{Deserialize, Serialize};

/// How long (s) one charge takes to build.
pub const CHARGE_SECS: f32 = 120.0;
/// The most charges a player can have stored.
pub const MAX_CHARGES: u8 = 2;
/// How long (s) the Aether Shroud lasts once used.
pub const DURATION_SECS: f32 = 8.0;
/// Movement speed multiplier while it's up.
pub const SPEED_MULT: f32 = 1.25;

/// One member's field upgrade, as replicated.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FieldUpgrade {
    /// Charges stored, ready to use (0..=[`MAX_CHARGES`]).
    pub charges: u8,
    /// Whole seconds built toward the next charge (0 while every charge is
    /// stored).
    pub charge_secs: u16,
    /// Whole seconds left of the one in use (0 = none in use).
    pub active_secs: u16,
}

impl FieldUpgrade {
    /// Whether the Aether Shroud is up right now.
    pub fn active(&self) -> bool {
        self.active_secs > 0
    }

    /// Whether every charge is stored (nothing's building).
    pub fn full(&self) -> bool {
        self.charges >= MAX_CHARGES
    }

    /// How far the next charge has built, 0..=1 (1 once they're all stored).
    pub fn progress(&self) -> f32 {
        if self.full() {
            1.0
        } else {
            (self.charge_secs as f32 / CHARGE_SECS).clamp(0.0, 1.0)
        }
    }
}

/// The server's running clock for one member's field upgrade, from which
/// [`FieldUpgrade`] is published.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldUpgradeClock {
    pub charges: u8,
    /// Seconds built toward the next charge.
    pub charge: f32,
    /// Seconds left of the one in use.
    pub active_left: f32,
}

impl FieldUpgradeClock {
    /// Run it on by `dt` seconds: the one in use runs down, and (while
    /// `building` — up in a running game) the next charge builds.
    pub fn tick(&mut self, dt: f32, building: bool) {
        self.active_left = (self.active_left - dt).max(0.0);
        if self.charges >= MAX_CHARGES {
            self.charge = 0.0;
            return;
        }
        if building {
            self.charge += dt;
            if self.charge >= CHARGE_SECS {
                self.charge -= CHARGE_SECS;
                self.charges += 1;
                if self.charges >= MAX_CHARGES {
                    self.charge = 0.0;
                }
            }
        }
    }

    /// Use a charge, if there is one and none is in use already.
    pub fn try_use(&mut self) -> bool {
        if self.charges == 0 || self.active_left > 0.0 {
            return false;
        }
        self.charges -= 1;
        self.active_left = DURATION_SECS;
        true
    }

    /// What everyone sees.
    pub fn published(&self) -> FieldUpgrade {
        FieldUpgrade {
            charges: self.charges,
            charge_secs: self.charge.floor() as u16,
            active_secs: self.active_left.ceil() as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_charge_builds_in_two_minutes_and_stops_at_two() {
        let mut c = FieldUpgradeClock::default();
        for _ in 0..(CHARGE_SECS as usize - 1) {
            c.tick(1.0, true);
        }
        assert_eq!(c.charges, 0);
        c.tick(1.0, true);
        assert_eq!(c.charges, 1);
        assert_eq!(c.published().charge_secs, 0);
        for _ in 0..(CHARGE_SECS as usize * 3) {
            c.tick(1.0, true);
        }
        assert_eq!(c.charges, MAX_CHARGES);
        assert_eq!(c.published().progress(), 1.0);
    }

    #[test]
    fn nothing_builds_while_not_building() {
        let mut c = FieldUpgradeClock::default();
        c.tick(500.0, false);
        assert_eq!(c, FieldUpgradeClock::default());
    }

    #[test]
    fn using_one_lasts_its_duration_and_takes_a_charge() {
        let mut c = FieldUpgradeClock { charges: 2, ..Default::default() };
        assert!(c.try_use());
        assert_eq!(c.charges, 1);
        assert!(c.published().active());
        assert_eq!(c.published().active_secs, DURATION_SECS as u16);
        // (Not a second one on top.)
        assert!(!c.try_use());
        c.tick(DURATION_SECS - 0.5, true);
        assert!(c.published().active());
        c.tick(0.5, true);
        assert!(!c.published().active());
        assert!(c.try_use());
        assert_eq!(c.charges, 0);
        c.tick(DURATION_SECS, true);
        assert!(!c.try_use());
    }

    #[test]
    fn the_next_charge_keeps_building_while_one_is_in_use() {
        let mut c = FieldUpgradeClock { charges: 1, ..Default::default() };
        assert!(c.try_use());
        c.tick(DURATION_SECS, true);
        assert_eq!(c.published().charge_secs, DURATION_SECS as u16);
    }
}
