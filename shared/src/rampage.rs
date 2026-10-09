//! `Zombies`' Rampage Inducer, Call of Duty: Cold War's: a machine (placed in
//! the level editor, [`crate::level::ZombiesLayout::rampage_inducer`]) that
//! any player can turn on — by holding interact at it for [`HOLD_SECS`]
//! ([`crate::ToggleRampage`]) — and back off the same way. While it's on
//! (`Lobby::rampage`) every zombie moves faster — part of the way from its
//! own speed to the cap ([`rampage_speed`]) — and they spawn quicker
//! ([`SPAWN_INTERVAL_MULT`]).
//!
//! The server runs it (`server::rampage`); `Lobby::rampage` is reset
//! whenever a game starts or ends.

use bevy::math::{Quat, Vec3};

use crate::level::Placement;
use crate::perks::PERK_USE_HEIGHT;
use crate::zombies::{ZOMBIE_MAX_SPEED, ZOMBIE_RUN_SPEED};
use crate::{Lobby, MapId};

/// Seconds interact has to be held at the inducer to turn it on or off.
pub const HOLD_SECS: f32 = 1.0;
/// How far (m, across the ground) from its middle a player can use it, at
/// scale 1.
pub const USE_RADIUS: f32 = 2.4;
/// Its solid box (half extents) as made — `models/props/rampage_inducer.glb`
/// at scale 1 is a 5.3 m machine on a base about 2.2 m across, standing on
/// the ground; the map's size for it (`shared/levels/sizes.ron`) brings it
/// down to the game's scale.
pub const HALF_EXTENTS: Vec3 = Vec3::new(1.1, 1.96, 1.0);
/// The middle of its glass globe as made (where its light comes from).
pub const GLOBE_CENTRE: Vec3 = Vec3::new(0.0, 1.7, 0.0);
/// How far from its own speed toward the cap ([`ZOMBIE_MAX_SPEED`]) a
/// zombie goes while it's on (0 none, 1 all the way).
pub const SPEED_BLEND: f32 = 0.5;
/// While it's on, zombies spawn this many times as often apart.
pub const SPAWN_INTERVAL_MULT: f32 = 0.5;

/// `map`'s inducer, if it has one.
pub fn layout(map: MapId) -> Option<Placement> {
    crate::level::layout(map).rampage_inducer
}

/// Whether the inducer can be used in `lobby`'s game right now: a running
/// `Zombies` game on a map with one.
pub fn available(lobby: &Lobby) -> bool {
    lobby.started && lobby.mode == crate::GameMode::Zombies && layout(lobby.map).is_some()
}

/// Whether feet at `feet` can use an inducer standing at `at` (`slack`
/// widens it — the server allows for the pose being a moment old).
pub fn in_range_of(at: Placement, feet: Vec3, slack: f32) -> bool {
    let d = feet - at.pos;
    // (A bigger one reaches further out.)
    let reach = USE_RADIUS * at.scale.max(0.3).sqrt() + (at.scale - 1.0).max(0.0) * HALF_EXTENTS.x;
    Vec3::new(d.x, 0.0, d.z).length() <= reach + slack && d.y.abs() <= PERK_USE_HEIGHT + slack
}

/// Its solid box on `map`, if it has one: `(centre, rotation, half extents)`.
pub fn solid_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    layout(map).map(|at| {
        let half = HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}

/// A zombie's speed (m/s) while the inducer's on: between its own `speed`
/// and the cap — [`SPEED_BLEND`] of the way.
pub fn rampage_speed(speed: f32) -> f32 {
    (speed + (ZOMBIE_MAX_SPEED - speed).max(0.0) * SPEED_BLEND).min(ZOMBIE_MAX_SPEED)
}

/// Whether a zombie of `speed` runs (rather than walks) while it's on.
pub fn runs(speed: f32) -> bool {
    rampage_speed(speed) >= ZOMBIE_RUN_SPEED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zombies::ZOMBIE_MIN_SPEED;

    #[test]
    fn rampage_speed_is_between_its_own_and_the_cap() {
        for speed in [ZOMBIE_MIN_SPEED, 2.0, 5.0, ZOMBIE_MAX_SPEED] {
            let fast = rampage_speed(speed);
            assert!(fast >= speed && fast <= ZOMBIE_MAX_SPEED, "{speed} -> {fast}");
        }
        assert!(rampage_speed(ZOMBIE_MIN_SPEED) > ZOMBIE_MIN_SPEED);
        assert_eq!(rampage_speed(ZOMBIE_MAX_SPEED), ZOMBIE_MAX_SPEED);
        // Round 1's walkers break into a run.
        assert!(runs(ZOMBIE_MIN_SPEED));
    }

    #[test]
    fn it_can_be_used_from_beside_it() {
        let at = Placement::new(Vec3::ZERO, 0.0).with_scale(0.45);
        assert!(in_range_of(at, Vec3::new(1.5, 0.0, 0.0), 0.0));
        assert!(!in_range_of(at, Vec3::new(5.0, 0.0, 0.0), 0.0));
        assert!(!in_range_of(at, Vec3::new(0.5, 6.0, 0.0), 0.0));
    }
}
