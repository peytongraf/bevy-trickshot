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
//! buffered, so the final kill can actually be replayed: for `FreeForAll` that's
//! a fixed, announced freeze ([`shared::MATCH_END_FREEZE_SECS`]) during which
//! nothing counts.

use std::collections::VecDeque;

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{
    ActorSample, Bot, GameChannel, KillCam, KillCamActor, KillCamSample, KnifeSample, Lobby,
    PlayerId, PlayerInput, PlayerPose, ThrownKnife, ACTOR_STRIDE_TICKS,
};

use crate::bots::{BotHit, LobbyBot};
use crate::pvp::PlayerKilled;

/// Samples before / after the kill (at `TICK_HZ`).
const PRE: u64 = (shared::TICK_HZ as u64) * 3;
const POST: u64 = (shared::TICK_HZ * 1.5) as u64;
/// The longest stretch (first kill → last kill) a `FreeForAll` best play covers —
/// on top of the usual `PRE` before its first kill and `POST` after its last.
const MAX_BEST_SPAN: u64 = (shared::TICK_HZ as u64) * 8;
/// Ring-buffer capacity: enough to cut a maximum-length best play
/// (`PRE + MAX_BEST_SPAN + POST`, a touch over 12.5 s, ~830 small samples per
/// player) out of it when its last kill's follow-through lands.
const CAP: usize = (PRE + MAX_BEST_SPAN + POST + 32) as usize;

/// One player's rolling recording.
#[derive(Component, Default)]
pub struct ReplayBuffer {
    frames: VecDeque<(u64, KillCamSample)>,
}

/// One [`ACTOR_STRIDE_TICKS`]-spaced snapshot of a lobby: every bot and player in
/// it, so a replay can show them as they were.
struct ActorFrame {
    seq: u64,
    actors: Vec<ActorEntry>,
}

struct ActorEntry {
    /// Stable per actor (player peer bits / bot entity bits), to string a
    /// timeline together.
    key: u64,
    bot: bool,
    /// The player's peer, so the killer can be left out of their own replay.
    peer: Option<PeerId>,
    /// (`tick` is filled in when a window is cut.)
    sample: ActorSample,
}

/// Every started lobby's recent [`ActorFrame`]s — long enough to cover the
/// longest replay (`CAP`). Recorded once per lobby (not per replay), and a
/// replay only pays for the frames inside its own window.
#[derive(Resource, Default)]
pub(crate) struct ActorLog(std::collections::HashMap<Entity, VecDeque<ActorFrame>>);

/// Cut the bots' and other players' timelines for ticks `lo..=hi` out of one
/// lobby's log, leaving `exclude` (the killer) out. `tick` in each sample is
/// relative to `lo`, matching the replay's own frame numbering.
fn cut_actors(
    log: Option<&VecDeque<ActorFrame>>,
    lo: u64,
    hi: u64,
    exclude: PeerId,
) -> Vec<KillCamActor> {
    let Some(log) = log else {
        return Vec::new();
    };
    let mut order: Vec<u64> = Vec::new();
    let mut by_key: std::collections::HashMap<u64, KillCamActor> = std::collections::HashMap::new();
    for frame in log.iter().filter(|f| in_range(f.seq, lo, hi)) {
        let tick = frame.seq.wrapping_sub(lo) as u16;
        for a in frame.actors.iter().filter(|a| a.peer != Some(exclude)) {
            let actor = by_key.entry(a.key).or_insert_with(|| {
                order.push(a.key);
                KillCamActor {
                    bot: a.bot,
                    samples: Vec::new(),
                }
            });
            actor.samples.push(ActorSample { tick, ..a.sample });
        }
    }
    order.into_iter().filter_map(|k| by_key.remove(&k)).collect()
}

/// Monotonic tick counter for buffer bookkeeping (avoids `Tick` wrap math).
#[derive(Resource, Default)]
pub(crate) struct ReplayClock(pub(crate) u64);

struct PendingCam {
    killer: PeerId,
    killer_name: String,
    kill_seq: u64,
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
    /// The best play so far — see [`BestFfaPlay`].
    best: Option<BestFfaPlay>,
}

/// A `FreeForAll` best play: the busiest run of kills by one player that fits in
/// [`MAX_BEST_SPAN`], with its replay already cut out (`PRE` before the run's
/// first kill to `POST` after its last — so a run of 2 kills 2 s apart is a
/// short replay and 5 kills in 8 s an 8 s one). Only built when it beats the
/// previous best, so recording stays cheap.
struct BestFfaPlay {
    kills: u32,
    /// Ticks from the run's first kill to its last.
    span: u64,
    msg: KillCam,
}

/// Every `FreeForAll` lobby's [`FfaMatch`].
#[derive(Resource, Default)]
pub(crate) struct FfaPlays(std::collections::HashMap<Entity, FfaMatch>);

impl FfaPlays {
    /// Take (and clear) `lobby`'s replay for `cam`, if it has one.
    pub(crate) fn take(&mut self, lobby: Entity, cam: shared::EndCam) -> Option<KillCam> {
        let m = self.0.remove(&lobby)?;
        let mut msg = match cam {
            shared::EndCam::BestPlay => m.best?.msg,
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

/// One lobby's match ending.
pub(crate) struct Ending {
    /// The tick it began at.
    began: u64,
    /// `FreeForAll`: a fixed [`shared::MATCH_END_FREEZE_SECS`] freeze that the
    /// clients are told about ([`shared::MatchEnding`]), and during which
    /// hits, kills and bots are ignored ([`EndingLobbies::is_frozen`]).
    /// `Freestyle` just waits for its pending replays to flush.
    freeze: bool,
    announced: bool,
}

/// Lobbies whose match is over but which haven't been told yet. The last
/// kill's replay needs `POST` ticks of follow-through buffered before it can be
/// sent, so an ending waits ([`finish_endings`]) — and, meanwhile, the final
/// kill's own victim isn't sent a separate replay if the end-of-match one is
/// the final kill (they'd see it twice).
#[derive(Resource, Default)]
pub(crate) struct EndingLobbies(std::collections::HashMap<Entity, Ending>);

/// Ticks a frozen ending lasts — must outlast `POST` (the last kill's
/// follow-through) plus the settle time, or the final kill couldn't be replayed.
const FREEZE_TICKS: u64 = (shared::MATCH_END_FREEZE_SECS as f64 * shared::TICK_HZ) as u64;
const _: () = assert!(FREEZE_TICKS > POST + 8);

impl EndingLobbies {
    /// Start ending `lobby` (a no-op if it already is). `freeze` for
    /// `FreeForAll`.
    pub(crate) fn begin(&mut self, lobby: Entity, now: u64, freeze: bool) {
        self.0.entry(lobby).or_insert(Ending {
            began: now,
            freeze,
            announced: false,
        });
    }

    pub(crate) fn is_ending(&self, lobby: Entity) -> bool {
        self.0.contains_key(&lobby)
    }

    /// The lobby's match is over and nothing that happens now counts.
    pub(crate) fn is_frozen(&self, lobby: Entity) -> bool {
        self.0.get(&lobby).is_some_and(|e| e.freeze)
    }
}

/// The run of kills by `killer` that ends with the kill at tick `at`: every one
/// within [`MAX_BEST_SPAN`] before it (inclusive). Returns how many kills that
/// is and the tick of the earliest — so the run's real length is `at - first`,
/// shorter than the maximum if the kills are close together.
fn kill_run(kills: &[(u64, PeerId)], killer: PeerId, at: u64) -> (u32, u64) {
    let lo = at.wrapping_sub(MAX_BEST_SPAN);
    let mut count = 0;
    let mut first = at;
    for (seq, k) in kills {
        if *k == killer && in_range(*seq, lo, at) {
            count += 1;
            if at.wrapping_sub(*seq) > at.wrapping_sub(first) {
                first = *seq;
            }
        }
    }
    (count, first)
}

/// Cut the frames from `lo` to `hi` (inclusive, wrap-tolerant) out of a
/// replay buffer, with the index of the frame at `kill_seq` in the result.
fn cut_window(
    frames: &VecDeque<(u64, KillCamSample)>,
    lo: u64,
    hi: u64,
    kill_seq: u64,
) -> (Vec<KillCamSample>, u32) {
    let window: Vec<KillCamSample> = frames
        .iter()
        .filter(|(seq, _)| in_range(*seq, lo, hi))
        .map(|(_, s)| *s)
        .collect();
    let kill_index = frames
        .iter()
        .filter(|(seq, _)| in_range(*seq, lo, kill_seq))
        .count()
        .saturating_sub(1) as u32;
    (window, kill_index)
}

pub struct KillCamPlugin;

impl Plugin for KillCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplayClock>()
            .init_resource::<PendingCams>()
            .init_resource::<BestPlays>()
            .init_resource::<FfaPlays>()
            .init_resource::<EndingLobbies>()
            .init_resource::<ActorLog>()
            .add_systems(
                FixedUpdate,
                (
                    advance_clock,
                    attach_buffers,
                    record_frames,
                    record_actors,
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

/// One recorded frame from a player's input this tick.
fn sample_from_input(
    i: &PlayerInput,
    thrown_knives: [Option<KnifeSample>; shared::throwing_knife::MAX_KNIVES_PER_PLAYER],
) -> KillCamSample {
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
        buf.frames.push_back((clock.0, sample_from_input(i, thrown_knives)));
        while buf.frames.len() > CAP {
            buf.frames.pop_front();
        }
    }
}

/// Every [`ACTOR_STRIDE_TICKS`] ticks, note where every bot and player in each
/// started lobby is and what they're doing.
fn record_actors(
    clock: Res<ReplayClock>,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    bots: Query<(Entity, &Bot, &LobbyBot)>,
    mut log: ResMut<ActorLog>,
) {
    if clock.0 % ACTOR_STRIDE_TICKS != 0 {
        return;
    }
    // Only lobbies that are playing keep a log.
    log.0.retain(|e, _| lobbies.get(*e).is_ok_and(|(_, l)| l.started));
    let keep = CAP / ACTOR_STRIDE_TICKS as usize + 4;
    for (lobby_e, lobby) in lobbies.iter().filter(|(_, l)| l.started) {
        let mut actors: Vec<ActorEntry> = poses
            .iter()
            .filter(|(id, _)| lobby.has(id.0))
            .map(|(id, pose)| ActorEntry {
                key: id.0.to_bits(),
                bot: false,
                peer: Some(id.0),
                sample: ActorSample::from_pose(0, pose),
            })
            .collect();
        actors.extend(bots.iter().filter(|(_, _, lb)| lb.lobby == lobby_e).map(
            |(e, b, _)| ActorEntry {
                key: e.to_bits(),
                bot: true,
                peer: None,
                sample: ActorSample::from_bot(0, b),
            },
        ));
        let frames = log.0.entry(lobby_e).or_default();
        frames.push_back(ActorFrame {
            seq: clock.0,
            actors,
        });
        while frames.len() > keep {
            frames.pop_front();
        }
    }
}

fn queue_killcams(
    clock: Res<ReplayClock>,
    mut bot_hits: EventReader<BotHit>,
    mut player_kills: EventReader<PlayerKilled>,
    lobbies: Query<(Entity, &Lobby)>,
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

    // Queues one replay window for the killer's lobby (the bots and players in
    // it are cut from the `ActorLog` when it flushes). `target = None` broadcasts to
    // the whole lobby (a bot kill — a shared highlight); `Some(peer)` sends
    // it to just that one peer (a `FreeForAll` kill — only the victim
    // watches how they died).
    let mut queue_one =
        |killer: PeerId, points: u32, target: Option<PeerId>| {
            let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.has(killer)) else {
                return;
            };
            let name = lobby
                .members
                .iter()
                .find(|m| m.peer == killer)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Someone".to_string());
            if target.is_some() {
                ffa.0.entry(lobby_e).or_default().kills.push((clock.0, killer));
            }
            pending.0.push(PendingCam {
                killer,
                killer_name: name,
                kill_seq: clock.0,
                points,
                lobby: lobby_e,
                target,
            });
        };

    for (killer, _killed_bots, points) in &by_shooter {
        queue_one(*killer, *points, None);
    }
    for ev in player_kills.read() {
        queue_one(ev.killer, 0, Some(ev.victim));
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
    actor_log: Res<ActorLog>,
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
        let (window, kill_index) = cut_window(
            &buf.frames,
            cam.kill_seq.wrapping_sub(PRE),
            cam.kill_seq.wrapping_add(POST),
            cam.kill_seq,
        );

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
            actors: cut_actors(
                actor_log.0.get(&cam.lobby),
                cam.kill_seq.wrapping_sub(PRE),
                cam.kill_seq.wrapping_add(POST),
                cam.killer,
            ),
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

        // `FreeForAll`: remember this kill as the latest, and see whether the
        // run of kills it ends is the best play so far — the most kills by one
        // player within `MAX_BEST_SPAN`; among equals the shorter (denser) run,
        // then the later. Only a new best pays for cutting its (long) replay.
        if cam.target.is_some() {
            let m = ffa.0.entry(cam.lobby).or_default();
            let (kills, first) = kill_run(&m.kills, cam.killer, cam.kill_seq);
            let span = cam.kill_seq.wrapping_sub(first);
            let better = m
                .best
                .as_ref()
                .is_none_or(|b| kills > b.kills || (kills == b.kills && span <= b.span));
            if better {
                let (samples, kill_index) = cut_window(
                    &buf.frames,
                    first.wrapping_sub(PRE),
                    cam.kill_seq.wrapping_add(POST),
                    cam.kill_seq,
                );
                let (lo, hi) = (first.wrapping_sub(PRE), cam.kill_seq.wrapping_add(POST));
                m.best = Some(BestFfaPlay {
                    kills,
                    span,
                    msg: KillCam {
                        samples,
                        kill_index,
                        actors: cut_actors(actor_log.0.get(&cam.lobby), lo, hi, cam.killer),
                        ..msg.clone()
                    },
                });
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

    // Tell each frozen lobby's players the match is over, so their clients can
    // stop and show VICTORY / DEFEAT for exactly the freeze.
    for (lobby_e, ending) in endings.0.iter_mut() {
        if !ending.freeze || ending.announced {
            continue;
        }
        ending.announced = true;
        let Ok((_, lobby)) = lobbies.get(*lobby_e) else {
            continue;
        };
        let msg = shared::MatchEnding {
            winners: lobby.top_scorers(),
        };
        if let Err(e) =
            sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers()))
        {
            error!("failed to send match ending: {e:?}");
        }
    }

    let ready: Vec<Entity> = endings
        .0
        .iter()
        .filter(|(lobby, ending)| {
            let waited = now.wrapping_sub(ending.began);
            if ending.freeze {
                return waited >= FREEZE_TICKS;
            }
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
            actors: Vec::new(),
            best_play: false,
            final_kill: false,
        }
    }

    const A: PeerId = PeerId::Netcode(1);
    const B: PeerId = PeerId::Netcode(2);

    #[test]
    fn a_run_is_the_killers_kills_within_the_max_span_ending_at_this_one() {
        let t = |secs: f64| (secs * shared::TICK_HZ) as u64 + 1000;
        let kills = [(t(0.0), A), (t(1.0), A), (t(1.5), B), (t(2.5), A), (t(20.0), A)];
        // Three by A up to 2.5 s; the run started at 0 s, so 2.5 s long.
        assert_eq!(kill_run(&kills, A, t(2.5)), (3, t(0.0)));
        assert_eq!(kill_run(&kills, B, t(1.5)), (1, t(1.5)));
        // A kill long after the rest is a run of one, of length zero.
        assert_eq!(kill_run(&kills, A, t(20.0)), (1, t(20.0)));
        // Exactly the max span back still counts; a tick further doesn't.
        let at = 1000 + MAX_BEST_SPAN;
        assert_eq!(kill_run(&[(1000, A), (at, A)], A, at), (2, 1000));
        assert_eq!(kill_run(&[(999, A), (at, A)], A, at), (1, at));
    }

    #[test]
    fn a_long_best_play_can_be_cut_from_the_buffer() {
        // The buffer holds 12.5 s+ so `PRE + MAX_BEST_SPAN + POST` always fits.
        assert!(CAP as u64 >= PRE + MAX_BEST_SPAN + POST);
        let sample = sample_from_input(&PlayerInput::default(), [None; shared::throwing_knife::MAX_KNIVES_PER_PLAYER]);
        let frames: VecDeque<(u64, KillCamSample)> = (0..CAP as u64).map(|i| (i, sample)).collect();
        let last = CAP as u64 - 1 - POST;
        let first = last - MAX_BEST_SPAN;
        let (window, kill_index) = cut_window(&frames, first - PRE, last + POST, last);
        assert_eq!(window.len() as u64, PRE + MAX_BEST_SPAN + POST + 1);
        assert_eq!(kill_index as u64, PRE + MAX_BEST_SPAN);
    }

    #[test]
    fn the_end_cam_setting_picks_which_replay_is_taken_and_marks_it() {
        let lobby = Entity::from_raw(7);
        let mut plays = FfaPlays::default();
        let m = plays.0.entry(lobby).or_default();
        m.best = Some(BestFfaPlay { kills: 3, span: 0, msg: cam("best") });
        m.last = Some(cam("last"));

        let last = plays.take(lobby, EndCam::FinalKill).expect("final kill");
        assert_eq!(last.killer_name, "last");
        assert!(last.best_play && last.final_kill);
        assert!(plays.take(lobby, EndCam::FinalKill).is_none(), "taking clears it");

        let m = plays.0.entry(lobby).or_default();
        m.best = Some(BestFfaPlay { kills: 3, span: 0, msg: cam("best") });
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
        e.begin(l, 100, true);
        e.begin(l, 500, false);
        assert!(e.is_ending(l));
        assert_eq!(e.0[&l].began, 100);
        assert!(e.is_frozen(l), "the first begin's freeze setting stands");
        let free = Entity::from_raw(4);
        e.begin(free, 1, false);
        assert!(e.is_ending(free) && !e.is_frozen(free));
        assert!(!e.is_frozen(Entity::from_raw(9)));
    }

    fn entry(key: u64, bot: bool, peer: Option<PeerId>, x: f32, alive: bool) -> ActorEntry {
        ActorEntry {
            key,
            bot,
            peer,
            sample: ActorSample {
                tick: 0,
                pos: [x, 0.0, 0.0],
                yaw: 0.0,
                ads: 0,
                flags: if alive { ActorSample::ALIVE } else { 0 },
            },
        }
    }

    #[test]
    fn cutting_actors_gives_each_a_timeline_in_window_relative_ticks_without_the_killer() {
        // Frames every 4 ticks: A (the killer), B (walking +1 m per frame, dies
        // at the 4th frame) and a bot standing still.
        let log: VecDeque<ActorFrame> = (0..10u64)
            .map(|i| ActorFrame {
                seq: 1000 + i * ACTOR_STRIDE_TICKS,
                actors: vec![
                    entry(1, false, Some(A), 50.0, true),
                    entry(2, false, Some(B), i as f32, i < 4),
                    entry(3, true, None, 9.0, true),
                ],
            })
            .collect();
        // A window over frames 2..=6 (ticks 1008..=1024), killer A.
        let actors = cut_actors(Some(&log), 1008, 1024, A);
        assert_eq!(actors.len(), 2, "the killer is left out");
        let (b, bot) = (&actors[0], &actors[1]);
        assert!(!b.bot && bot.bot);
        let ticks: Vec<u16> = b.samples.iter().map(|s| s.tick).collect();
        assert_eq!(ticks, vec![0, 4, 8, 12, 16]);
        // Moving, then dead from the 4th frame on (the frame at tick 12).
        assert_eq!(b.samples[0].pos[0], 2.0);
        assert!(b.samples[1].has(ActorSample::ALIVE) && !b.samples[2].has(ActorSample::ALIVE));
        // Nothing logged (or no lobby log at all) is just no actors.
        assert!(cut_actors(Some(&log), 5000, 6000, A).is_empty());
        assert!(cut_actors(None, 1008, 1024, A).is_empty());
    }
}
