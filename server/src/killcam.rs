//! Kill-cam recording. Every player carries a per-tick ring buffer of
//! [`shared::KillCamSample`]s (eye pose + one-shot sound bits). On a bot kill we
//! note the killer + tick, wait for the follow-through to be buffered, then
//! ship the `[kill − 3 s, kill + 1.5 s]` window to the lobby.
//!
//! It also keeps each match's end-of-match replay: `Freestyle`'s best-scoring
//! shot ([`BestPlays`]), and for `FreeForAll` either the match's best play (the
//! most kills one player got inside a replay window) or its final kill
//! ([`FfaPlays`], per the lobby's [`shared::EndCam`]). Ending a match is
//! deferred ([`EndingLobbies`]) until the last kill's follow-through has been
//! buffered, so the final kill can actually be replayed.

use std::collections::VecDeque;

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{
    Bot, GameChannel, KillCam, KillCamBot, KillCamPlayer, KillCamSample, KnifeSample, Lobby,
    PlayerId, PlayerInput, PlayerPose, ThrownKnife,
};

use crate::bots::{BotHit, LobbyBot};
use crate::pvp::PlayerKilled;

/// Samples before / after the kill (at `TICK_HZ`).
const PRE: u64 = (shared::TICK_HZ as u64) * 3;
const POST: u64 = (shared::TICK_HZ * 1.5) as u64;
/// Ring-buffer capacity (a touch over `PRE + POST`).
const CAP: usize = (PRE + POST + 32) as usize;

/// One player's rolling recording.
#[derive(Component, Default)]
pub struct ReplayBuffer {
    frames: VecDeque<(u64, KillCamSample)>,
}

/// Monotonic tick counter for buffer bookkeeping (avoids `Tick` wrap math).
#[derive(Resource, Default)]
pub(crate) struct ReplayClock(pub(crate) u64);

struct PendingCam {
    killer: PeerId,
    killer_name: String,
    kill_seq: u64,
    bots: Vec<KillCamBot>,
    players: Vec<KillCamPlayer>,
    /// Points this shot scored (`shared::scoring::score_multi_kill`'s total)
    /// — compared against `BestPlays` once the replay window is flushed.
    /// Always `0` for a `FreeForAll` kill (no style scoring there; those
    /// are tracked by kill count in [`FfaPlays`] instead).
    points: u32,
    lobby: Entity,
    /// Who to send the replay to once it's ready: the whole lobby (bot
    /// kills — a shared highlight), or just this one peer (`FreeForAll`
    /// — only the victim watches how they died; nobody, if that victim is a
    /// bot — the replay is still built, for [`FfaPlays`]).
    target: Option<PeerId>,
}

#[derive(Resource, Default)]
struct PendingCams(Vec<PendingCam>);

/// The single highest-scoring kill cam seen so far this match, per lobby —
/// replayed to everyone (with `KillCam::best_play` set) right before
/// [`shared::MatchOver`] when the clock hits zero. Cleared the moment it's
/// sent, so a lobby that plays another match from the same room starts fresh.
#[derive(Resource, Default)]
pub(crate) struct BestPlays(std::collections::HashMap<Entity, BestPlay>);

pub(crate) struct BestPlay {
    points: u32,
    msg: KillCam,
}

impl BestPlays {
    /// Take (and clear) the best play recorded for `lobby`, if any scoring
    /// shot happened this match.
    pub(crate) fn take(&mut self, lobby: Entity) -> Option<KillCam> {
        self.0.remove(&lobby).map(|b| b.msg)
    }
}

/// Ticks an ending waits before it can conclude there's nothing pending.
const END_SETTLE_TICKS: u64 = 4;

/// One `FreeForAll` lobby's kills this match, for its end-of-match replay.
#[derive(Default)]
pub(crate) struct FfaMatch {
    /// `(tick, killer)` of every kill so far — counted for the best play.
    kills: Vec<(u64, PeerId)>,
    /// The most recent kill's replay.
    last: Option<KillCam>,
    /// The replay of the kill that capped the busiest window so far, and how
    /// many kills (by that killer) it had in it.
    best: Option<(u32, KillCam)>,
}

/// Every `FreeForAll` lobby's [`FfaMatch`].
#[derive(Resource, Default)]
pub(crate) struct FfaPlays(std::collections::HashMap<Entity, FfaMatch>);

impl FfaPlays {
    /// Take (and clear) `lobby`'s replay for `cam`, if it has one.
    pub(crate) fn take(&mut self, lobby: Entity, cam: shared::EndCam) -> Option<KillCam> {
        let m = self.0.remove(&lobby)?;
        let mut msg = match cam {
            shared::EndCam::BestPlay => m.best.map(|(_, msg)| msg)?,
            shared::EndCam::FinalKill => {
                let mut msg = m.last?;
                msg.final_kill = true;
                msg
            }
        };
        msg.best_play = true;
        Some(msg)
    }
}

/// Lobbies whose match is over but which haven't been told yet, each with the
/// tick it began at. The last kill's replay needs `POST` ticks of
/// follow-through buffered before it can be sent, so an ending waits for every
/// pending replay to flush ([`finish_endings`]) — and, meanwhile, the final
/// kill's own victim isn't sent a separate replay if the end-of-match one is
/// the final kill (they'd see it twice).
#[derive(Resource, Default)]
pub(crate) struct EndingLobbies(std::collections::HashMap<Entity, u64>);

impl EndingLobbies {
    /// Start ending `lobby` (a no-op if it already is).
    pub(crate) fn begin(&mut self, lobby: Entity, now: u64) {
        self.0.entry(lobby).or_insert(now);
    }

    pub(crate) fn is_ending(&self, lobby: Entity) -> bool {
        self.0.contains_key(&lobby)
    }
}

/// How many kills `killer` got in the replay window ending at tick `at`
/// (inclusive) — the "3 s before the kill" a kill cam shows.
fn kills_in_window(kills: &[(u64, PeerId)], killer: PeerId, at: u64) -> u32 {
    kills
        .iter()
        .filter(|(seq, k)| *k == killer && in_range(*seq, at.wrapping_sub(PRE - 1), at))
        .count() as u32
}

pub struct KillCamPlugin;

impl Plugin for KillCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplayClock>()
            .init_resource::<PendingCams>()
            .init_resource::<BestPlays>()
            .init_resource::<FfaPlays>()
            .init_resource::<EndingLobbies>()
            .add_systems(
                FixedUpdate,
                (
                    advance_clock,
                    attach_buffers,
                    record_frames,
                    queue_killcams,
                    flush_killcams,
                    finish_endings,
                )
                    .chain(),
            );
    }
}

fn advance_clock(mut clock: ResMut<ReplayClock>) {
    clock.0 = clock.0.wrapping_add(1);
}

fn attach_buffers(
    missing: Query<Entity, (With<PlayerId>, Without<ReplayBuffer>)>,
    mut commands: Commands,
) {
    for e in &missing {
        commands.entity(e).insert(ReplayBuffer::default());
    }
}

fn record_frames(
    clock: Res<ReplayClock>,
    mut players: Query<(&PlayerId, &ActionState<PlayerInput>, &mut ReplayBuffer)>,
    knives: Query<(Entity, &ThrownKnife)>,
) {
    for (id, action, mut buf) in &mut players {
        let i = &action.0;
        // This player's own thrown knives right now (the server owns their
        // simulation, so it's stamped here rather than sent by the client),
        // in a stable (entity) order so a knife keeps its slot across frames.
        let mut mine: Vec<(Entity, &ThrownKnife)> =
            knives.iter().filter(|(_, k)| k.owner == id.0).collect();
        mine.sort_by_key(|(e, _)| *e);
        let mut thrown_knives = [None; shared::throwing_knife::MAX_KNIVES_PER_PLAYER];
        for (slot, (_, k)) in thrown_knives.iter_mut().zip(mine) {
            *slot = Some(KnifeSample {
                pos: k.pos.to_array(),
                rot: k.rot.to_array(),
            });
        }
        buf.frames.push_back((
            clock.0,
            KillCamSample {
                translation: i.translation,
                yaw: i.yaw,
                pitch: i.pitch,
                shake_trauma: i.shake_trauma,
                shake_phase: i.shake_phase,
                shake_recoil: i.shake_recoil,
                sway_offset: i.sway_offset,
                fov_deg: i.fov_deg,
                scope_zoom: i.scope_zoom,
                sound_bits: i.sound_bits,
                anim_time: i.anim_time,
                knife_anim_time: i.knife_anim_time,
                ads_t: i.ads_t,
                ground_pt: i.ground_pt,
                blood_pt: i.blood_pt,
                tracer: i.tracer,
                weapon_visible: i.weapon_visible,
                knife_active: i.knife_active,
                sniper_active: i.sniper_active,
                knife_visible: i.knife_visible,
                arms_slide: i.arms_slide,
                arms_anim_time: i.arms_anim_time,
                arms_knife_in_hand: i.arms_knife_in_hand,
                thrown_knives,
                crouch_drop: i.crouch_drop,
            },
        ));
        while buf.frames.len() > CAP {
            buf.frames.pop_front();
        }
    }
}

fn queue_killcams(
    clock: Res<ReplayClock>,
    mut bot_hits: EventReader<BotHit>,
    mut player_kills: EventReader<PlayerKilled>,
    lobbies: Query<(Entity, &Lobby)>,
    bots: Query<(Entity, &Bot, &LobbyBot)>,
    players: Query<(&PlayerId, &PlayerPose)>,
    mut pending: ResMut<PendingCams>,
    mut ffa: ResMut<FfaPlays>,
) {
    // Group this tick's bot kills by shooter first — a collateral shot fires
    // one `BotHit` per pierced bot, all in the same tick, and should become
    // one kill cam with every one of them falling together, not a separate
    // replay per bot. Only the first `BotHit` of a pierced chain carries
    // non-zero `points` (see `resolve_shots`), so summing is equivalent to
    // taking it.
    let mut by_shooter: Vec<(PeerId, Vec<Entity>, u32)> = Vec::new();
    for ev in bot_hits.read() {
        match by_shooter.iter_mut().find(|(shooter, ..)| *shooter == ev.by) {
            Some((_, killed, points)) => {
                killed.push(ev.bot);
                *points += ev.points;
            }
            None => by_shooter.push((ev.by, vec![ev.bot], ev.points)),
        }
    }

    // Freezes the killer's lobby (bots + every other member) as it stands
    // now and queues one replay window for it. `target = None` broadcasts to
    // the whole lobby (a bot kill — a shared highlight); `Some(peer)` sends
    // it to just that one peer (a `FreeForAll` kill — only the victim
    // watches how they died).
    let mut queue_one =
        |killer: PeerId, killed_bots: &[Entity], points: u32, target: Option<PeerId>| {
            let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.has(killer)) else {
                return;
            };
            let name = lobby
                .members
                .iter()
                .find(|m| m.peer == killer)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Someone".to_string());
            let snap: Vec<KillCamBot> = bots
                .iter()
                .filter(|(_, _, lb)| lb.lobby == lobby_e)
                .map(|(e, b, _)| KillCamBot {
                    pos: b.pos.to_array(),
                    yaw: b.yaw,
                    killed: killed_bots.contains(&e),
                })
                .collect();
            // Freeze every other lobby member too — the killer excluded,
            // since the replay is a first-person fly-through of their own
            // view. For a `FreeForAll` kill this includes the victim, which
            // is exactly who needs to see themselves get shot.
            let player_snap: Vec<KillCamPlayer> = players
                .iter()
                .filter(|(id, _)| id.0 != killer && lobby.has(id.0))
                .map(|(id, pose)| KillCamPlayer {
                    pos: pose.translation.to_array(),
                    yaw: pose.yaw,
                    // `target` is only ever `Some` for a `FreeForAll` kill,
                    // where it's the victim.
                    killed: target == Some(id.0),
                })
                .collect();
            if target.is_some() {
                ffa.0.entry(lobby_e).or_default().kills.push((clock.0, killer));
            }
            pending.0.push(PendingCam {
                killer,
                killer_name: name,
                kill_seq: clock.0,
                players: player_snap,
                bots: snap,
                points,
                lobby: lobby_e,
                target,
            });
        };

    for (killer, killed_bots, points) in &by_shooter {
        queue_one(*killer, killed_bots, *points, None);
    }
    for ev in player_kills.read() {
        queue_one(ev.killer, &[], 0, Some(ev.victim));
    }
}

fn flush_killcams(
    clock: Res<ReplayClock>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut pending: ResMut<PendingCams>,
    mut best_plays: ResMut<BestPlays>,
    mut ffa: ResMut<FfaPlays>,
    endings: Res<EndingLobbies>,
    buffers: Query<(&PlayerId, &ReplayBuffer)>,
    lobbies: Query<(Entity, &Lobby)>,
) {
    let server = server.into_inner();
    let now = clock.0;
    pending.0.retain(|cam| {
        if now.wrapping_sub(cam.kill_seq) < POST {
            return true; // not enough follow-through buffered yet
        }

        let Some((_, buf)) = buffers.iter().find(|(pid, _)| pid.0 == cam.killer) else {
            return false; // killer gone
        };
        let lo = cam.kill_seq.wrapping_sub(PRE);
        let hi = cam.kill_seq.wrapping_add(POST);
        let window: Vec<KillCamSample> = buf
            .frames
            .iter()
            .filter(|(seq, _)| in_range(*seq, lo, hi))
            .map(|(_, s)| *s)
            .collect();
        let kill_index = buf
            .frames
            .iter()
            .filter(|(seq, _)| in_range(*seq, lo, hi) && in_range(*seq, lo, cam.kill_seq))
            .count()
            .saturating_sub(1) as u32;

        if window.len() < 8 {
            return false; // nothing worth showing
        }

        let targets: Vec<PeerId> = match cam.target {
            Some(peer) => vec![peer],
            None => lobbies
                .iter()
                .find(|(_, l)| l.has(cam.killer))
                .map(|(_, l)| l.real_peers())
                .unwrap_or_default(),
        };
        // No client behind a bot victim; and if the lobby's match is ending on
        // its final kill, the end-of-match replay of it goes to everyone
        // instead (see `EndingLobbies`).
        let final_kill_ending = endings.is_ending(cam.lobby)
            && lobbies
                .get(cam.lobby)
                .is_ok_and(|(_, l)| l.end_cam == shared::EndCam::FinalKill);
        let deliver = cam.target.is_none_or(|p| !shared::bot_players::is_bot_peer(p))
            && !(cam.target.is_some() && final_kill_ending);

        let msg = KillCam {
            killer_name: cam.killer_name.clone(),
            samples: window,
            kill_index,
            bots: cam.bots.clone(),
            players: cam.players.clone(),
            best_play: false,
            final_kill: false,
        };
        if deliver {
            if let Err(e) =
                sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(targets))
            {
                error!("failed to send kill cam: {e:?}");
            }
        }

        // `FreeForAll`: remember this kill as the latest, and as the best play
        // if its window holds the most kills by one player so far (ties go to
        // the later one).
        if cam.target.is_some() {
            let m = ffa.0.entry(cam.lobby).or_default();
            let count = kills_in_window(&m.kills, cam.killer, cam.kill_seq);
            if m.best.as_ref().is_none_or(|(best, _)| count >= *best) {
                m.best = Some((count, msg.clone()));
            }
            m.last = Some(msg.clone());
        }

        // Track the match's best (highest-scoring) shot per lobby, so it can
        // be replayed for everyone right before `MatchOver`. 0-point shots
        // (a miss that still happened to graze a bot) don't count as a "play".
        if cam.points > 0 {
            let best = best_plays.0.entry(cam.lobby).or_insert(BestPlay {
                points: 0,
                msg: msg.clone(),
            });
            if cam.points >= best.points {
                best.points = cam.points;
                best.msg = msg;
            }
        }
        false
    });
}

/// Ends every lobby in [`EndingLobbies`] once none of its replays are still
/// waiting on their follow-through (or the wait ran out). It always waits a few
/// ticks first: the kill that ended the match reaches `queue_killcams` a tick
/// or two after the score that triggered the ending.
fn finish_endings(
    clock: Res<ReplayClock>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    pending: Res<PendingCams>,
    mut endings: ResMut<EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut best_plays: ResMut<BestPlays>,
    mut ffa: ResMut<FfaPlays>,
) {
    if endings.0.is_empty() {
        return;
    }
    let server = server.into_inner();
    let now = clock.0;
    let ready: Vec<Entity> = endings
        .0
        .iter()
        .filter(|(lobby, began)| {
            let waited = now.wrapping_sub(**began);
            waited >= END_SETTLE_TICKS && !pending.0.iter().any(|c| c.lobby == **lobby)
                || waited >= POST + 32
        })
        .map(|(e, _)| *e)
        .collect();
    for lobby_e in ready {
        endings.0.remove(&lobby_e);
        if let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) {
            if lobby.started {
                crate::lobby::end_match(
                    lobby_e,
                    &mut lobby,
                    server,
                    &mut sender,
                    &mut best_plays,
                    &mut ffa,
                );
            }
        }
    }
}

/// `seq` in `[lo, hi]`, tolerant of the `u64` counter wrapping.
fn in_range(seq: u64, lo: u64, hi: u64) -> bool {
    seq.wrapping_sub(lo) <= hi.wrapping_sub(lo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::EndCam;

    fn cam(name: &str) -> KillCam {
        KillCam {
            killer_name: name.into(),
            samples: Vec::new(),
            kill_index: 0,
            bots: Vec::new(),
            players: Vec::new(),
            best_play: false,
            final_kill: false,
        }
    }

    const A: PeerId = PeerId::Netcode(1);
    const B: PeerId = PeerId::Netcode(2);

    #[test]
    fn a_window_counts_only_that_killers_kills_inside_the_replay_window() {
        let t = |secs: f64| (secs * shared::TICK_HZ) as u64 + 1000;
        let kills = [(t(0.0), A), (t(1.0), A), (t(1.5), B), (t(2.5), A), (t(9.0), A)];
        // Anchored on A's kill at 2.5 s: the 0 s kill is 2.5 s back, the 1 s one 1.5 s.
        assert_eq!(kills_in_window(&kills, A, t(2.5)), 3);
        assert_eq!(kills_in_window(&kills, B, t(1.5)), 1);
        // A kill 9 s later has nothing else near it.
        assert_eq!(kills_in_window(&kills, A, t(9.0)), 1);
        // Just over the window back doesn't count.
        assert_eq!(kills_in_window(&[(1000, A), (1000 + PRE, A)], A, 1000 + PRE), 1);
        assert_eq!(kills_in_window(&[(1000, A), (1000 + PRE - 1, A)], A, 1000 + PRE - 1), 2);
    }

    #[test]
    fn the_end_cam_setting_picks_which_replay_is_taken_and_marks_it() {
        let lobby = Entity::from_raw(7);
        let mut plays = FfaPlays::default();
        let m = plays.0.entry(lobby).or_default();
        m.best = Some((3, cam("best")));
        m.last = Some(cam("last"));

        let last = plays.take(lobby, EndCam::FinalKill).expect("final kill");
        assert_eq!(last.killer_name, "last");
        assert!(last.best_play && last.final_kill);
        assert!(plays.take(lobby, EndCam::FinalKill).is_none(), "taking clears it");

        let m = plays.0.entry(lobby).or_default();
        m.best = Some((3, cam("best")));
        m.last = Some(cam("last"));
        let best = plays.take(lobby, EndCam::BestPlay).expect("best play");
        assert_eq!(best.killer_name, "best");
        assert!(best.best_play && !best.final_kill);
    }

    #[test]
    fn a_match_with_no_kills_has_nothing_to_replay() {
        let mut plays = FfaPlays::default();
        assert!(plays.take(Entity::from_raw(1), EndCam::BestPlay).is_none());
        plays.0.entry(Entity::from_raw(1)).or_default();
        assert!(plays.take(Entity::from_raw(1), EndCam::FinalKill).is_none());
    }

    #[test]
    fn ending_a_lobby_twice_keeps_the_first_tick() {
        let mut e = EndingLobbies::default();
        let l = Entity::from_raw(3);
        e.begin(l, 100);
        e.begin(l, 500);
        assert!(e.is_ending(l));
        assert_eq!(e.0[&l], 100);
    }
}
