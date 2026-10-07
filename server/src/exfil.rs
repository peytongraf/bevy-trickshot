//! `Zombies` exfil (`shared::exfil`), on top of `crate::zombies`' rounds.
//!
//! A member holding interact at the radio on an exfil round calls it
//! ([`on_call_exfil`]): `Lobby::exfil` goes to `Calling` — nothing attacks
//! (`Lobby::bots_hold_fire`) and every screen whites out — and half-way into
//! that, while the screens are white, the round's enemies are cleared away
//! ([`ExfilRun::clear_round`]). Then the wave: `crate::zombies::run_rounds`
//! keeps exactly as many enemies alive or on their way as there are kills
//! still needed ([`ExfilRun::top_up`]) — so the last one needed is the last
//! one there — spawning them quickly, a mix of zombies and hellhounds a few
//! rounds tougher than the round's ([`shared::exfil::ROUND_BOOST`]), plus
//! [`shared::exfil::bosses`]. Only a kill by a player inside the exfil area
//! counts ([`ExfilRun::count_kill`]); one from outside it is replaced. Every
//! kill needed in time and the game ends escaped; time up first, failed.
//!
//! The run lives on the lobby's `ZombieRounds` (gone with the game), and
//! `Lobby::exfil` is reset when the next game starts (`lobby::on_start`).

use bevy::prelude::*;
use lightyear::prelude::*;

use shared::exfil::{Exfil, CALLING_SECS, TIME_LIMIT_SECS};
use shared::{CallExfil, GameMode, Lobby, PlayerId, PlayerPose};

use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;
use crate::zombies::ZombieRounds;

/// Seconds between the wave's spawns — quick, so the clock's spent
/// fighting rather than waiting.
pub(crate) const SPAWN_INTERVAL_SECS: f32 = 0.3;
/// The chance a spawn is a hellhound rather than a zombie.
pub(crate) const DOG_SHARE: f32 = 0.3;

/// A called exfil, on its lobby's `ZombieRounds`.
pub(crate) struct ExfilRun {
    /// When the white-out ends and the wave begins (`Time::elapsed_secs`).
    calling_until: f32,
    /// The round's own enemies have been cleared away.
    cleared: bool,
    /// When time runs out, once the wave's begun.
    ends_at: Option<f32>,
    /// Kills (from inside the area) still needed...
    pub(crate) kills_left: u32,
    /// ...and how many of those are bosses.
    pub(crate) bosses_left: u32,
}

impl ExfilRun {
    /// Called at `now`, on `round`, for `members` players.
    pub(crate) fn new(now: f32, round: u32, members: usize) -> Self {
        let zombies = crate::zombies::zombies_in_round(round, members);
        let dogs = shared::dogs::dogs_in_round(round, members).div_ceil(2);
        let bosses = shared::exfil::bosses(round);
        Self {
            calling_until: now + CALLING_SECS,
            cleared: false,
            ends_at: None,
            kills_left: zombies + dogs + bosses,
            bosses_left: bosses,
        }
    }

    /// The game's been paused for `dt` seconds: hold every clock.
    pub(crate) fn pause(&mut self, dt: f32) {
        self.calling_until += dt;
        if let Some(t) = self.ends_at.as_mut() {
            *t += dt;
        }
    }

    /// Whether it's time to clear the round's enemies away — half-way into
    /// the white-out, when every screen's white.
    pub(crate) fn wants_clear(&self, now: f32) -> bool {
        !self.cleared && now >= self.calling_until - CALLING_SECS * 0.5
    }

    /// The round's enemies are gone (`crate::zombies::run_rounds` despawned
    /// them): nothing of it is still coming either.
    pub(crate) fn clear_round(&mut self, rounds_to_spawn: &mut u32) {
        self.cleared = true;
        *rounds_to_spawn = 0;
    }

    /// Start the wave once the white-out's over: `Some(time limit end)`
    /// the moment it starts, else `None`.
    pub(crate) fn begin_if_due(&mut self, now: f32) -> Option<f32> {
        if self.ends_at.is_none() && now >= self.calling_until {
            self.ends_at = Some(now + TIME_LIMIT_SECS as f32);
            return self.ends_at;
        }
        None
    }

    /// Whether the wave's begun.
    pub(crate) fn begun(&self) -> bool {
        self.ends_at.is_some()
    }

    /// Whole seconds left on the clock.
    pub(crate) fn secs_left(&self, now: f32) -> u32 {
        self.ends_at.map_or(TIME_LIMIT_SECS, |t| (t - now).max(0.0).ceil() as u32)
    }

    pub(crate) fn timed_out(&self, now: f32) -> bool {
        self.ends_at.is_some_and(|t| now >= t)
    }

    /// A kill: counted only from inside the area (`inside`).
    pub(crate) fn count_kill(&mut self, inside: bool, boss: bool) {
        if !inside || !self.begun() {
            return;
        }
        self.kills_left = self.kills_left.saturating_sub(1);
        if boss {
            self.bosses_left = self.bosses_left.saturating_sub(1);
        }
    }

    /// How many more bosses and other enemies to send, given those alive
    /// and on their way: exactly enough that every kill still needed has
    /// one enemy to kill (an enemy killed from outside the area is replaced
    /// this way). `(bosses, others)`.
    pub(crate) fn top_up(&self, bosses_there: u32, others_there: u32) -> (u32, u32) {
        let others_left = self.kills_left.saturating_sub(self.bosses_left);
        (
            self.bosses_left.saturating_sub(bosses_there),
            others_left.saturating_sub(others_there),
        )
    }
}

pub struct ExfilPlugin;

impl Plugin for ExfilPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_call_exfil);
    }
}

/// A member wants to call the exfil: they must be in a running, unpaused
/// `Zombies` game where it can be called (`shared::exfil::available`),
/// alive and at the radio (a little slack — their pose is a moment old).
/// Anything else is ignored.
fn on_call_exfil(
    trigger: Trigger<RemoteTrigger<CallExfil>>,
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby, &mut ZombieRounds)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, mut lobby, mut rounds)) = lobbies
        .iter_mut()
        .find(|(_, l, _)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e)
        || lobby.paused
        || !shared::exfil::available(&lobby)
        || rounds.round != lobby.round
        || rounds.exfil.is_some()
    {
        return;
    }
    let Some((radio, _)) = shared::exfil::layout(lobby.map) else {
        return;
    };
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    if !combat.alive || !shared::exfil::radio_in_range_of(radio, feet, 0.75) {
        return;
    }
    let members = lobby.real_count();
    rounds.exfil = Some(ExfilRun::new(time.elapsed_secs(), rounds.round, members));
    lobby.exfil = Exfil::Calling;
    info!("lobby {lobby_e:?}: {peer:?} called the exfil on round {}", rounds.round);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kills_inside_count_and_outside_ones_are_replaced() {
        let mut run = ExfilRun::new(0.0, 10, 1);
        let total = run.kills_left;
        assert_eq!(run.bosses_left, 2);
        // Nothing counts during the white-out.
        run.count_kill(true, false);
        assert_eq!(run.kills_left, total);
        assert!(run.begin_if_due(CALLING_SECS).is_some());
        assert_eq!(run.secs_left(CALLING_SECS), TIME_LIMIT_SECS);
        // Everything there: nothing more to send.
        assert_eq!(run.top_up(2, total - 2), (0, 0));
        // One killed from outside: it doesn't count, and it's sent again.
        run.count_kill(false, false);
        assert_eq!(run.kills_left, total);
        assert_eq!(run.top_up(2, total - 3), (0, 1));
        // A boss killed from inside: one fewer needed, no replacement.
        run.count_kill(true, true);
        assert_eq!((run.kills_left, run.bosses_left), (total - 1, 1));
        assert_eq!(run.top_up(1, total - 2), (0, 0));
        // A boss killed from outside comes back.
        assert_eq!(run.top_up(0, total - 2), (1, 0));
        assert!(run.timed_out(CALLING_SECS + TIME_LIMIT_SECS as f32));
    }
}
