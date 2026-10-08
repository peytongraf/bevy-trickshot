//! Flash bangs (`Zombies` only, `shared::flash_bang` — a tactical): the
//! server owns the flight, the fuse, the stun and the drops.
//!
//! * A client's [`ThrowFlashBang`] becomes a [`ThrownFlashBang`] replicated
//!   to the lobby, stepped every tick like a frag (bouncing, rolling to a
//!   stop) until its fixed fuse runs out. Then it's gone, every zombie within
//!   [`STUN_RADIUS`] is stunned for [`STUN_SECS`] (slowed, no attacking —
//!   `ai::BotBrain::stun`), and the lobby's told ([`FlashBangDetonated`]) for
//!   the flash, its sound and its thrower's white-out. It hurts nobody.
//! * A zombie a player kills sometimes drops one ([`FlashBangDrop`]); any
//!   living player in reach can pick it up ([`PickUpFlashBang`]) — dropping
//!   the other tactical they carried, if any.
//!
//! Everything is removed once its lobby's game isn't running.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::flash_bang::{DROP_CHANCE, FUSE_SECS, STUN_RADIUS, STUN_SECS};
use shared::frag::FragBody;
use shared::molotov::{drop_spot, DROP_LINGER_SECS};
use shared::throwing_knife::in_server_pickup_range;
use shared::{
    FlashBangDetonated, FlashBangDrop, FlashBangPickedUp, GameChannel, GameMode, Lobby, PickUpFlashBang, PlayerId,
    PlayerPose, ThrowFlashBang, ThrownFlashBang,
};

use crate::ai::BotBrain;
use crate::collision::MapColliders;
use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;
use crate::zombies::Zombie;

/// Least time (s) between one player's throws.
const MIN_THROW_INTERVAL_SECS: f32 = 0.4;
/// A throw's reported origin further than this (m) from the player's eye is
/// replaced by the eye.
const MAX_ORIGIN_DRIFT: f32 = 3.0;

/// The server side of one [`ThrownFlashBang`].
#[derive(Component)]
struct FlashSim {
    body: FragBody,
    owner: PeerId,
    lobby: Entity,
    /// Seconds of its fuse left (held while paused).
    fuse: f32,
}

/// The server side of one [`FlashBangDrop`].
#[derive(Component)]
struct DropSim {
    lobby: Entity,
    age: f32,
}

/// When each player last threw one (`Time::elapsed_secs`).
#[derive(Resource, Default)]
struct LastThrow(HashMap<PeerId, f32>);

pub struct FlashBangsPlugin;

impl Plugin for FlashBangsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastThrow>()
            .add_observer(on_throw_flash_bang)
            .add_observer(on_pick_up_flash_bang)
            .add_systems(FixedUpdate, step_flash_bangs.before(crate::ai::drive_bots))
            .add_systems(
                FixedUpdate,
                (roll_drops, age_drops, cull_orphans)
                    .chain()
                    .after(crate::pvp::apply_player_hits),
            );
    }
}

/// The running `Zombies` lobby `peer` is playing in.
fn zombies_lobby<'a>(lobbies: &'a Query<(Entity, &Lobby)>, peer: PeerId) -> Option<(Entity, &'a Lobby)> {
    lobbies
        .iter()
        .find(|(_, l)| l.started && !l.paused && l.mode == GameMode::Zombies && l.has(peer))
}

/// A client threw a flash bang: validate, then launch it. (How many they
/// have left is client-side, like the rest of their equipment.)
fn on_throw_flash_bang(
    trigger: Trigger<RemoteTrigger<ThrowFlashBang>>,
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

    let body = FragBody::thrown(origin, dir);
    let members = lobby.real_peers();
    commands.spawn((
        Name::from("ThrownFlashBang"),
        ThrownFlashBang {
            owner: peer,
            pos: body.pos,
            rot: body.rot,
        },
        FlashSim {
            body,
            owner: peer,
            lobby: lobby_e,
            fuse: FUSE_SECS,
        },
        Replicate::to_clients(NetworkTarget::Only(members.clone())),
        InterpolationTarget::to_clients(NetworkTarget::Only(members)),
    ));
    info!("{peer:?} threw a flash bang");
}

/// Fly every flash bang one tick and run its fuse (held while paused); when
/// it's out, stun the zombies near it and tell the lobby.
#[allow(clippy::too_many_arguments)]
fn step_flash_bangs(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<&Lobby>,
    mut flashes: Query<(Entity, &mut FlashSim, &mut ThrownFlashBang)>,
    mut zombies: Query<(&LobbyPlayer, &PlayerPose, &PlayerCombat, &mut BotBrain), With<Zombie>>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for (entity, mut sim, mut replicated) in &mut flashes {
        let Ok(lobby) = lobbies.get(sim.lobby) else {
            continue; // `cull_orphans` removes it
        };
        if lobby.paused || endings.is_ending(sim.lobby) {
            continue;
        }
        sim.fuse -= dt;
        if sim.fuse <= 0.0 {
            commands.entity(entity).try_despawn();
            let at = sim.body.pos;
            let mut n = 0;
            for (lp, pose, combat, mut brain) in &mut zombies {
                if lp.lobby != sim.lobby || !combat.alive {
                    continue;
                }
                let middle = pose.translation - Vec3::Y * (EYE_HEIGHT * 0.5);
                if middle.distance(at) <= STUN_RADIUS {
                    brain.stun(now + STUN_SECS);
                    n += 1;
                }
            }
            let msg = FlashBangDetonated {
                owner: sim.owner,
                at: at.to_array(),
            };
            if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers())) {
                error!("failed to send flash bang: {e:?}");
            }
            info!("{:?}'s flash bang went off, stunning {n} zombies", sim.owner);
            continue;
        }
        sim.body.step(dt, &colliders.for_lobby(lobby));
        if sim.body.lost() {
            commands.entity(entity).try_despawn();
            continue;
        }
        let next = ThrownFlashBang {
            owner: sim.owner,
            pos: sim.body.pos,
            rot: sim.body.rot,
        };
        if *replicated != next {
            *replicated = next;
        }
    }
}

/// Leave `count` dropped flash bangs spread around `feet` — the ones a
/// player carried when they swapped to another tactical.
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
            Name::from("FlashBangDrop"),
            FlashBangDrop {
                pos: drop_spot(feet, seed, world),
                yaw: seed * std::f32::consts::TAU,
            },
            DropSim { lobby: lobby_e, age: 0.0 },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
    }
}

/// A zombie a player killed sometimes drops a flash bang beside its body.
fn roll_drops(
    mut kills: EventReader<ZombieKilled>,
    colliders: Res<MapColliders>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (i, ev) in kills.read().enumerate() {
        let Ok(lobby) = lobbies.get(ev.lobby) else { continue };
        let seed = (ev.feet.x.to_bits() as u64).rotate_left(23)
            ^ (ev.feet.z.to_bits() as u64).rotate_left(47)
            ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ 0x666C_6173_68;
        if shared::bots::rand01(seed) >= DROP_CHANCE {
            continue;
        }
        let side = shared::bots::rand01(seed ^ 0x2545_F491_4F6C_DD1D);
        let pos = drop_spot(ev.feet, side, &colliders.for_lobby(lobby));
        let yaw = shared::bots::rand01(seed.rotate_left(29)) * std::f32::consts::TAU;
        commands.spawn((
            Name::from("FlashBangDrop"),
            FlashBangDrop { pos, yaw },
            DropSim { lobby: ev.lobby, age: 0.0 },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
        info!("lobby {:?}: a zombie dropped a flash bang", ev.lobby);
    }
}

/// A client asked to pick up the dropped flash bang nearest them —
/// dropping the other tactical they carried.
#[allow(clippy::too_many_arguments)]
fn on_pick_up_flash_bang(
    trigger: Trigger<RemoteTrigger<PickUpFlashBang>>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    drops: Query<(Entity, &DropSim, &FlashBangDrop)>,
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
        .filter(|(_, sim, d)| sim.lobby == lobby_e && in_server_pickup_range(feet, eye, d.pos))
        .min_by(|a, b| a.2.pos.distance_squared(eye).total_cmp(&b.2.pos.distance_squared(eye)));
    let Some((entity, ..)) = nearest else {
        return;
    };
    commands.entity(entity).try_despawn();
    crate::lethals::drop_carried(
        &mut commands,
        lobby_e,
        lobby,
        peer,
        feet,
        trigger.trigger.dropping,
        shared::lethal::LethalKind::FlashBang,
        &colliders.for_lobby(lobby),
    );
    if let Err(e) = sender.send::<_, GameChannel>(&FlashBangPickedUp, server.into_inner(), &NetworkTarget::Single(peer))
    {
        error!("failed to send flash bang pickup: {e:?}");
    }
    info!("{peer:?} picked up a flash bang");
}

/// Age the dropped flash bangs (not while paused) and remove old ones.
fn age_drops(time: Res<Time>, lobbies: Query<&Lobby>, mut drops: Query<(Entity, &mut DropSim)>, mut commands: Commands) {
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

/// Remove flash bangs and drops whose lobby's game has ended or gone.
fn cull_orphans(
    lobbies: Query<&Lobby>,
    flashes: Query<(Entity, &FlashSim)>,
    drops: Query<(Entity, &DropSim)>,
    mut commands: Commands,
) {
    let ended = |lobby: Entity| lobbies.get(lobby).map(|l| !l.started).unwrap_or(true);
    let doomed: Vec<Entity> = flashes
        .iter()
        .filter(|(_, s)| ended(s.lobby))
        .map(|(e, _)| e)
        .chain(drops.iter().filter(|(_, s)| ended(s.lobby)).map(|(e, _)| e))
        .collect();
    for e in doomed {
        commands.entity(e).try_despawn();
    }
}
