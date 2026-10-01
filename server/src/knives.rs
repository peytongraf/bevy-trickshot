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
//!
//! A knife that kills drops to the ground beside its victim
//! ([`KnifeBody::drop_beside`]), clear of the body. A stopped knife lies where it landed for
//! [`shared::throwing_knife::REST_LINGER_SECS`]; any living player in its
//! lobby close enough can pick it up ([`PickUpKnife`]) for one more knife.
//! A player carries one kind of lethal: picking a knife up while carrying
//! molotovs drops them ([`crate::molotovs::drop_around`]), and picking up a
//! molotov drops their knives here ([`drop_knives_around`]).

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::ballistics::Target;
use shared::bots::{BOT_HEAD_RADIUS, BOT_HEIGHT, BOT_RADIUS};
use shared::hitbox::Capsule;
use shared::throwing_knife::{in_pickup_range, KnifeBody, MAX_CARRIED, MAX_KNIVES_PER_PLAYER};
use shared::{
    Bot, GameChannel, GameMode, KnifePickedUp, Lobby, PickUpKnife, PlayerId, PlayerPose,
    ThrowKnife, ThrowingKnifeHit, ThrowingKnifeImpact, ThrownKnife, TrickScore,
};

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

/// Impacts softer than this (m/s into the surface) make no sound — a knife
/// skidding along the ground bounces over and over, barely touching it.
const MIN_IMPACT_SOUND_SPEED: f32 = 1.5;
/// Least time (s) between one knife's impact sounds.
const MIN_IMPACT_SOUND_INTERVAL_SECS: f32 = 0.08;

/// The server-side simulation state of one [`ThrownKnife`].
#[derive(Component)]
struct KnifeSim {
    body: KnifeBody,
    owner: PeerId,
    lobby: Entity,
    /// `KnifeBody::age` at this knife's last announced impact sound.
    last_impact_sound: f32,
}

/// When each player last threw (`Time::elapsed_secs`).
#[derive(Resource, Default)]
struct LastThrow(HashMap<PeerId, f32>);

/// What a knife can hit.
enum Victim {
    Player(PeerId),
    Bot(Entity),
}

/// A [`Victim`], with where they stand and how wide they are — where a knife
/// that kills them drops.
struct VictimAt {
    who: Victim,
    feet: Vec3,
    radius: f32,
}

pub struct KnivesPlugin;

impl Plugin for KnivesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MapColliders::load())
            .init_resource::<LastThrow>()
            .add_observer(on_throw_knife)
            .add_observer(on_pick_up_knife)
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

    let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.started && !l.paused && l.has(peer)) else {
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
    // (Only knives still flying count — stopped ones lie around for a minute.)
    if knives
        .iter()
        .filter(|k| k.owner == peer && !k.body.resting)
        .count()
        >= MAX_KNIVES_PER_PLAYER
    {
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
    let members: Vec<PeerId> = lobby.real_peers();
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
            last_impact_sound: f32::NEG_INFINITY,
        },
        Replicate::to_clients(NetworkTarget::Only(members.clone())),
        InterpolationTarget::to_clients(NetworkTarget::Only(members)),
    ));
    info!("{peer:?} threw a knife");
}

/// A client asked to pick up the stopped knife nearest them: if one's in
/// range, remove it and tell them they've got it — and drop any molotovs
/// they were carrying instead.
#[allow(clippy::too_many_arguments)]
fn on_pick_up_knife(
    trigger: Trigger<RemoteTrigger<PickUpKnife>>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    knives: Query<(Entity, &KnifeSim)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.started && !l.paused && l.has(peer)) else {
        return;
    };
    if combats.iter().any(|(id, c)| id.0 == peer && !c.alive) {
        return;
    }
    let Some((_, pose)) = poses.iter().find(|(id, _)| id.0 == peer) else {
        return;
    };
    let eye = pose.translation;
    let feet = eye - Vec3::Y * EYE_HEIGHT;
    let nearest = knives
        .iter()
        .filter(|(_, k)| {
            k.lobby == lobby_e && k.body.resting && in_pickup_range(feet, eye, k.body.pos)
        })
        .min_by(|a, b| {
            a.1.body.pos.distance_squared(eye).total_cmp(&b.1.body.pos.distance_squared(eye))
        });
    let Some((knife, _)) = nearest else {
        return;
    };
    commands.entity(knife).try_despawn();
    let drop = trigger.trigger.drop_molotovs.min(shared::molotov::MAX_MOLOTOVS);
    if drop > 0 && lobby.mode == GameMode::Zombies {
        crate::molotovs::drop_around(&mut commands, lobby_e, lobby, feet, drop, &colliders.for_lobby(lobby));
    }
    if let Err(e) =
        sender.send::<_, GameChannel>(&KnifePickedUp, server.into_inner(), &NetworkTarget::Single(peer))
    {
        error!("failed to send knife pickup: {e:?}");
    }
    info!("{peer:?} picked up a throwing knife");
}

/// Leave `count` (capped at [`MAX_CARRIED`]) knives lying on the ground
/// around `feet`, owned by `owner` — the knives a player carried when they
/// picked up a molotov. Anyone in the lobby can pick them up again.
pub(crate) fn drop_knives_around(
    commands: &mut Commands,
    lobby_e: Entity,
    lobby: &Lobby,
    owner: PeerId,
    feet: Vec3,
    count: u32,
    world: &dyn shared::map::CollisionWorld,
) {
    let count = count.min(MAX_CARRIED);
    let members: Vec<PeerId> = lobby.real_peers();
    for i in 0..count {
        let seed = (i as f32 + 0.5) / count as f32;
        let ground = shared::molotov::drop_spot(feet, seed, world);
        let body = KnifeBody::lying_at(ground, seed * std::f32::consts::TAU + 1.3);
        commands.spawn((
            Name::from("ThrownKnife"),
            ThrownKnife {
                owner,
                pos: body.pos,
                rot: body.rot,
                resting: true,
            },
            KnifeSim {
                body,
                owner,
                lobby: lobby_e,
                last_impact_sound: f32::NEG_INFINITY,
            },
            Replicate::to_clients(NetworkTarget::Only(members.clone())),
            InterpolationTarget::to_clients(NetworkTarget::Only(members.clone())),
        ));
    }
    info!("{owner:?} dropped {count} throwing knife(s)");
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
    poses: Query<(&PlayerId, &PlayerPose, &crate::lobby::LobbyPlayer)>,
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
        // Paused: hang where it is.
        if lobby.paused {
            continue;
        }

        // Who this knife may kill, per the lobby's mode (no one, once it's
        // lying still).
        let mut victims: HashMap<u64, VictimAt> = HashMap::new();
        let mut targets: Vec<Target> = Vec::new();
        match lobby.mode {
            _ if sim.body.resting => {}
            GameMode::FreeForAll | GameMode::Zombies => {
                for (id, pose, lp) in &poses {
                    if id.0 == sim.owner || lp.lobby != sim.lobby {
                        continue;
                    }
                    // `Zombies`: a player's knife only kills zombies.
                    if lobby.mode == GameMode::Zombies
                        && shared::bot_players::is_bot_peer(id.0)
                            == shared::bot_players::is_bot_peer(sim.owner)
                    {
                        continue;
                    }
                    if combats.iter().any(|(c_id, c)| c_id.0 == id.0 && !c.alive) {
                        continue;
                    }
                    let key = id.0.to_bits();
                    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
                    victims.insert(
                        key,
                        VictimAt {
                            who: Victim::Player(id.0),
                            feet,
                            radius: PLAYER_RADIUS,
                        },
                    );
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
                    victims.insert(
                        key,
                        VictimAt {
                            who: Victim::Bot(bot_e),
                            feet: bot.pos,
                            radius: BOT_RADIUS,
                        },
                    );
                    targets.push(Target {
                        id: key,
                        body: Capsule::standing(bot.pos, BOT_HEIGHT, BOT_RADIUS),
                        head: Capsule::head(bot.pos, BOT_HEIGHT, BOT_HEAD_RADIUS),
                    });
                }
            }
        }

        let world = &colliders.for_lobby(lobby);
        let hit = sim.body.step(dt, world, &targets);

        if let Some(hit) = hit {
            let owner = sim.owner;
            match victims.get(&hit.target).map(|v| &v.who) {
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
                    let damage = if lobby.mode == GameMode::Zombies {
                        shared::zombies::ZOMBIE_KNIFE_DAMAGE
                    } else {
                        shared::melee::KNIFE_DAMAGE
                    };
                    player_hits.write(PlayerHit {
                        victim: *victim,
                        killer: owner,
                        damage,
                        bomb_shot: false,
                        blast: false,
                        critical: false,
                    });
                    info!("{owner:?} killed {victim:?} with a throwing knife");
                }
                None => {}
            }
            // Everyone in the lobby hears the hit from where it landed.
            if let Some(victim) = victims.get(&hit.target) {
                let members: Vec<PeerId> = lobby.real_peers();
                let msg = ThrowingKnifeHit {
                    point: hit.point.to_array(),
                };
                if let Err(e) =
                    sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(members))
                {
                    error!("failed to send throwing knife hit: {e:?}");
                }
                // Then it drops beside the body, to be picked up.
                sim.body.drop_beside(victim.feet, victim.radius, world);
            } else {
                commands.entity(entity).try_despawn();
                continue;
            }
        }

        // A surface strike: every lobby member hears an impact sound from the
        // spot (the server picks which of the clips, so it's the same for
        // all). Skips feather-light touches and rate-limits a knife that's
        // bouncing rapidly.
        if let Some(impact) = sim.body.impact {
            if impact.speed >= MIN_IMPACT_SOUND_SPEED
                && sim.body.age - sim.last_impact_sound >= MIN_IMPACT_SOUND_INTERVAL_SECS
            {
                sim.last_impact_sound = sim.body.age;
                let members: Vec<PeerId> = lobby.real_peers();
                // A cheap deterministic scramble of which knife / which bounce
                // — no `rand` needed.
                let variant = (entity.to_bits() ^ (sim.body.bounces as u64).wrapping_mul(0x9E37_79B9))
                    .wrapping_mul(0x2545_F491_4F6C_DD1D)
                    >> 56;
                let msg = ThrowingKnifeImpact {
                    point: impact.point.to_array(),
                    variant: variant as u8,
                };
                if let Err(e) =
                    sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(members))
                {
                    error!("failed to send throwing knife impact: {e:?}");
                }
            }
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
