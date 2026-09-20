//! Thrown throwing knives: the server owns the whole flight.
//!
//! A client's [`ThrowKnife`] request (sent when its throw animation reaches
//! the release point) becomes a [`ThrownKnife`] entity replicated to the
//! lobby, stepped every tick by [`shared::throwing_knife::KnifeBody`] against
//! the map's collision mesh ([`crate::collision`]) — it arcs, bounces off
//! walls and the ground losing energy, and stops; a moving knife that reaches
//! a valid target kills it:
//!
//! * `Freestyle` — bots die (flat throwing-knife points); other players are
//!   ignored entirely, the knife flies straight through them.
//! * `FreeForAll` — other players die through the normal [`PlayerHit`] path
//!   (so respawn, kill credit and the victim's kill cam all follow); there
//!   are no bots.
//!
//! The thrower is never hit by their own knife, and a dead player can neither
//! throw nor be hit.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::ballistics::Target;
use shared::bots::{BOT_HEAD_RADIUS, BOT_HEIGHT, BOT_RADIUS};
use shared::hitbox::Capsule;
use shared::throwing_knife::{KnifeBody, MAX_KNIVES_PER_PLAYER};
use shared::{Bot, GameChannel, GameMode, Lobby, PlayerId, PlayerPose, ThrowKnife, ThrownKnife, TrickScore};

use crate::bots::{BotHit, LobbyBot};
use crate::collision::MapColliders;
use crate::pvp::{PlayerCombat, PlayerHit};
use crate::sim::{EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_RADIUS};

/// Least time (s) between one player's throws — the throw animation is
/// already longer than this; it just stops a modified client spamming.
const MIN_THROW_INTERVAL_SECS: f32 = 0.5;
/// A throw's reported origin further than this (m) from the player's real
/// eye position is ignored in favour of the eye position — movement is
/// client-authoritative, but a knife shouldn't start across the map.
const MAX_ORIGIN_DRIFT: f32 = 3.0;

/// The server-side simulation state of one [`ThrownKnife`].
#[derive(Component)]
struct KnifeSim {
    body: KnifeBody,
    owner: PeerId,
    lobby: Entity,
}

/// When each player last threw (`Time::elapsed_secs`).
#[derive(Resource, Default)]
struct LastThrow(HashMap<PeerId, f32>);

/// What a knife can hit.
enum Victim {
    Player(PeerId),
    Bot(Entity),
}

pub struct KnivesPlugin;

impl Plugin for KnivesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MapColliders::load())
            .init_resource::<LastThrow>()
            .add_observer(on_throw_knife)
            .add_systems(FixedUpdate, (step_knives, cull_orphan_knives).chain());
    }
}

/// A client asked to throw a knife: validate, then spawn it.
#[allow(clippy::too_many_arguments)]
fn on_throw_knife(
    trigger: Trigger<RemoteTrigger<ThrowKnife>>,
    time: Res<Time>,
    mut last: ResMut<LastThrow>,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    knives: Query<&KnifeSim>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let req = &trigger.trigger;

    let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.started && l.has(peer)) else {
        return;
    };
    // `FreeForAll` players mid-respawn can't throw (`Freestyle` players have
    // no `PlayerCombat` and are always alive).
    if combats.iter().any(|(id, c)| id.0 == peer && !c.alive) {
        return;
    }
    let Some((_, pose)) = poses.iter().find(|(id, _)| id.0 == peer) else {
        return;
    };

    let now = time.elapsed_secs();
    if last
        .0
        .get(&peer)
        .is_some_and(|t| now - t < MIN_THROW_INTERVAL_SECS)
    {
        return;
    }
    if knives.iter().filter(|k| k.owner == peer).count() >= MAX_KNIVES_PER_PLAYER {
        return;
    }

    let dir = Vec3::from_array(req.dir);
    if !dir.is_finite() || dir.length_squared() < 1e-6 {
        return;
    }
    let mut origin = Vec3::from_array(req.origin);
    if !origin.is_finite() || origin.distance(pose.translation) > MAX_ORIGIN_DRIFT {
        origin = pose.translation;
    }
    last.0.insert(peer, now);

    let body = KnifeBody::thrown(origin, dir);
    let members: Vec<PeerId> = lobby.members.iter().map(|m| m.peer).collect();
    commands.spawn((
        Name::from("ThrownKnife"),
        ThrownKnife {
            owner: peer,
            pos: body.pos,
            rot: body.rot,
            resting: false,
        },
        KnifeSim {
            body,
            owner: peer,
            lobby: lobby_e,
        },
        Replicate::to_clients(NetworkTarget::Only(members.clone())),
        InterpolationTarget::to_clients(NetworkTarget::Only(members)),
    ));
    info!("{peer:?} threw a knife");
}

/// Step every knife one tick: fly / bounce / rest, and apply a kill if it hit
/// someone it's allowed to.
#[allow(clippy::too_many_arguments)]
fn step_knives(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    bots: Query<(Entity, &Bot, &LobbyBot)>,
    mut knives: Query<(Entity, &mut KnifeSim, &mut ThrownKnife)>,
    mut bot_hits: EventWriter<BotHit>,
    mut player_hits: EventWriter<PlayerHit>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let server = server.into_inner();

    for (entity, mut sim, mut replicated) in &mut knives {
        let Ok((_, lobby)) = lobbies.get(sim.lobby) else {
            continue; // `cull_orphan_knives` removes it
        };

        // Who this knife may kill, per the lobby's mode.
        let mut victims: HashMap<u64, Victim> = HashMap::new();
        let mut targets: Vec<Target> = Vec::new();
        match lobby.mode {
            GameMode::FreeForAll => {
                for (id, pose) in &poses {
                    if id.0 == sim.owner || !lobby.has(id.0) {
                        continue;
                    }
                    if combats.iter().any(|(c_id, c)| c_id.0 == id.0 && !c.alive) {
                        continue;
                    }
                    let key = id.0.to_bits();
                    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
                    victims.insert(key, Victim::Player(id.0));
                    targets.push(Target {
                        id: key,
                        body: Capsule::standing(feet, PLAYER_HEIGHT, PLAYER_RADIUS),
                        head: Capsule::head(feet, PLAYER_HEIGHT, 0.12),
                    });
                }
            }
            GameMode::Freestyle => {
                for (bot_e, bot, lb) in &bots {
                    if lb.lobby != sim.lobby || !bot.alive {
                        continue;
                    }
                    let key = bot_e.to_bits();
                    victims.insert(key, Victim::Bot(bot_e));
                    targets.push(Target {
                        id: key,
                        body: Capsule::standing(bot.pos, BOT_HEIGHT, BOT_RADIUS),
                        head: Capsule::head(bot.pos, BOT_HEIGHT, BOT_HEAD_RADIUS),
                    });
                }
            }
        }

        let world = colliders.world(lobby.map);
        let hit = sim.body.step(dt, world, &targets);

        if let Some(hit) = hit {
            let owner = sim.owner;
            match victims.get(&hit.target) {
                Some(Victim::Bot(bot)) => {
                    let (points, lines) = shared::scoring::score_throwing_knife_kill();
                    bot_hits.write(BotHit {
                        bot: *bot,
                        by: owner,
                        points,
                    });
                    let trick = TrickScore {
                        shooter: owner,
                        total: points,
                        lines,
                    };
                    if let Err(e) =
                        sender.send::<_, GameChannel>(&trick, server, &NetworkTarget::All)
                    {
                        error!("failed to broadcast trick score: {e:?}");
                    }
                    info!("{owner:?} killed a bot with a throwing knife for {points} pts");
                }
                Some(Victim::Player(victim)) => {
                    player_hits.write(PlayerHit {
                        victim: *victim,
                        killer: owner,
                        damage: shared::melee::KNIFE_DAMAGE,
                    });
                    info!("{owner:?} killed {victim:?} with a throwing knife");
                }
                None => {}
            }
            commands.entity(entity).try_despawn();
            continue;
        }

        if sim.body.finished() {
            commands.entity(entity).try_despawn();
            continue;
        }

        // Publish the new state — only when it changed, so a knife lying still
        // stops costing replication bandwidth.
        let next = ThrownKnife {
            owner: sim.owner,
            pos: sim.body.pos,
            rot: sim.body.rot,
            resting: sim.body.resting,
        };
        if *replicated != next {
            *replicated = next;
        }
    }
}

/// Drop knives whose lobby has ended or gone away.
fn cull_orphan_knives(
    knives: Query<(Entity, &KnifeSim)>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (entity, sim) in &knives {
        let ended = lobbies.get(sim.lobby).map(|l| !l.started).unwrap_or(true);
        if ended {
            commands.entity(entity).try_despawn();
        }
    }
}
