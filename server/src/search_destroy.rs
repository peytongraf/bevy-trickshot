//! `SearchAndDestroy` (`shared::search_destroy`), the server's side: the
//! rounds — held at the spawns for the pre-round countdown, then the round
//! clock — the bomb (carried, dropped where its carrier dies and picked up
//! by any attacker walking over it, planted, burning, defused or going
//! off), who wins each round and the match, and bringing everyone back at
//! their team's spawn for the next one. It all lives on the lobby
//! ([`Lobby::snd`]) for everyone's HUD; deaths are `crate::pvp`'s (which
//! keeps the dead down until the next round, and stops friendly fire).
//!
//! Its bots (`crate::ai`) are told where to go ([`crate::ai::BotBrain`]'s
//! `objective`): the attackers to this round's site (or the dropped bomb),
//! the defenders split between the two — and, once it's planted, everyone
//! to the bomb. A bot standing where it can plant or defuse does.
//!
//! Per game: a lobby that isn't running a `SearchAndDestroy` game has its
//! run dropped, and [`Lobby::snd`] is cleared whenever a game starts
//! (`crate::lobby` — it's kept once one's over, for the results screen).

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::has_no_client;
use shared::bots::rand01;
use shared::search_destroy::{
    attackers_in, layout, site_at, within, Bomb, Phase, RoundEnd, Site, Team, DEFUSE_RADIUS, DEFUSE_SECS, FUSE_SECS,
    MAX_ROUNDS_TO_WIN, MIN_ROUNDS_TO_WIN, PICKUP_RADIUS, PLANT_SECS, PRE_ROUND_SECS, ROUND_OVER_SECS, ROUND_SECS,
    SPAWN_SPREAD,
};
use shared::{GameChannel, GameMode, Lobby, MapId, PlayerId, PlayerInput, PlayerPose, PlayerRespawn, SndBombExploded};

use crate::ai::BotBrain;
use crate::lobby::LobbyPlayer;
use crate::nav::NavGraphs;
use crate::pvp::{HitCause, PlayerCombat, PlayerHit};
use crate::sim::EYE_HEIGHT;

/// One running game's state the clients don't need.
struct Run {
    /// When the current phase's clock runs out (`Time::elapsed_secs`).
    ends_at: f32,
    /// Who's planting or defusing, and since when.
    using: Option<(PeerId, f32)>,
    /// Who planted the bomb (credited with what it kills).
    planter: Option<PeerId>,
    /// The site this round's attacking bots go for.
    target_site: Site,
}

#[derive(Resource, Default)]
struct Runs(HashMap<Entity, Run>);

pub struct SearchDestroyPlugin;

impl Plugin for SearchDestroyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Runs>()
            .add_observer(on_set_team)
            .add_observer(on_set_snd_rounds)
            .add_systems(FixedUpdate, run_rounds.after(crate::pvp::apply_player_hits));
    }
}

/// Where someone on the attacking (or defending) side starts a round on
/// `map`: somewhere near their team's spawn, on its floor, facing the
/// sites.
pub(crate) fn spawn_spot(navs: &NavGraphs, map: MapId, attacking: bool, seed: u64) -> (Vec3, f32) {
    let l = layout(map);
    let at = if attacking { l.attack_spawn } else { l.defend_spawn };
    let spot = navs.graph(map).random_spot_near_level(at, SPAWN_SPREAD, seed).unwrap_or(at);
    (spot, l.facing(spot))
}

/// A member picks their team, any time the lobby's waiting.
fn on_set_team(trigger: Trigger<RemoteTrigger<shared::SetTeam>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    let team = trigger.trigger.team;
    let Some(mut lobby) = lobbies.iter_mut().find(|l| l.has(peer) && !l.started) else {
        return;
    };
    if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer && m.team != team) {
        m.team = team;
        info!("{peer:?} joined the {} team", team.label());
    }
}

/// The leader sets how many rounds a team needs to win, while the lobby's
/// waiting.
fn on_set_snd_rounds(trigger: Trigger<RemoteTrigger<shared::SetSndRounds>>, mut lobbies: Query<&mut Lobby>) {
    let peer = trigger.from;
    let rounds = trigger.trigger.rounds.clamp(MIN_ROUNDS_TO_WIN, MAX_ROUNDS_TO_WIN);
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer && !l.started) {
        if lobby.snd_rounds_to_win != rounds {
            lobby.snd_rounds_to_win = rounds;
            info!("lobby rounds to win set to {rounds} by {peer:?}");
        }
    }
}

/// One player in the game, as the rounds see them.
struct Body {
    peer: PeerId,
    team: Team,
    alive: bool,
    feet: Vec3,
    /// Holding interact (a player), or standing where it can be used (a bot).
    using: bool,
}

type Players<'w, 's> = Query<
    'w,
    's,
    (
        &'static PlayerId,
        &'static LobbyPlayer,
        &'static PlayerPose,
        &'static mut PlayerCombat,
        Option<&'static ActionState<PlayerInput>>,
        Option<&'static mut BotBrain>,
    ),
>;

#[allow(clippy::too_many_arguments)]
fn run_rounds(
    time: Res<Time>,
    clock: Res<crate::killcam::ReplayClock>,
    navs: Res<NavGraphs>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut endings: ResMut<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut players: Players,
    mut hits: EventWriter<PlayerHit>,
    mut runs: ResMut<Runs>,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    runs.0.retain(|e, _| {
        lobbies
            .get(*e)
            .is_ok_and(|(_, l)| l.started && l.mode == GameMode::SearchAndDestroy)
    });

    for (lobby_e, mut lobby) in &mut lobbies {
        if !lobby.started || lobby.mode != GameMode::SearchAndDestroy {
            continue;
        }
        // The match is over: everything stays as it is.
        if endings.is_ending(lobby_e) {
            continue;
        }
        let run = runs.0.entry(lobby_e).or_insert(Run {
            ends_at: now,
            using: None,
            planter: None,
            target_site: Site::A,
        });
        // Paused: hold the clocks (and any plant / defuse) where they are.
        if lobby.paused {
            run.ends_at += dt;
            if let Some((_, since)) = run.using.as_mut() {
                *since += dt;
            }
            continue;
        }
        // Still loading in: everyone held where they are.
        if !lobby.members.iter().all(|m| m.loaded) {
            for (_, lp, .., brain) in &mut players {
                if let (true, Some(mut brain)) = (lp.lobby == lobby_e, brain) {
                    brain.held = true;
                }
            }
            continue;
        }
        let map = lobby.map;
        let sites = layout(map);
        let team_of = |lobby: &Lobby, peer: PeerId| lobby.team_of(peer).unwrap_or_default();

        // Round 1 (everyone's already at their spawn — `crate::lobby`).
        if lobby.snd.round == 0 {
            start_round(&mut lobby, run, 1, now, &navs, server, &mut sender, &mut players, lobby_e, false);
        }

        let bodies: Vec<Body> = players
            .iter()
            .filter(|(_, lp, ..)| lp.lobby == lobby_e)
            .map(|(id, _, pose, combat, input, brain)| {
                let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
                Body {
                    peer: id.0,
                    team: team_of(&lobby, id.0),
                    alive: combat.alive,
                    feet,
                    using: match brain {
                        // (A bot uses whatever it's standing at.)
                        Some(_) => true,
                        None => input.is_some_and(|i| i.0.snd_use),
                    },
                }
            })
            .collect();
        let attackers = lobby.snd.attackers;
        let living = |team: Team| bodies.iter().filter(|b| b.team == team && b.alive).count();
        let fielded = |team: Team| bodies.iter().any(|b| b.team == team);
        let body = |peer: PeerId| bodies.iter().find(|b| b.peer == peer);

        match lobby.snd.phase {
            Phase::PreRound => {
                if now >= run.ends_at {
                    lobby.snd.phase = Phase::Live;
                    run.ends_at = now + ROUND_SECS as f32;
                    info!("lobby {lobby_e:?}: round {} is on", lobby.snd.round);
                }
            }
            Phase::Live => {
                let mut over: Option<(Team, RoundEnd)> = None;
                // The bomb: dropped where its carrier fell (or handed to
                // another attacker if they've gone), picked up by any
                // attacker walking over it.
                match lobby.snd.bomb {
                    Bomb::Carried(carrier) => match body(carrier) {
                        Some(b) if b.alive => {}
                        Some(b) => {
                            lobby.snd.bomb = Bomb::Dropped(b.feet.to_array());
                            info!("lobby {lobby_e:?}: the bomb's down where {carrier:?} fell");
                        }
                        None => {
                            lobby.snd.bomb = random_carrier(&bodies, attackers, now);
                        }
                    },
                    Bomb::Dropped(at) => {
                        let at = Vec3::from_array(at);
                        if let Some(b) = bodies
                            .iter()
                            .find(|b| b.alive && b.team == attackers && within(at, b.feet, PICKUP_RADIUS))
                        {
                            lobby.snd.bomb = Bomb::Carried(b.peer);
                            info!("lobby {lobby_e:?}: {:?} picked up the bomb", b.peer);
                        }
                    }
                    _ => {}
                }

                // Who's planting or defusing: the carrier at a site, or a
                // defender at the planted bomb — holding it down.
                let user: Option<(PeerId, Option<Site>)> = match lobby.snd.bomb {
                    Bomb::Carried(carrier) => body(carrier)
                        .filter(|b| b.alive && b.using)
                        .and_then(|b| site_at(map, b.feet).map(|s| (b.peer, Some(s)))),
                    Bomb::Planted { at, .. } => {
                        let at = Vec3::from_array(at);
                        let can = |b: &&Body| {
                            b.alive && b.using && b.team != attackers && within(at, b.feet, DEFUSE_RADIUS)
                        };
                        // (Whoever's already at it keeps it.)
                        let current = run.using.and_then(|(p, _)| bodies.iter().find(|b| b.peer == p)).filter(can);
                        current.or_else(|| bodies.iter().find(can)).map(|b| (b.peer, None))
                    }
                    _ => None,
                };
                run.using = match (user, run.using) {
                    (Some((peer, _)), Some((was, since))) if peer == was => Some((was, since)),
                    (Some((peer, _)), _) => Some((peer, now)),
                    (None, _) => None,
                };
                if lobby.snd.using != run.using.map(|(p, _)| p) {
                    lobby.snd.using = run.using.map(|(p, _)| p);
                }
                if let (Some((peer, site)), Some((_, since))) = (user, run.using) {
                    match site {
                        Some(site) if now - since >= PLANT_SECS => {
                            let feet = body(peer).map_or(sites.site(site), |b| b.feet);
                            lobby.snd.bomb = Bomb::Planted { site, at: feet.to_array() };
                            run.ends_at = now + FUSE_SECS as f32;
                            run.planter = Some(peer);
                            run.using = None;
                            lobby.snd.using = None;
                            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                                m.plants += 1;
                            }
                            info!("lobby {lobby_e:?}: {peer:?} planted the bomb at {}", site.label());
                        }
                        None if now - since >= DEFUSE_SECS => {
                            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                                m.defuses += 1;
                            }
                            over = Some((attackers.other(), RoundEnd::Defused));
                        }
                        _ => {}
                    }
                }

                let planted = lobby.snd.bomb.planted();
                if over.is_none() {
                    over = if now >= run.ends_at && planted {
                        // It goes off: everyone close by dies (the planter
                        // credited), attackers win.
                        if let Bomb::Planted { at, .. } = lobby.snd.bomb {
                            let at = Vec3::from_array(at);
                            for b in bodies.iter().filter(|b| b.alive && b.feet.distance(at) <= shared::search_destroy::BLAST_RADIUS) {
                                hits.write(PlayerHit {
                                    cause: HitCause::Frag,
                                    victim: b.peer,
                                    killer: run.planter.filter(|p| body(*p).is_some()).unwrap_or(b.peer),
                                    damage: 10_000.0,
                                    bomb_shot: false,
                                    blast: true,
                                    critical: false,
                                    point: None,
                                });
                            }
                            let msg = SndBombExploded { at: at.to_array() };
                            if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers())) {
                                error!("failed to send the bomb going off: {e:?}");
                            }
                        }
                        Some((attackers, RoundEnd::Detonated))
                    } else if now >= run.ends_at {
                        Some((attackers.other(), RoundEnd::TimeUp))
                    } else if fielded(attackers.other()) && living(attackers.other()) == 0 {
                        Some((attackers, RoundEnd::Eliminated))
                    } else if fielded(attackers) && living(attackers) == 0 && !planted {
                        Some((attackers.other(), RoundEnd::Eliminated))
                    } else {
                        None
                    };
                }
                if let Some((winner, how)) = over {
                    lobby.snd.wins[winner.index()] += 1;
                    lobby.snd.phase = Phase::Over { winner, how };
                    lobby.snd.using = None;
                    run.using = None;
                    run.ends_at = now + ROUND_OVER_SECS as f32;
                    info!(
                        "lobby {lobby_e:?}: round {} to {} ({how:?}) — {}–{}",
                        lobby.snd.round,
                        winner.label(),
                        lobby.snd.wins[0],
                        lobby.snd.wins[1]
                    );
                }
            }
            Phase::Over { .. } => {
                if now >= run.ends_at {
                    // A team's there: the match is over (the usual end —
                    // VICTORY / DEFEAT, then the results).
                    if lobby.snd.wins.iter().any(|&w| w >= lobby.rounds_to_win()) {
                        endings.begin(lobby_e, clock.0, true);
                        continue;
                    }
                    let next = lobby.snd.round + 1;
                    start_round(&mut lobby, run, next, now, &navs, server, &mut sender, &mut players, lobby_e, true);
                }
            }
        }

        let secs = (run.ends_at - now).max(0.0).ceil() as u32;
        if lobby.snd.secs_left != secs {
            lobby.snd.secs_left = secs;
        }
        steer_bots(&lobby, run, &mut players, lobby_e);
    }
}

/// Hand the bomb to a random living attacker (if there's one).
fn random_carrier(bodies: &[Body], attackers: Team, now: f32) -> Bomb {
    let pool: Vec<PeerId> = bodies.iter().filter(|b| b.team == attackers && b.alive).map(|b| b.peer).collect();
    if pool.is_empty() {
        return Bomb::None;
    }
    let r = rand01(now.to_bits() as u64 ^ 0x5eed_b0b);
    Bomb::Carried(pool[((r * pool.len() as f32) as usize).min(pool.len() - 1)])
}

/// Round `n` begins: sides set, the dead back, the bomb to a random
/// attacker, everyone held for the countdown — at their team's spawn
/// (`respawn`; round 1's starts are the game's own).
#[allow(clippy::too_many_arguments)]
fn start_round(
    lobby: &mut Lobby,
    run: &mut Run,
    n: u32,
    now: f32,
    navs: &NavGraphs,
    server: &Server,
    sender: &mut ServerMultiMessageSender,
    players: &mut Players,
    lobby_e: Entity,
    respawn: bool,
) {
    let attackers = attackers_in(n);
    lobby.snd.round = n;
    lobby.snd.attackers = attackers;
    lobby.snd.phase = Phase::PreRound;
    lobby.snd.dead.clear();
    lobby.snd.using = None;
    run.using = None;
    run.planter = None;
    run.ends_at = now + PRE_ROUND_SECS as f32;
    let seed = (now.to_bits() as u64) ^ lobby_e.to_bits() ^ n as u64;
    run.target_site = if rand01(seed) < 0.5 { Site::A } else { Site::B };

    let mut bodies = Vec::new();
    for (i, (id, lp, pose, mut combat, _, brain)) in players.iter_mut().enumerate() {
        if lp.lobby != lobby_e {
            continue;
        }
        let peer = id.0;
        let team = lobby.team_of(peer).unwrap_or_default();
        if respawn {
            let (feet, yaw) = spawn_spot(navs, lobby.map, team == attackers, seed ^ peer.to_bits() ^ (i as u64) << 20);
            combat.respawn(now);
            match brain {
                Some(mut brain) => brain.place(feet, yaw, now),
                None if !has_no_client(peer) => {
                    let msg = PlayerRespawn { pos: feet.to_array(), yaw, immediate: true };
                    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
                        error!("failed to send a round spawn to {peer:?}: {e:?}");
                    }
                }
                None => {}
            }
        }
        bodies.push(Body {
            peer,
            team,
            alive: combat.alive,
            feet: pose.translation - Vec3::Y * EYE_HEIGHT,
            using: false,
        });
    }
    lobby.snd.bomb = random_carrier(&bodies, attackers, now);
    info!("lobby {lobby_e:?}: round {n}, {} attacking", attackers.label());
}

/// Tell each bot where to go this tick (and hold them before a round).
fn steer_bots(lobby: &Lobby, run: &Run, players: &mut Players, lobby_e: Entity) {
    let sites = layout(lobby.map);
    let snd = &lobby.snd;
    let attackers = snd.attackers;
    let site = sites.site(run.target_site);
    // The dropped bomb goes to whichever attacking bot's nearest it.
    let fetcher = match snd.bomb {
        Bomb::Dropped(at) => {
            let at = Vec3::from_array(at);
            players
                .iter()
                .filter(|(id, lp, _, combat, _, brain)| {
                    lp.lobby == lobby_e && combat.alive && brain.is_some() && lobby.team_of(id.0) == Some(attackers)
                })
                .min_by(|a, b| {
                    let d = |p: &PlayerPose| p.translation.distance_squared(at);
                    d(a.2).total_cmp(&d(b.2))
                })
                .map(|(id, ..)| id.0)
        }
        _ => None,
    };
    let mut defenders_seen = 0usize;
    for (id, lp, _, _, _, brain) in players.iter_mut() {
        let Some(mut brain) = brain else { continue };
        if lp.lobby != lobby_e {
            continue;
        }
        let attacking = lobby.team_of(id.0) == Some(attackers);
        brain.held = snd.phase == Phase::PreRound;
        brain.objective = match (snd.phase, snd.bomb) {
            (Phase::Live, Bomb::Planted { at, .. }) => Some(Vec3::from_array(at)),
            (Phase::Live, Bomb::Dropped(at)) if attacking && fetcher == Some(id.0) => Some(Vec3::from_array(at)),
            (Phase::Live, _) if attacking => Some(site),
            // (The defenders split between the two sites.)
            (Phase::Live, _) => {
                defenders_seen += 1;
                Some(sites.sites[defenders_seen % 2])
            }
            _ => None,
        };
    }
}
