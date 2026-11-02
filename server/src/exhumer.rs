//! The Exhumer, Der Tempel's wonder weapon (`shared::exhumer` — what it does,
//! and its numbers). It fires no bullet (`sim` ignores its shots); instead:
//!
//! * **Its beam** ([`run_beams`]) — while a player holds aim with it in hand
//!   (`PlayerPose::beaming`, from their input), every tick the beam from
//!   their eye along their look, stopped by the map, takes hold of the
//!   nearest few zombies on its line and drains them: an ordinary one by a
//!   share of its health (dead in `DRAIN_SECS`, whatever its round), held
//!   in place while it does; what can't be lifted by a flat amount. What's
//!   drained is taken every `BEAM_TICK_SECS`, and the clients told how far
//!   drained each is ([`shared::exhumer::ExhumerDrain`] — they lift them
//!   with it). One drained dead dies by the shooter (their kill, their
//!   points), thrown limp out of the air (a ragdoll, from `blast_from`),
//!   and its soul goes into their Exhumer ([`shared::exhumer::ExhumerSouls`]).
//! * **Letting them out** ([`on_release_souls`]) — every soul held goes, one
//!   after each of the nearest zombies; only as many as there are zombies
//!   about, the rest kept. Each tears its zombie's soul out as it gets
//!   there ([`run_tearing`]); what can't be lifted is hurt instead.
//!
//! The souls are each player's, kept here, and the only ammo it has. All
//! of it is per game: what's drained, souls in flight and souls held whose
//! lobby's gone or stopped (or whose player's no longer carrying it) are
//! dropped, and a paused game's wait.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use shared::bot_players::is_bot_peer;
use shared::exhumer::{
    beam_dir, beam_targets, liftable, ExhumerDrain, ExhumerSouls, ExhumerSwarm, ReleaseSouls, BEAM_RANGE,
    BEAM_TICK_SECS, DRAIN_SECS, HEAVY_DAMAGE, HEAVY_DPS, LIFT_HEIGHT, MAX_SOULS, RELEASE_COOLDOWN_SECS, RELEASE_RANGE,
    SWARM_SPEED,
};
use shared::map::CollisionWorld;
use shared::weapon::{SlotWeapon, WeaponId};
use shared::{GameChannel, GameMode, Lobby, PlayerId, PlayerPose};

use crate::ai::BotBrain;
use crate::collision::MapColliders;
use crate::lobby::LobbyPlayer;
use crate::pvp::{HitCause, PlayerCombat, PlayerHit};
use crate::sim::{EYE_HEIGHT, PLAYER_HEIGHT};

pub(crate) struct ExhumerPlugin;

impl Plugin for ExhumerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Exhumers>()
            .add_observer(on_release_souls)
            .add_systems(
                FixedUpdate,
                (run_beams, run_tearing).chain().after(crate::sim::apply_client_pose),
            );
    }
}

/// A soul let out, on its way to `zombie`: torn out in `left` s.
struct Tearing {
    lobby: Entity,
    shooter: PeerId,
    zombie: PeerId,
    left: f32,
}

/// Everything the Exhumer has going on, across every lobby.
#[derive(Resource, Default)]
pub(crate) struct Exhumers {
    /// Each player's souls, by (lobby, player).
    souls: HashMap<(Entity, PeerId), u32>,
    /// What each beam's drained of each zombie it holds, not yet taken:
    /// (shooter, zombie) → damage.
    drained: HashMap<(PeerId, PeerId), f32>,
    /// Each beam's time (s) to its next take.
    tick: HashMap<PeerId, f32>,
    /// Souls let out, on their way.
    tearing: Vec<Tearing>,
    /// When (`Time::elapsed_secs`) each player last let their souls out.
    released_at: HashMap<PeerId, f32>,
}

/// Where a zombie's chest is, at `pose`.
fn chest(pose: &PlayerPose) -> Vec3 {
    pose.translation - Vec3::Y * (EYE_HEIGHT - PLAYER_HEIGHT * 0.6)
}

fn send_to<M: lightyear::prelude::Message>(sender: &mut ServerMultiMessageSender, server: &Server, lobby: &Lobby, msg: &M) {
    if let Err(e) = sender.send::<_, GameChannel>(msg, server, &NetworkTarget::Only(lobby.real_peers())) {
        error!("failed to send an Exhumer message: {e:?}");
    }
}

/// A zombie's soul torn out by `shooter`: it dies by them, thrown limp out of
/// the air (`from` over it).
fn tear(hits: &mut EventWriter<PlayerHit>, shooter: PeerId, zombie: PeerId, health: f32, at: Vec3) {
    hits.write(PlayerHit {
        cause: HitCause::Exhumer,
        victim: zombie,
        killer: shooter,
        damage: health + 1.0,
        bomb_shot: false,
        blast: true,
        critical: false,
        point: None,
        // (Dropped limp: a ragdoll, pushed down from just over it.)
        blast_from: Some(at + Vec3::Y * 2.5),
    });
}

pub(crate) type ZombieQuery<'w, 's> = Query<
    'w,
    's,
    (&'static PlayerId, &'static PlayerPose, &'static PlayerCombat, &'static LobbyPlayer, Option<&'static mut BotBrain>),
>;

/// Every beam this tick: the zombies it holds drained (held in place while
/// they are), taken every `BEAM_TICK_SECS` — a soul for every one it kills.
/// And forget what's no longer in a game, or no longer held.
#[allow(clippy::too_many_arguments)]
fn run_beams(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    colliders: Res<MapColliders>,
    mut ex: ResMut<Exhumers>,
    lobbies: Query<(Entity, &Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat, &LobbyPlayer), Without<crate::zombies::Zombie>>,
    mut zombies: ZombieQuery,
    mut hits: EventWriter<PlayerHit>,
) {
    let server = server.into_inner();
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let running = |e: Entity| lobbies.get(e).ok().map(|(_, l)| l).filter(|l| l.started && l.mode == GameMode::Zombies);
    let carries = |l: &Lobby, peer: PeerId| {
        l.members.iter().any(|m| m.peer == peer && m.weapons.contains(&SlotWeapon::Gun(WeaponId::Exhumer)))
    };
    ex.souls.retain(|(lobby_e, peer), _| running(*lobby_e).is_some_and(|l| carries(l, *peer)));
    ex.released_at.retain(|peer, _| lobbies.iter().any(|(_, l)| l.started && l.has(*peer)));

    let mut beaming = Vec::new();
    for (id, pose, combat, lp) in &players {
        let Some(lobby) = running(lp.lobby) else { continue };
        if !pose.beaming || !combat.alive || combat.down.is_some() || lobby.paused || !carries(lobby, id.0) {
            continue;
        }
        beaming.push((id.0, lp.lobby, pose.translation, beam_dir(pose.yaw, pose.pitch)));
    }
    // (A beam let go, or a zombie it's let go of: what it drained but didn't
    // take goes.)
    let mut held_now: Vec<(PeerId, PeerId)> = Vec::new();
    let mut souls: HashMap<(Entity, PeerId), Vec<[f32; 3]>> = HashMap::default();
    for (shooter, lobby_e, eye, dir) in beaming {
        let Some(lobby) = running(lobby_e) else { continue };
        let reach = colliders.for_lobby(lobby).raycast(eye, dir, BEAM_RANGE).map_or(BEAM_RANGE, |h| h.distance);
        let chests = zombies
            .iter()
            .filter(|(id, _, c, lp, _)| lp.lobby == lobby_e && c.alive && is_bot_peer(id.0))
            .map(|(id, pose, ..)| (id.0, chest(pose)));
        let held = beam_targets(eye, dir, reach, chests);
        let take = {
            let tick = ex.tick.entry(shooter).or_insert(BEAM_TICK_SECS);
            *tick -= dt;
            if *tick <= 0.0 {
                *tick += BEAM_TICK_SECS;
                true
            } else {
                false
            }
        };
        let mut drained_shares = Vec::new();
        for (id, pose, combat, _, brain) in zombies.iter_mut() {
            if !held.contains(&id.0) {
                continue;
            }
            held_now.push((shooter, id.0));
            let lifts = liftable(pose.zombie);
            let rate = if lifts { combat.max_health() / DRAIN_SECS } else { HEAVY_DPS };
            if let Some(mut brain) = brain.filter(|_| lifts) {
                // (Held where it is while it's drained.)
                brain.stun(now + BEAM_TICK_SECS * 2.0);
            }
            let acc = ex.drained.entry((shooter, id.0)).or_insert(0.0);
            *acc += rate * dt;
            let left = (combat.health - *acc).max(0.0);
            if lifts {
                drained_shares.push((id.0, (1.0 - left / combat.max_health().max(1.0)).clamp(0.0, 1.0)));
            }
            if !take {
                continue;
            }
            let damage = std::mem::take(acc);
            if damage >= combat.health {
                // Drained dead: its soul's torn out, into the Exhumer.
                let lift = if lifts { LIFT_HEIGHT } else { 0.0 };
                let at = chest(pose) + Vec3::Y * lift;
                tear(&mut hits, shooter, id.0, combat.health, at);
                souls.entry((lobby_e, shooter)).or_default().push(at.to_array());
            } else {
                hits.write(PlayerHit {
                    cause: HitCause::Exhumer,
                    victim: id.0,
                    killer: shooter,
                    damage,
                    bomb_shot: false,
                    blast: true,
                    critical: false,
                    point: None,
                    blast_from: None,
                });
            }
        }
        if take && !drained_shares.is_empty() {
            send_to(&mut sender, server, lobby, &ExhumerDrain { shooter, zombies: drained_shares });
        }
    }
    ex.drained.retain(|k, _| held_now.contains(k));
    let shooters: Vec<PeerId> = held_now.iter().map(|(s, _)| *s).collect();
    ex.tick.retain(|s, _| shooters.contains(s));
    for ((lobby_e, shooter), from) in souls {
        let Some(lobby) = running(lobby_e) else { continue };
        let held = ex.souls.entry((lobby_e, shooter)).or_insert(0);
        *held = (*held + from.len() as u32).min(MAX_SOULS);
        let msg = ExhumerSouls { shooter, from, souls: *held };
        send_to(&mut sender, server, lobby, &msg);
    }
}

/// The souls let out, as each gets to its zombie: its soul torn out (no
/// soul back for it).
fn run_tearing(
    time: Res<Time>,
    mut ex: ResMut<Exhumers>,
    lobbies: Query<&Lobby>,
    zombies: Query<(&PlayerId, &PlayerPose, &PlayerCombat, &LobbyPlayer)>,
    mut hits: EventWriter<PlayerHit>,
) {
    let dt = time.delta_secs();
    let mut i = 0;
    while i < ex.tearing.len() {
        let t = &mut ex.tearing[i];
        let Some(lobby) = lobbies.get(t.lobby).ok().filter(|l| l.started) else {
            ex.tearing.swap_remove(i);
            continue;
        };
        if !lobby.paused {
            t.left -= dt;
        }
        if t.left > 0.0 {
            i += 1;
            continue;
        }
        let t = ex.tearing.swap_remove(i);
        let found = zombies.iter().find(|(id, _, c, lp)| id.0 == t.zombie && c.alive && lp.lobby == t.lobby);
        let Some((_, pose, combat, _)) = found else { continue };
        if liftable(pose.zombie) || pose.zombie.is_dog() {
            tear(&mut hits, t.shooter, t.zombie, combat.health, chest(pose));
        } else {
            hits.write(PlayerHit {
                cause: HitCause::Exhumer,
                victim: t.zombie,
                killer: t.shooter,
                damage: HEAVY_DAMAGE,
                bomb_shot: false,
                blast: true,
                critical: false,
                point: None,
                blast_from: None,
            });
        }
    }
}

/// A player lets their Exhumer's souls out: in a running `Zombies` game,
/// up, carrying it, with souls. One goes after each of the nearest zombies
/// (within `RELEASE_RANGE`) — only as many as there are, the rest kept.
#[allow(clippy::too_many_arguments)]
fn on_release_souls(
    trigger: Trigger<RemoteTrigger<ReleaseSouls>>,
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut ex: ResMut<Exhumers>,
    lobbies: Query<(Entity, &Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat), Without<crate::zombies::Zombie>>,
    zombies: Query<(&PlayerId, &PlayerPose, &PlayerCombat, &LobbyPlayer)>,
) {
    let peer = trigger.from;
    let now = time.elapsed_secs();
    let Some((lobby_e, lobby)) = lobbies
        .iter()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer) && !l.paused)
    else {
        return;
    };
    let carries = lobby
        .members
        .iter()
        .any(|m| m.peer == peer && m.weapons.contains(&SlotWeapon::Gun(WeaponId::Exhumer)));
    let Some((_, eye, _)) = players.iter().find(|(id, _, c)| id.0 == peer && c.alive && c.down.is_none()) else {
        return;
    };
    let held = ex.souls.get(&(lobby_e, peer)).copied().unwrap_or(0);
    let cooled = ex.released_at.get(&peer).is_none_or(|at| now - at >= RELEASE_COOLDOWN_SECS * 0.9);
    if !carries || held == 0 || !cooled {
        return;
    }
    let origin = eye.translation;
    let taken: Vec<PeerId> = ex.tearing.iter().map(|t| t.zombie).collect();
    let mut near: Vec<(f32, PeerId, Vec3)> = zombies
        .iter()
        .filter(|(id, _, c, lp)| lp.lobby == lobby_e && c.alive && is_bot_peer(id.0) && !taken.contains(&id.0))
        .map(|(id, pose, ..)| {
            let at = chest(pose);
            (at.distance(origin), id.0, at)
        })
        .filter(|(d, ..)| *d <= RELEASE_RANGE)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(held as usize);
    if near.is_empty() {
        return;
    }
    let left = held - near.len() as u32;
    ex.souls.insert((lobby_e, peer), left);
    ex.released_at.insert(peer, now);
    let mut targets = Vec::new();
    for (d, zombie, at) in near {
        ex.tearing.push(Tearing { lobby: lobby_e, shooter: peer, zombie, left: d / SWARM_SPEED + 0.2 });
        targets.push((zombie, at.to_array()));
    }
    info!("{peer:?} let {} of their Exhumer's souls out ({left} kept)", targets.len());
    let server = server.into_inner();
    send_to(&mut sender, server, lobby, &ExhumerSwarm { shooter: peer, origin: origin.to_array(), targets });
    send_to(&mut sender, server, lobby, &ExhumerSouls { shooter: peer, from: Vec::new(), souls: left });
}
