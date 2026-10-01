//! Molotovs (`Zombies` only): the server owns the flight, the fire and the
//! drops (see `shared::molotov`).
//!
//! * A client's [`ThrowMolotov`] (sent when its throw animation reaches the
//!   release point) becomes a [`ThrownMolotov`] replicated to the lobby,
//!   stepped every tick until it touches the map or a zombie.
//! * There it breaks: the bottle is removed and a [`MolotovFire`] spreads
//!   over the surfaces around ([`shared::molotov::fire_spots`]) for
//!   [`FIRE_SECS`]. Every [`FIRE_TICK_SECS`] it hurts each zombie standing in
//!   it — and the thrower, if they walk into their own fire — through the
//!   normal [`PlayerHit`] path (so kills score, count and can drop things).
//!   Other lobby members are never burnt.
//! * A zombie a player kills sometimes drops a molotov beside its body
//!   ([`MolotovDrop`], always with the lobby's debug `molotov_test` on); any
//!   living player in reach can pick it up ([`PickUpMolotov`]).
//!
//! Everything is removed once its lobby's game isn't running.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::ballistics::Target;
use shared::bot_players::is_bot_peer;
use shared::hitbox::Capsule;
use shared::molotov::{
    drop_spot, fire_spots, in_fire, MolotovBody, DROP_CHANCE, DROP_LINGER_SECS, FIRE_SECS,
    FIRE_TICK_SECS, SELF_TICK_DAMAGE, ZOMBIE_TICK_DAMAGE,
};
use shared::throwing_knife::in_pickup_range;
use shared::{
    GameChannel, GameMode, Lobby, MolotovBurst, MolotovDrop, MolotovFire, MolotovPickedUp, PickUpMolotov,
    PlayerId, PlayerPose, SetMolotovTest, ThrowMolotov, ThrownMolotov,
};

use crate::collision::MapColliders;
use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, PlayerHit, ZombieKilled};
use crate::sim::{EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_RADIUS};

/// Least time (s) between one player's throws.
const MIN_THROW_INTERVAL_SECS: f32 = 0.5;
/// A throw's reported origin further than this (m) from the player's eye is
/// replaced by the eye (see `knives::MAX_ORIGIN_DRIFT`).
const MAX_ORIGIN_DRIFT: f32 = 3.0;

/// The server side of one [`ThrownMolotov`].
#[derive(Component)]
struct MolotovSim {
    body: MolotovBody,
    owner: PeerId,
    lobby: Entity,
}

/// The server side of one [`MolotovFire`].
#[derive(Component)]
struct FireSim {
    owner: PeerId,
    lobby: Entity,
    spots: Vec<Vec3>,
    /// Seconds it's been burning (not counting pauses).
    age: f32,
    /// Seconds since the last damage tick.
    since_tick: f32,
}

/// The server side of one [`MolotovDrop`].
#[derive(Component)]
struct DropSim {
    lobby: Entity,
    age: f32,
}

/// When each player last threw one (`Time::elapsed_secs`).
#[derive(Resource, Default)]
struct LastThrow(HashMap<PeerId, f32>);

pub struct MolotovsPlugin;

impl Plugin for MolotovsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastThrow>()
            .add_observer(on_throw_molotov)
            .add_observer(on_pick_up_molotov)
            .add_observer(on_set_molotov_test)
            .add_systems(
                FixedUpdate,
                (step_molotovs, burn)
                    .chain()
                    .before(crate::pvp::apply_player_hits),
            )
            .add_systems(
                FixedUpdate,
                (roll_drops, age_drops, cull_orphans)
                    .chain()
                    .after(crate::pvp::apply_player_hits),
            );
    }
}

/// The running `Zombies` lobby `peer` is playing in.
fn zombies_lobby<'a>(
    lobbies: &'a Query<(Entity, &Lobby)>,
    peer: PeerId,
) -> Option<(Entity, &'a Lobby)> {
    lobbies
        .iter()
        .find(|(_, l)| l.started && !l.paused && l.mode == GameMode::Zombies && l.has(peer))
}

/// A client asked to throw a molotov: validate, then launch it. (How many
/// the player has left is client-side, like the rest of its ammo.)
#[allow(clippy::too_many_arguments)]
fn on_throw_molotov(
    trigger: Trigger<RemoteTrigger<ThrowMolotov>>,
    time: Res<Time>,
    mut last: ResMut<LastThrow>,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let req = &trigger.trigger;
    let Some((lobby_e, lobby)) = zombies_lobby(&lobbies, peer) else {
        return;
    };
    if combats.iter().any(|(id, c)| id.0 == peer && !c.alive) {
        return;
    }
    let Some((_, pose)) = poses.iter().find(|(id, _)| id.0 == peer) else {
        return;
    };
    let now = time.elapsed_secs();
    if last.0.get(&peer).is_some_and(|t| now - t < MIN_THROW_INTERVAL_SECS) {
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

    let body = MolotovBody::thrown(origin, dir);
    let members = lobby.real_peers();
    commands.spawn((
        Name::from("ThrownMolotov"),
        ThrownMolotov {
            owner: peer,
            pos: body.pos,
            rot: body.rot,
        },
        MolotovSim {
            body,
            owner: peer,
            lobby: lobby_e,
        },
        Replicate::to_clients(NetworkTarget::Only(members.clone())),
        InterpolationTarget::to_clients(NetworkTarget::Only(members)),
    ));
    info!("{peer:?} threw a molotov");
}

/// Fly every bottle one tick; one that touched something breaks into a fire
/// (and the whole lobby hears it burst there).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn step_molotovs(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    colliders: Res<MapColliders>,
    lobbies: Query<&Lobby>,
    zombies: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat)>,
    mut molotovs: Query<(Entity, &mut MolotovSim, &mut ThrownMolotov)>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let server = server.into_inner();
    for (entity, mut sim, mut replicated) in &mut molotovs {
        let Ok(lobby) = lobbies.get(sim.lobby) else {
            continue; // `cull_orphans` removes it
        };
        if lobby.paused {
            continue;
        }
        let targets: Vec<Target> = zombies
            .iter()
            .filter(|(id, _, lp, c)| is_bot_peer(id.0) && c.alive && lp.lobby == sim.lobby)
            .map(|(id, pose, ..)| {
                let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
                Target {
                    id: id.0.to_bits(),
                    body: Capsule::standing(feet, PLAYER_HEIGHT, PLAYER_RADIUS),
                    head: Capsule::head(feet, PLAYER_HEIGHT, 0.12),
                }
            })
            .collect();
        let world = &colliders.for_lobby(lobby);
        if let Some(shatter) = sim.body.step(dt, world, &targets) {
            commands.entity(entity).try_despawn();
            let spots = fire_spots(shatter, world);
            let members = lobby.real_peers();
            let burst = MolotovBurst {
                point: shatter.point.to_array(),
            };
            if let Err(e) = sender.send::<_, GameChannel>(
                &burst,
                server,
                &NetworkTarget::Only(members.clone()),
            ) {
                error!("failed to send molotov burst: {e:?}");
            }
            info!("{:?}'s molotov broke into {} fire spots", sim.owner, spots.len());
            commands.spawn((
                Name::from("MolotovFire"),
                MolotovFire {
                    owner: sim.owner,
                    center: shatter.point,
                    spots: spots.clone(),
                },
                FireSim {
                    owner: sim.owner,
                    lobby: sim.lobby,
                    spots,
                    age: 0.0,
                    // The first tick lands right away.
                    since_tick: FIRE_TICK_SECS,
                },
                Replicate::to_clients(NetworkTarget::Only(members)),
            ));
            continue;
        }
        if sim.body.finished() {
            commands.entity(entity).try_despawn();
            continue;
        }
        let next = ThrownMolotov {
            owner: sim.owner,
            pos: sim.body.pos,
            rot: sim.body.rot,
        };
        if *replicated != next {
            *replicated = next;
        }
    }
}

/// Burn whoever's standing in a fire — zombies, and the thrower in their own
/// — a small hit every [`FIRE_TICK_SECS`]; put fires out once they've burnt
/// for [`FIRE_SECS`].
fn burn(
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<&Lobby>,
    bodies: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, Option<&PlayerCombat>)>,
    mut fires: Query<(Entity, &mut FireSim)>,
    mut hits: EventWriter<PlayerHit>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut fire) in &mut fires {
        let Ok(lobby) = lobbies.get(fire.lobby) else {
            continue;
        };
        if lobby.paused || endings.is_ending(fire.lobby) {
            continue;
        }
        fire.age += dt;
        if fire.age >= FIRE_SECS {
            commands.entity(entity).try_despawn();
            continue;
        }
        fire.since_tick += dt;
        if fire.since_tick < FIRE_TICK_SECS {
            continue;
        }
        fire.since_tick -= FIRE_TICK_SECS;
        for (id, pose, lp, combat) in &bodies {
            if lp.lobby != fire.lobby || combat.is_some_and(|c| !c.alive) {
                continue;
            }
            let damage = if is_bot_peer(id.0) {
                ZOMBIE_TICK_DAMAGE
            } else if id.0 == fire.owner {
                SELF_TICK_DAMAGE
            } else {
                continue; // never the rest of the lobby
            };
            let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
            if !in_fire(feet, &fire.spots) {
                continue;
            }
            hits.write(PlayerHit {
                victim: id.0,
                killer: fire.owner,
                damage,
                bomb_shot: false,
                blast: false,
                critical: false,
            });
        }
    }
}

/// Maybe drop a molotov beside each zombie a player killed.
fn roll_drops(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    mut kills: EventReader<ZombieKilled>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (i, ev) in kills.read().enumerate() {
        let Ok(lobby) = lobbies.get(ev.lobby) else {
            continue;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            continue;
        }
        // A different scramble from the power-up roll's, so the two drops
        // don't always come together.
        let seed = (time.elapsed().as_nanos() as u64)
            .wrapping_mul(0x2545_F491_4F6C_DD1D)
            ^ (ev.feet.x.to_bits() as u64).rotate_left(21)
            ^ (ev.feet.z.to_bits() as u64).rotate_left(43)
            ^ (i as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
        let chance = if lobby.molotov_test { 1.0 } else { DROP_CHANCE };
        if shared::bots::rand01(seed) >= chance {
            continue;
        }
        let side = shared::bots::rand01(seed ^ 0x5851_F42D_4C95_7F2D);
        let pos = drop_spot(ev.feet, side, &colliders.for_lobby(lobby));
        let yaw = shared::bots::rand01(seed.rotate_left(17)) * std::f32::consts::TAU;
        commands.spawn((
            Name::from("MolotovDrop"),
            MolotovDrop { pos, yaw },
            DropSim {
                lobby: ev.lobby,
                age: 0.0,
            },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
        info!("lobby {:?}: a zombie dropped a molotov", ev.lobby);
    }
}

/// Age the dropped molotovs (not while paused) and remove old ones.
fn age_drops(
    time: Res<Time>,
    lobbies: Query<&Lobby>,
    mut drops: Query<(Entity, &mut DropSim)>,
    mut commands: Commands,
) {
    for (entity, mut sim) in &mut drops {
        if lobbies.get(sim.lobby).is_ok_and(|l| l.paused) {
            continue;
        }
        sim.age += time.delta_secs();
        if sim.age >= DROP_LINGER_SECS {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Leave `count` dropped molotovs spread around `feet` — the ones a player
/// carried when they picked up a throwing knife. They linger like a
/// zombie's drop.
pub(crate) fn drop_around(
    commands: &mut Commands,
    lobby_e: Entity,
    lobby: &Lobby,
    feet: Vec3,
    count: u32,
    world: &dyn shared::map::CollisionWorld,
) {
    for i in 0..count {
        let seed = (i as f32 + 0.5) / count as f32;
        commands.spawn((
            Name::from("MolotovDrop"),
            MolotovDrop {
                pos: drop_spot(feet, seed, world),
                yaw: seed * std::f32::consts::TAU,
            },
            DropSim {
                lobby: lobby_e,
                age: 0.0,
            },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
    }
}

/// A client asked to pick up the dropped molotov nearest them — dropping any
/// throwing knives they were carrying instead.
#[allow(clippy::too_many_arguments)]
fn on_pick_up_molotov(
    trigger: Trigger<RemoteTrigger<PickUpMolotov>>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    drops: Query<(Entity, &DropSim, &MolotovDrop)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let Some((lobby_e, lobby)) = zombies_lobby(&lobbies, peer) else {
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
    let nearest = drops
        .iter()
        .filter(|(_, sim, d)| sim.lobby == lobby_e && in_pickup_range(feet, eye, d.pos))
        .min_by(|a, b| a.2.pos.distance_squared(eye).total_cmp(&b.2.pos.distance_squared(eye)));
    let Some((entity, ..)) = nearest else {
        return;
    };
    commands.entity(entity).try_despawn();
    let drop = trigger.trigger.drop_knives;
    if drop > 0 {
        crate::knives::drop_knives_around(
            &mut commands,
            lobby_e,
            lobby,
            peer,
            feet,
            drop,
            &colliders.for_lobby(lobby),
        );
    }
    if let Err(e) = sender.send::<_, GameChannel>(
        &MolotovPickedUp,
        server.into_inner(),
        &NetworkTarget::Single(peer),
    ) {
        error!("failed to send molotov pickup: {e:?}");
    }
    info!("{peer:?} picked up a molotov");
}

/// Debug: the leader makes every zombie kill drop a molotov (or not).
fn on_set_molotov_test(
    trigger: Trigger<RemoteTrigger<SetMolotovTest>>,
    mut lobbies: Query<&mut Lobby>,
) {
    let peer = trigger.from;
    let on = trigger.trigger.on;
    if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.leader == peer) {
        if lobby.molotov_test != on {
            lobby.molotov_test = on;
            info!("lobby molotov drops {} by {peer:?} (debug)", if on { "always on" } else { "back to normal" });
        }
    }
}

/// Remove bottles, fires and drops whose lobby's game has ended or gone.
fn cull_orphans(
    lobbies: Query<&Lobby>,
    molotovs: Query<(Entity, &MolotovSim)>,
    fires: Query<(Entity, &FireSim)>,
    drops: Query<(Entity, &DropSim)>,
    mut commands: Commands,
) {
    let ended = |lobby: Entity| lobbies.get(lobby).map(|l| !l.started).unwrap_or(true);
    let doomed = molotovs
        .iter()
        .filter(|(_, s)| ended(s.lobby))
        .map(|(e, _)| e)
        .chain(fires.iter().filter(|(_, s)| ended(s.lobby)).map(|(e, _)| e))
        .chain(drops.iter().filter(|(_, s)| ended(s.lobby)).map(|(e, _)| e))
        .collect::<Vec<_>>();
    for e in doomed {
        commands.entity(e).try_despawn();
    }
}
