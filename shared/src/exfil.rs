//! `Zombies` exfil, Call of Duty: Cold War's: from round
//! [`EXFIL_FIRST_ROUND`] and every [`EXFIL_ROUND_EVERY`] rounds after (11,
//! 21, 31, ...), the radio (placed in the level editor,
//! [`crate::level::ZombiesLayout::exfil_radio`]) can be used to call in an
//! extraction. Holding interact at it for [`HOLD_SECS`] calls it
//! ([`crate::CallExfil`]): for [`CALLING_SECS`] every screen whites out and
//! no enemy attacks ([`Exfil::Calling`]) while the round's enemies are
//! cleared away, then the exfil wave starts ([`Exfil::Active`]) — tougher
//! than the round it replaces, with dogs and [`bosses`] — and the party has
//! [`TIME_LIMIT_SECS`] to kill every enemy of it. Only kills by a player
//! standing inside the exfil area ([`crate::level::ExfilArea`], shown by four
//! orange lines on the ground once it's called) count; an enemy killed from
//! outside it is replaced. Kill them all in time and the party escapes
//! ([`Exfil::Escaped`]), the round counting as survived; run out of time and
//! the game's over ([`Exfil::Failed`]), the round not counting.
//!
//! The server runs it (`server::exfil`); [`Exfil`] is replicated as
//! `Lobby::exfil` (reset whenever a game starts, kept after one ends for the
//! results screen).

use bevy::math::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::level::{ExfilArea, Placement};
use crate::perks::PERK_USE_HEIGHT;
use crate::{Lobby, MapId};

/// Exfil can first be called on this round, then every this-many rounds
/// after — 11, 21, 31, ...
pub const EXFIL_FIRST_ROUND: u32 = 11;
pub const EXFIL_ROUND_EVERY: u32 = 10;
/// Seconds interact has to be held at the radio to call it.
pub const HOLD_SECS: f32 = 1.0;
/// Seconds of white-out (and no attacks) once it's called.
pub const CALLING_SECS: f32 = 2.5;
/// Seconds the party has to kill the exfil wave.
pub const TIME_LIMIT_SECS: u32 = 90;
/// How many rounds tougher (health, speed, sharpness) the exfil wave's
/// zombies and dogs are than the round it's called on.
pub const ROUND_BOOST: u32 = 3;
/// How far (m, across the ground) from the radio's middle a player can use it.
pub const USE_RADIUS: f32 = 2.2;
/// The radio's solid box (half extents, m) at scale 1 — the model
/// (`models/props/exfil_radio.glb`) is this size as made, standing on the
/// ground.
pub const RADIO_HALF_EXTENTS: Vec3 = Vec3::new(0.9, 1.0, 0.5);
/// The area's size (m) when it's first put on a map.
pub const DEFAULT_AREA_SIZE: Vec2 = Vec2::new(14.0, 14.0);

/// Where a `Zombies` game's exfil is at (`Lobby::exfil`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Exfil {
    /// Not called (yet) this game.
    #[default]
    Idle,
    /// Just called: the white-out, when nothing attacks.
    Calling,
    /// The wave's on: whole seconds left to kill it (`Lobby::enemies_left`
    /// is what's still to kill).
    Active { secs_left: u32 },
    /// Every enemy killed in time — the game's over, and won.
    Escaped,
    /// Time ran out — the game's over, the round not survived.
    Failed,
}

impl Exfil {
    /// Called, and the game's still on.
    pub fn running(self) -> bool {
        matches!(self, Exfil::Calling | Exfil::Active { .. })
    }
}

/// Whether `round` is an exfil round.
pub fn is_exfil_round(round: u32) -> bool {
    round >= EXFIL_FIRST_ROUND && (round - EXFIL_FIRST_ROUND) % EXFIL_ROUND_EVERY == 0
}

/// How many bosses the exfil wave called on `round` brings: a tenth of the
/// round, plus one (2 on round 11, 3 on round 21, ...).
pub fn bosses(round: u32) -> u32 {
    round / 10 + 1
}

/// `map`'s exfil — its radio and its area — if it has both.
pub fn layout(map: MapId) -> Option<(Placement, ExfilArea)> {
    let l = crate::level::layout(map);
    Some((l.exfil_radio?, l.exfil_area?))
}

/// Whether exfil can be called in `lobby`'s game right now: an exfil round
/// that's started (not the pre-game countdown), not called yet, on a map
/// with a radio and an area.
pub fn available(lobby: &Lobby) -> bool {
    lobby.started
        && lobby.mode == crate::GameMode::Zombies
        && lobby.countdown_left == 0
        && is_exfil_round(lobby.round)
        && lobby.exfil == Exfil::Idle
        && layout(lobby.map).is_some()
}

/// Whether feet at `feet` can use a radio standing at `at` (`slack` widens
/// it — the server allows for the pose being a moment old).
pub fn radio_in_range_of(at: Placement, feet: Vec3, slack: f32) -> bool {
    let d = feet - at.pos;
    // (A bigger radio reaches further out.)
    let reach = USE_RADIUS + (at.scale - 1.0).max(0.0) * RADIO_HALF_EXTENTS.x;
    Vec3::new(d.x, 0.0, d.z).length() <= reach + slack && d.y.abs() <= PERK_USE_HEIGHT + slack
}

/// The radio's solid box on `map`, if it has one: `(centre, rotation, half
/// extents)`.
pub fn radio_box(map: MapId) -> Option<(Vec3, bevy::math::Quat, Vec3)> {
    crate::level::layout(map).exfil_radio.map(|at| {
        let half = RADIO_HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}

impl ExfilArea {
    /// Whether `feet` is inside it, seen from above (any height).
    pub fn contains(&self, feet: Vec3) -> bool {
        let local = self.at.rotation().inverse() * (feet - self.at.pos);
        local.x.abs() <= self.width * 0.5 && local.z.abs() <= self.depth * 0.5
    }

    /// Its four corners on the ground, in order round it.
    pub fn corners(&self) -> [Vec3; 4] {
        let rot = self.at.rotation();
        let (hx, hz) = (self.width * 0.5, self.depth * 0.5);
        [(-hx, -hz), (hx, -hz), (hx, hz), (-hx, hz)].map(|(x, z)| self.at.pos + rot * Vec3::new(x, 0.0, z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exfil_rounds_and_their_bosses() {
        assert!(!is_exfil_round(0));
        assert!(!is_exfil_round(1) && !is_exfil_round(10));
        assert!(is_exfil_round(11) && is_exfil_round(21) && is_exfil_round(31));
        assert!(!is_exfil_round(15) && !is_exfil_round(20) && !is_exfil_round(22));
        assert_eq!(bosses(11), 2);
        assert_eq!(bosses(21), 3);
        assert_eq!(bosses(31), 4);
    }

    #[test]
    fn the_area_is_a_turned_rectangle() {
        let area = ExfilArea {
            at: Placement::new(Vec3::new(10.0, 2.0, 0.0), 90.0),
            width: 4.0,
            depth: 10.0,
        };
        // Turned a quarter: its width runs along z, its depth along x.
        assert!(area.contains(Vec3::new(14.5, 0.0, 0.0)));
        assert!(!area.contains(Vec3::new(10.0, 2.0, 2.5)));
        assert!(area.contains(Vec3::new(10.0, 9.0, 1.9)));
        for c in area.corners() {
            assert!(area.contains(c * 0.999 + area.at.pos * 0.001));
        }
    }

    #[test]
    fn the_radio_is_used_from_a_couple_of_metres() {
        let at = Placement::new(Vec3::ZERO, 0.0);
        assert!(radio_in_range_of(at, Vec3::X * 2.0, 0.0));
        assert!(!radio_in_range_of(at, Vec3::X * 3.0, 0.0));
    }
}
