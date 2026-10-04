//! `Zombies` last stand and reviving, Call of Duty style.
//!
//! A player whose health hits zero in a `Zombies` game doesn't die: they go
//! **down** — prone, crawling slowly, unable to attack, and ignored by the
//! zombies — and start bleeding out ([`BLEED_OUT_SECS`]). A teammate who
//! holds the interact key next to them ([`REVIVE_RANGE`]) for
//! [`revive_secs`] brings them back up at full health; the bleed-out clock
//! stops while someone's reviving them. As it runs down they lose their
//! perks one at a time, most recently bought first ([`perks_kept`]). If it
//! runs out they **bleed out**: dead until the next round starts, when they
//! come back next to the party. Everybody down (or out) at once is game
//! over.
//!
//! Playing alone, Quick Revive gets you back up by yourself
//! ([`SOLO_SELF_REVIVE_SECS`]) — and is used up doing it. Without it a solo
//! down is game over, as in Call of Duty.
//!
//! The server owns all of it (`server::revive`); the state each client needs
//! is replicated as [`Downed`] on the downed player's entity.

use bevy::prelude::*;
use lightyear::prelude::PeerId;
use serde::{Deserialize, Serialize};

use crate::perks::Perk;

/// Seconds a downed player lasts before bleeding out.
pub const BLEED_OUT_SECS: f32 = 30.0;

/// Seconds a revive takes without Quick Revive...
pub const REVIVE_SECS: f32 = 3.0;
/// ...and with it (the reviver's Quick Revive, as in Call of Duty).
pub const QUICK_REVIVE_REVIVE_SECS: f32 = 1.5;

/// Seconds solo Quick Revive takes to get you back up.
pub const SOLO_SELF_REVIVE_SECS: f32 = 8.0;

/// Furthest (m, feet to feet) a reviver can be from who they're reviving.
pub const REVIVE_RANGE: f32 = 1.8;

/// How fast (m/s) a downed player crawls.
pub const DOWNED_CRAWL_SPEED: f32 = 0.35;

/// Seconds a revive takes for a reviver owning `reviver_perks`.
pub fn revive_secs(reviver_perks: &[Perk]) -> f32 {
    if reviver_perks.contains(&Perk::QuickRevive) {
        QUICK_REVIVE_REVIVE_SECS
    } else {
        REVIVE_SECS
    }
}

/// How many of the `count` perks a downed player held (in purchase order)
/// they still have with `left` of the bleed-out clock to go — they're
/// spaced evenly along it, so the `i`th (0-based) goes once less than
/// `(i + 1) / (count + 1)` of it is left: the last bought first, the first
/// bought last, and none before it starts or after it's out.
pub fn perks_kept(count: usize, left: f32) -> usize {
    let frac = (left / BLEED_OUT_SECS).clamp(0.0, 1.0);
    ((frac * (count + 1) as f32).floor() as usize).min(count)
}

/// Where along the bleed-out bar (`0.0` empty … `1.0` full) the `i`th of
/// `count` perks sits — the point it's lost at ([`perks_kept`]).
pub fn perk_mark(i: usize, count: usize) -> f32 {
    (i + 1) as f32 / (count + 1) as f32
}

/// On a `Zombies` player's entity while they're down (or bled out), from the
/// server (`server::revive`). Only changes on events — going down, a revive
/// starting or stopping, bleeding out — and each client runs the clocks on
/// from the snapshot itself, so it isn't re-sent every tick. Gone once
/// they're back up.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Downed {
    /// The perks they held when they went down, in purchase order — the
    /// bleed-out bar's icons ([`perk_mark`]). Which they still have is
    /// their `LobbyMember::perks`.
    pub perks: Vec<Perk>,
    /// Bleed-out seconds left as of this update. The clock's stopped while
    /// someone's reviving them (or the game's paused).
    pub bleed_left: f32,
    /// Who's reviving them right now, if anyone (themselves, for solo Quick
    /// Revive).
    pub reviver: Option<PeerId>,
    /// Seconds the current revive takes in all — it started at this update.
    pub revive_secs: f32,
    /// Bled out: dead until the next round.
    pub bled_out: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perks_go_last_bought_first_evenly_along_the_bleed_out() {
        // Three perks: marks at 1/4, 2/4, 3/4 of the clock.
        assert_eq!(perks_kept(3, BLEED_OUT_SECS), 3);
        assert_eq!(perks_kept(3, BLEED_OUT_SECS * 0.76), 3);
        assert_eq!(perks_kept(3, BLEED_OUT_SECS * 0.74), 2);
        assert_eq!(perks_kept(3, BLEED_OUT_SECS * 0.49), 1);
        assert_eq!(perks_kept(3, BLEED_OUT_SECS * 0.24), 0);
        assert_eq!(perks_kept(3, 0.0), 0);
        assert_eq!(perks_kept(0, BLEED_OUT_SECS), 0);
        // Each mark is exactly where its perk goes.
        for count in 1..6 {
            for i in 0..count {
                let at = perk_mark(i, count) * BLEED_OUT_SECS;
                assert!(perks_kept(count, at + 0.01) > i);
                assert!(perks_kept(count, at - 0.01) <= i);
            }
        }
    }

    #[test]
    fn quick_revive_halves_a_revive() {
        assert_eq!(revive_secs(&[]), REVIVE_SECS);
        assert_eq!(revive_secs(&[Perk::Juggernog, Perk::QuickRevive]), QUICK_REVIVE_REVIVE_SECS);
    }
}
