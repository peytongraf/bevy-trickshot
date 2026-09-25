//! Lobby lifecycle: create / join / leave / start, all driven by client
//! triggers. Lobbies are plain entities carrying a replicated [`shared::Lobby`]
//! component (replicated to everyone so every client's browser is live).
//!
//! `StartGame` (from the leader) flips `started` and spawns one replicated
//! player entity per member, scoped with `NetworkTarget::Only` to that lobby's
//! members so lobbies don't see each other.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::{bot_peer, BOT_NAMES, MAX_BOTS};
use shared::bots::rand01;
use shared::{
    AddBots, AssetsReady, ClearBots, CreateLobby, EndGame, GameChannel, GameMode, JoinLobby,
    LeaveLobby, Lobby, LobbyError, LobbyMember, MapId, MatchOver, PlayerId, PlayerInput,
    PlayerName, PlayerPose, SetEndCam, SetGameMode, SetKillLimit, SetBotsPassive, SetMap, SetPaused, SetTimeLimit,
    StartGame,
};

use crate::ai::{BotBrain, NextBotId};

/// Bounds on the leader-set match length (seconds) — 1 to 45 minutes.
const MIN_TIME_LIMIT: u32 = 60;
const MAX_TIME_LIMIT: u32 = 45 * 60;
/// Default match length for a fresh lobby (seconds).
const DEFAULT_TIME_LIMIT: u32 = 5 * 60;

/// Bounds on the leader-set `FreeForAll` kill limit.
pub const MIN_KILL_LIMIT: u32 = 5;
pub const MAX_KILL_LIMIT: u32 = 100;
/// Default kill limit for a fresh lobby.
const DEFAULT_KILL_LIMIT: u32 = 30;

/// Tags a replicated in-world player entity with the lobby it belongs to, so the
/// session's players can be found again (rescoping / cleanup).
#[derive(Component)]
pub struct LobbyPlayer {
    pub lobby: Entity,
}

/// Max *real* players per lobby (bots have their own cap, `MAX_BOTS`).
const MAX_MEMBERS: usize = 8;

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_create)
            .add_observer(on_join)
            .add_observer(on_leave)
            .add_observer(on_start)
            .add_observer(on_assets_ready)
            .add_observer(on_end_game)
            .add_observer(on_set_paused)
            .add_observer(on_set_bots_passive)
            .add_observer(on_set_time_limit)
            .add_observer(on_set_game_mode)
            .add_observer(on_set_map)
            .add_observer(on_set_kill_limit)
            .add_observer(on_set_end_cam)
            .add_observer(on_add_bots)
            .add_observer(on_clear_bots)
            .add_observer(on_disconnect)
            .add_systems(Update, tick_match_clock);
    }
}

// --- helpers --------------------------------------------------------------

/// Remove `peer` from every lobby (optionally skipping `except`), promoting a
/// new leader or despawning the lobby as needed. In-world player entities for
/// the departing peer (and for the whole lobby, if it empties) are despawned
/// too — `get_entity` guards against a double-despawn when `ControlledBy`
/// already cleaned one up on disconnect.
fn remove_peer(
    peer: PeerId,
    except: Option<Entity>,
    lobbies: &mut Query<(Entity, &mut Lobby)>,
    players: &Query<(Entity, &PlayerId, &LobbyPlayer)>,
    commands: &mut Commands,
) {
    for (entity, mut lobby) in lobbies.iter_mut() {
        if Some(entity) == except || !lobby.has(peer) {
            continue;
        }
        lobby.members.retain(|m| m.peer != peer);

        let despawn = |commands: &mut Commands, e: Entity| {
            if let Ok(mut ec) = commands.get_entity(e) {
                ec.despawn();
            }
        };

        // A lobby with nobody real left is over, whatever bots remain in it.
        if lobby.real_count() == 0 {
            for (pe, _, _) in players.iter().filter(|(_, _, lp)| lp.lobby == entity) {
                despawn(commands, pe);
            }
            despawn(commands, entity);
            info!("lobby {entity:?} is empty — despawned");
        } else {
            for (pe, pid, lp) in players.iter() {
                if lp.lobby == entity && pid.0 == peer {
                    despawn(commands, pe);
                }
            }
            if lobby.leader == peer {
                // (`real_count() > 0` here, so there is a real member to promote.)
                lobby.leader = lobby.real_peers()[0];
                // Only the leader can resume, so don't leave the new one
                // stuck in a pause they never chose.
                lobby.paused = false;
                info!("lobby {entity:?}: leader left, promoted {:?}", lobby.leader);
            }
        }
    }
}

fn reject(sender: &mut ServerMultiMessageSender, server: &Server, peer: PeerId, reason: &str) {
    let msg = LobbyError {
        reason: reason.to_string(),
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
        error!("failed to send LobbyError to {peer:?}: {e:?}");
    }
}

// --- observers ----------------------------------------------------------

fn on_create(
    trigger: Trigger<RemoteTrigger<CreateLobby>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let ev = &trigger.trigger;

    // Creating implies leaving whatever lobby they were in.
    remove_peer(peer, None, &mut lobbies, &players, &mut commands);

    let name = if ev.name.trim().is_empty() {
        format!("{}'s lobby", ev.player_name)
    } else {
        ev.name.trim().to_string()
    };

    let entity = commands
        .spawn((
            Name::from("Lobby"),
            Lobby {
                name,
                leader: peer,
                mode: GameMode::default(),
                map: MapId::default(),
                started: false,
                time_limit_secs: DEFAULT_TIME_LIMIT,
                time_left_secs: DEFAULT_TIME_LIMIT,
                kill_limit: DEFAULT_KILL_LIMIT,
                end_cam: shared::EndCam::default(),
                round: 0,
                enemies_left: 0,
                paused: false,
                bots_passive: false,
                members: vec![LobbyMember {
                    peer,
                    name: ev.player_name.clone(),
                    score: 0,
                    loaded: false,
                    bot: None,
                    kills: 0,
                    perks: Vec::new(),
                }],
            },
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();
    info!("{peer:?} created lobby {entity:?}");
}

fn on_join(
    trigger: Trigger<RemoteTrigger<JoinLobby>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let target = trigger.trigger.lobby;
    let player_name = trigger.trigger.player_name.clone();

    // Validate against the target lobby without holding a mutable borrow.
    let Ok((_, lobby)) = lobbies.get(target) else {
        reject(&mut sender, &server, peer, "That lobby no longer exists.");
        return;
    };
    if lobby.started {
        reject(&mut sender, &server, peer, "That game has already started.");
        return;
    }
    if lobby.real_count() >= MAX_MEMBERS && !lobby.has(peer) {
        reject(&mut sender, &server, peer, "That lobby is full.");
        return;
    }

    remove_peer(peer, Some(target), &mut lobbies, &players, &mut commands);

    let Ok((_, mut lobby)) = lobbies.get_mut(target) else {
        return;
    };
    if !lobby.has(peer) {
        lobby.members.push(LobbyMember {
            peer,
            name: player_name,
            score: 0,
            loaded: false,
            bot: None,
            kills: 0,
            perks: Vec::new(),
        });
        info!("{peer:?} joined lobby {target:?}");
    }
}

fn on_leave(
    trigger: Trigger<RemoteTrigger<LeaveLobby>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    remove_peer(trigger.from, None, &mut lobbies, &players, &mut commands);
    info!("{:?} left their lobby", trigger.from);
}

fn on_start(
    trigger: Trigger<RemoteTrigger<StartGame>>,
    time: Res<Time>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    clients: Query<(Entity, &RemoteId), With<ClientOf>>,
    old_players: Query<(Entity, &LobbyPlayer)>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let Some((lobby_entity, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.leader == peer && !l.started)
    else {
        return;
    };

    lobby.started = true;
    lobby.paused = false;
    lobby.time_left_secs = lobby.time_limit_secs;
    // (`crate::zombies` starts round 1.)
    lobby.round = 0;
    lobby.enemies_left = 0;
    for m in &mut lobby.members {
        m.score = 0;
        m.kills = 0;
        m.perks.clear();
        // A bot has no client to load anything.
        m.loaded = m.bot.is_some();
    }
    let mode = lobby.mode;
    let members: Vec<shared::LobbyMember> = lobby.members.clone();
    info!(
        "lobby {lobby_entity:?} started by {peer:?} with {} member(s) ({} bot), {}s limit, mode {mode:?}",
        members.len(),
        lobby.bot_count(),
        lobby.time_limit_secs,
    );

    // A previous match in this lobby may have left its player entities behind
    // (bots included) — start from a clean slate rather than doubling up.
    for (e, lp) in &old_players {
        if lp.lobby == lobby_entity {
            if let Ok(mut ec) = commands.get_entity(e) {
                ec.despawn();
            }
        }
    }

    // Replication only ever targets real clients — there's nobody behind a bot.
    let real: Vec<PeerId> = members
        .iter()
        .filter(|m| m.bot.is_none())
        .map(|m| m.peer)
        .collect();
    // `FreeForAll` spreads spawns out (see `shared::spawns::spawn_point`),
    // each one avoiding every spot already handed out this start.
    let mut taken_spawns: Vec<Vec3> = Vec::new();
    for member in &members {
        let owner = if member.bot.is_some() {
            None
        } else {
            let Some((owner, _)) = clients.iter().find(|(_, rid)| rid.0 == member.peer) else {
                warn!(
                    "no client link for {:?}; skipping player spawn",
                    member.peer
                );
                continue;
            };
            Some(owner)
        };
        let others: Vec<PeerId> = real.iter().copied().filter(|p| *p != member.peer).collect();

        // Where they start. A map with hand-placed spawn points (Shipment) puts
        // *everyone* — players and bots, either mode — on one of them, facing the
        // way it says; otherwise `FreeForAll` spreads them over a ring, and
        // `Freestyle` leaves a real player where their client already is.
        let designated = shared::spawns::designated_spawns(lobby.map).is_some();
        let spawn = (designated || mode != GameMode::Freestyle).then(|| {
            let seed = time.elapsed().as_nanos() as u64
                ^ member.peer.to_bits()
                ^ lobby_entity.to_bits();
            let (pos, yaw) = shared::spawns::spawn_point(seed, &taken_spawns, lobby.map);
            taken_spawns.push(pos);
            (pos, yaw)
        });
        // (`spawn`'s position is on the ground; the pose holds the eye.)
        let pose = match spawn {
            Some((pos, yaw)) => PlayerPose {
                translation: pos + Vec3::Y * crate::sim::EYE_HEIGHT,
                yaw,
                ..default()
            },
            None => PlayerPose::default(),
        };
        // Real clients move their own rig, so tell them where to stand — now,
        // not after the usual respawn delay.
        if let (Some((pos, yaw)), true) = (spawn, designated && member.bot.is_none()) {
            let msg = shared::PlayerRespawn {
                pos: pos.to_array(),
                yaw,
                immediate: true,
            };
            if let Err(e) =
                sender.send::<_, GameChannel>(&msg, &server, &NetworkTarget::Single(member.peer))
            {
                error!("failed to send start spawn to {:?}: {e:?}", member.peer);
            }
        }

        let mut ec = commands.spawn((
            Name::from(if member.bot.is_some() { "BotPlayer" } else { "Player" }),
            LobbyPlayer {
                lobby: lobby_entity,
            },
            PlayerId(member.peer),
            PlayerName(member.name.clone()),
            pose,
            ActionState::<PlayerInput>::default(),
            Replicate::to_clients(NetworkTarget::Only(real.clone())),
            InterpolationTarget::to_clients(NetworkTarget::Only(others)),
        ));
        match (owner, member.bot) {
            // A real player: predicted by their own client, driven by its input.
            (Some(owner), _) => {
                ec.insert((
                    PredictionTarget::to_clients(NetworkTarget::Single(member.peer)),
                    ControlledBy {
                        owner,
                        lifetime: Lifetime::SessionBased,
                    },
                ));
            }
            // A bot: driven by `ai::drive_bots`, standing where `pose` put it
            // (the pose holds the eye position; the brain wants the feet).
            (None, Some(difficulty)) => {
                let (feet, yaw) = spawn.unwrap_or((Vec3::ZERO, 0.0));
                let seed = time.elapsed().as_nanos() as u64 ^ member.peer.to_bits();
                ec.insert(BotBrain::new(difficulty, feet, seed).facing(yaw));
            }
            (None, None) => {}
        }
        // Health + combat state for every player in either mode: falls hurt in
        // Freestyle too, and `FreeForAll` shots take health off the same bar.
        ec.insert((
            crate::pvp::PlayerCombat::default(),
            shared::PlayerHealth(shared::health::FULL_HEALTH),
        ));
        let entity = ec.id();
        info!("  spawned player {entity:?} for {:?}", member.peer);
    }
}

/// A client reports it's finished loading this match's assets — see
/// [`shared::AssetsReady`]. Marks that member `loaded` in whichever started
/// lobby they're in; a stray report from a lobby that hasn't started (or
/// isn't theirs) is simply ignored.
fn on_assets_ready(trigger: Trigger<RemoteTrigger<AssetsReady>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    for mut lobby in &mut lobbies {
        if lobby.started && lobby.has(peer) {
            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                m.loaded = true;
            }
            break;
        }
    }
}

/// The leader ends the game for the whole party: despawn every player entity in
/// the session and disband the lobby, so all members fall back to the main menu
/// (`cull_orphan_bots` then drops the lobby's bots). Ignored for non-leaders.
fn on_end_game(
    trigger: Trigger<RemoteTrigger<EndGame>>,
    lobbies: Query<(Entity, &Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let Some((lobby_entity, _)) = lobbies.iter().find(|(_, l)| l.leader == peer) else {
        return;
    };

    let despawn = |commands: &mut Commands, e: Entity| {
        if let Ok(mut ec) = commands.get_entity(e) {
            ec.despawn();
        }
    };
    for (pe, _, _) in players.iter().filter(|(_, _, lp)| lp.lobby == lobby_entity) {
        despawn(&mut commands, pe);
    }
    despawn(&mut commands, lobby_entity);
    info!("{peer:?} ended the game for lobby {lobby_entity:?} (leave with party)");
}

/// The leader pauses / resumes their running game for the whole party. Not
/// once the match is ending (nothing to pause — it's already frozen).
fn on_set_paused(
    trigger: Trigger<RemoteTrigger<SetPaused>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
) {
    let peer = trigger.from;
    let paused = trigger.trigger.paused;
    let Some((lobby_e, mut lobby)) = lobbies.iter_mut().find(|(_, l)| l.leader == peer && l.started) else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused == paused {
        return;
    }
    lobby.paused = paused;
    info!("lobby {lobby_e:?} {} by {peer:?}", if paused { "paused" } else { "resumed" });
}

/// Debug: the leader stops (or lets) their lobby's bots fire, any time.
fn on_set_bots_passive(trigger: Trigger<RemoteTrigger<SetBotsPassive>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    let passive = trigger.trigger.passive;
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer) {
        if lobby.bots_passive != passive {
            lobby.bots_passive = passive;
            info!("lobby bots {} by {peer:?} (debug)", if passive { "made passive" } else { "allowed to attack" });
        }
    }
}

/// The leader picks the match length while the lobby is still waiting.
fn on_set_time_limit(
    trigger: Trigger<RemoteTrigger<SetTimeLimit>>,
    mut lobbies: Query<&mut Lobby>,
) {
    let peer = trigger.from;
    let secs = trigger
        .trigger
        .secs
        .clamp(MIN_TIME_LIMIT, MAX_TIME_LIMIT);
    if let Some(mut lobby) = lobbies
        .iter_mut()
        .find(|l| l.leader == peer && !l.started)
    {
        lobby.time_limit_secs = secs;
        lobby.time_left_secs = secs;
        info!("lobby time limit set to {secs}s by {peer:?}");
    }
}

/// The leader picks the lobby's game mode while it's still waiting.
fn on_set_game_mode(trigger: Trigger<RemoteTrigger<SetGameMode>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    let mode = trigger.trigger.mode;
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) {
        lobby.mode = mode;
        // Bots only exist in `FreeForAll`.
        if mode != GameMode::FreeForAll {
            lobby.members.retain(|m| m.bot.is_none());
        }
        info!("lobby mode set to {mode:?} by {peer:?}");
    }
}

/// Add up to `count` bots at `difficulty` to `lobby`, trimmed to fit
/// [`MAX_BOTS`] in total; returns how many were actually added. Each gets a
/// fake peer id (from `next_id`, which advances) and a random name from
/// [`BOT_NAMES`] that nobody else in the lobby has.
fn add_bots(
    lobby: &mut Lobby,
    count: usize,
    difficulty: shared::bot_players::BotDifficulty,
    next_id: &mut u64,
    seed: u64,
) -> usize {
    let room = MAX_BOTS.saturating_sub(lobby.bot_count());
    let count = count.min(room);
    for n in 0..count {
        let free: Vec<&str> = BOT_NAMES
            .iter()
            .copied()
            .filter(|name| !lobby.members.iter().any(|m| m.name == *name))
            .collect();
        let name = if free.is_empty() {
            format!("Bot {}", *next_id)
        } else {
            let roll = rand01(seed ^ (*next_id).wrapping_mul(0x9e37_79b9) ^ (n as u64) << 40);
            free[(roll * free.len() as f32) as usize % free.len()].to_string()
        };
        lobby.members.push(LobbyMember {
            peer: bot_peer(*next_id),
            name,
            score: 0,
            loaded: true,
            bot: Some(difficulty),
            kills: 0,
            perks: Vec::new(),
        });
        *next_id += 1;
    }
    count
}

/// The leader adds bots to a `FreeForAll` lobby that's still waiting: `count`
/// of them at one difficulty (call it again for another difficulty) — see
/// [`add_bots`].
fn on_add_bots(
    trigger: Trigger<RemoteTrigger<AddBots>>,
    time: Res<Time>,
    mut next: ResMut<NextBotId>,
    mut lobbies: Query<&mut Lobby>,
) {
    let peer = trigger.from;
    let req = &trigger.trigger;
    let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) else {
        return;
    };
    if lobby.mode != GameMode::FreeForAll {
        return;
    }
    let seed = time.elapsed().as_nanos() as u64;
    let added = add_bots(&mut lobby, req.count as usize, req.difficulty, &mut next.0, seed);
    info!(
        "{peer:?} added {added} {:?} bot(s) — {} in the lobby",
        req.difficulty,
        lobby.bot_count()
    );
}

/// The leader removes every bot from a waiting lobby.
fn on_clear_bots(trigger: Trigger<RemoteTrigger<ClearBots>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) {
        lobby.members.retain(|m| m.bot.is_none());
        info!("{peer:?} cleared the lobby's bots");
    }
}

/// The leader picks the lobby's map while it's still waiting.
fn on_set_map(trigger: Trigger<RemoteTrigger<SetMap>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    let map = trigger.trigger.map;
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) {
        lobby.map = map;
        info!("lobby map set to {map:?} by {peer:?}");
    }
}

/// The leader picks `FreeForAll`'s kill limit while the lobby is still waiting.
fn on_set_kill_limit(trigger: Trigger<RemoteTrigger<SetKillLimit>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    let kills = trigger.trigger.kills.clamp(MIN_KILL_LIMIT, MAX_KILL_LIMIT);
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) {
        lobby.kill_limit = kills;
        info!("lobby kill limit set to {kills} by {peer:?}");
    }
}

/// The leader picks what a `FreeForAll` match replays at the end while the
/// lobby is still waiting.
fn on_set_end_cam(trigger: Trigger<RemoteTrigger<SetEndCam>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) {
        lobby.end_cam = trigger.trigger.cam;
        info!("lobby end cam set to {:?} by {peer:?}", lobby.end_cam);
    }
}

/// Declare the top scorer (or top killer, in `FreeForAll`) the winner, tell
/// everyone, and end the match — `started` flipping false drops the clients
/// back to the lobby room. Shared by [`tick_match_clock`] (time ran out) and
/// [`crate::pvp::check_kill_limit`] (someone hit the kill limit first).
pub(crate) fn end_match(
    lobby_e: Entity,
    lobby: &mut Lobby,
    server: &Server,
    sender: &mut ServerMultiMessageSender,
    best_plays: &mut crate::killcam::BestPlays,
    ffa_plays: &mut crate::killcam::FfaPlays,
) {
    let (winner_name, winner_score) = lobby
        .members
        .iter()
        .max_by_key(|m| m.score)
        .map(|m| (m.name.clone(), m.score))
        .unwrap_or_default();
    let targets: Vec<PeerId> = lobby.real_peers();

    // Replay the match's best (highest-scoring) shot for everyone before
    // the results screen. `GameChannel` is unordered, so `MatchOver`
    // below carries an explicit `best_play_sent` flag rather than relying
    // on this being received first — the client holds the results screen
    // off until it's actually seen the flagged replay play out (see
    // `net::flush_pending_match_end`).
    // `FreeForAll` replays whatever the leader picked (the match's best play,
    // or its final kill), for everyone whoever won; `Freestyle` its
    // best-scoring shot.
    let mut best_play = match lobby.mode {
        GameMode::FreeForAll => ffa_plays.take(lobby_e, lobby.end_cam),
        GameMode::Freestyle => best_plays.take(lobby_e),
        // No replays in `Zombies` — straight to the results.
        GameMode::Zombies => None,
    };
    if let Some(best) = &mut best_play {
        best.best_play = true;
        if let Err(e) =
            sender.send::<_, GameChannel>(best, server, &NetworkTarget::Only(targets.clone()))
        {
            error!("failed to broadcast best play: {e:?}");
        }
    }

    let msg = MatchOver {
        winner_name: winner_name.clone(),
        winner_score,
        best_play_sent: best_play.is_some(),
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(targets)) {
        error!("failed to broadcast match result: {e:?}");
    }
    lobby.started = false;
    lobby.paused = false;
    info!("match over — {winner_name} wins with {winner_score}");
}

/// Count every started lobby's clock down one second at a time; at zero, end
/// the match (see [`end_match`], reached via `killcam::EndingLobbies`).
fn tick_match_clock(
    time: Res<Time>,
    clock: Res<crate::killcam::ReplayClock>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut endings: ResMut<crate::killcam::EndingLobbies>,
    mut acc: Local<f32>,
) {
    *acc += time.delta_secs();
    if *acc < 1.0 {
        return;
    }
    *acc -= 1.0;

    for (lobby_e, mut lobby) in &mut lobbies {
        // `Zombies` has no clock: it lasts until someone dies.
        if !lobby.started || lobby.paused || lobby.time_left_secs == 0 || lobby.mode == GameMode::Zombies {
            continue;
        }
        lobby.time_left_secs -= 1;
        if lobby.time_left_secs > 0 {
            continue;
        }
        endings.begin(lobby_e, clock.0, lobby.mode == GameMode::FreeForAll);
    }
}

fn on_disconnect(
    trigger: Trigger<OnAdd, Disconnected>,
    clients: Query<&RemoteId, With<ClientOf>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    let Ok(peer) = clients.get(trigger.target()) else {
        return;
    };
    remove_peer(peer.0, None, &mut lobbies, &players, &mut commands);
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::bot_players::{is_bot_peer, BotDifficulty};

    fn lobby() -> Lobby {
        Lobby {
            name: "test".into(),
            leader: PeerId::Netcode(1),
            mode: GameMode::FreeForAll,
            map: MapId::default(),
            started: false,
            time_limit_secs: 300,
            time_left_secs: 300,
            kill_limit: 30,
            end_cam: shared::EndCam::default(),
            round: 0,
            enemies_left: 0,
            paused: false,
            bots_passive: false,
            members: vec![LobbyMember {
                peer: PeerId::Netcode(1),
                name: "Host".into(),
                score: 0,
                loaded: false,
                bot: None,
                kills: 0,
                perks: Vec::new(),
            }],
        }
    }

    #[test]
    fn different_counts_and_difficulties_stack() {
        let mut l = lobby();
        let mut next = 0;
        assert_eq!(add_bots(&mut l, 4, BotDifficulty::Recruit, &mut next, 1), 4);
        assert_eq!(add_bots(&mut l, 5, BotDifficulty::Veteran, &mut next, 2), 5);
        assert_eq!(l.bot_count(), 9);
        let recruits = l.members.iter().filter(|m| m.bot == Some(BotDifficulty::Recruit)).count();
        let veterans = l.members.iter().filter(|m| m.bot == Some(BotDifficulty::Veteran)).count();
        assert_eq!((recruits, veterans), (4, 5));
    }

    #[test]
    fn the_lobby_never_holds_more_than_twenty_bots() {
        let mut l = lobby();
        let mut next = 0;
        assert_eq!(add_bots(&mut l, 15, BotDifficulty::Regular, &mut next, 3), 15);
        // Only 5 fit.
        assert_eq!(add_bots(&mut l, 10, BotDifficulty::Hardened, &mut next, 4), 5);
        assert_eq!(l.bot_count(), MAX_BOTS);
        assert_eq!(add_bots(&mut l, 1, BotDifficulty::Recruit, &mut next, 5), 0);
    }

    #[test]
    fn every_bot_gets_a_unique_name_from_the_list_all_twenty_of_them() {
        let mut l = lobby();
        let mut next = 0;
        add_bots(&mut l, 20, BotDifficulty::Recruit, &mut next, 9);
        let mut names: Vec<&str> = l
            .members
            .iter()
            .filter(|m| m.bot.is_some())
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names.len(), 20);
        assert!(names.iter().all(|n| BOT_NAMES.contains(n)));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 20, "a name was used twice");
    }

    #[test]
    fn a_bot_never_takes_a_name_a_real_player_has() {
        let mut l = lobby();
        l.members[0].name = "Alex".into();
        let mut next = 0;
        add_bots(&mut l, 19, BotDifficulty::Recruit, &mut next, 11);
        assert_eq!(l.members.iter().filter(|m| m.name == "Alex").count(), 1);
    }

    #[test]
    fn bots_have_fake_peers_and_are_left_out_of_real_peers() {
        let mut l = lobby();
        let mut next = 0;
        add_bots(&mut l, 3, BotDifficulty::Regular, &mut next, 13);
        assert_eq!(l.real_peers(), vec![PeerId::Netcode(1)]);
        assert_eq!(l.real_count(), 1);
        assert!(l.members.iter().filter(|m| m.bot.is_some()).all(|m| is_bot_peer(m.peer)));
        // Bots count as being in the lobby (`Lobby::has`), so shots etc. still see them.
        assert!(l.has(bot_peer(0)));
    }
}
