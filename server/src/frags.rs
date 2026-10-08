//! Frags (`Zombies` only, `shared::frag`): the server owns the flight, the
//! fuse and the drops.
//!
//! * A client's [`ThrowFrag`] (sent as it leaves the hand, with what's left
//!   of the fuse its pin-pull started) becomes a [`ThrownFrag`] replicated
//!   to the lobby, stepped every tick — bouncing, rolling to a stop — until
//!   its fuse runs out, when it blows up: the Bomb Shot's blast, but with
//!   the frag's damage and sound ([`BombBlast::frag`]), hurting zombies and
//!   its thrower, never another player (`pvp::apply_bomb_blasts`). One
//!   cooked off in the hand goes off where it was at once.
//! * A zombie a player kills sometimes drops one ([`FragDrop`]); any living
//!   player in reach can pick it up ([`PickUpFrag`]).
//!
//! Everything is removed once its lobby's game isn't running.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::frag::{FragBody, DROP_CHANCE, FUSE_SECS};
use shared::molotov::{drop_spot, DROP_LINGER_SECS};
use shared::throwing_knife::in_server_pickup_range;
use shared::{FragDrop, FragPickedUp, GameChannel, GameMode, Lobby, PickUpFrag, PlayerId, PlayerPose, ThrowFrag, ThrownFrag};

use crate::collision::MapColliders;
use crate::pvp::{BombBlast, PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;

/// Least time (s) between one player's throws.
const MIN_THROW_INTERVAL_SECS: f32 = 0.4;
/// A throw's reported origin further than this (m) from the player's eye is
/// replaced by the eye.
const MAX_ORIGIN_DRIFT: f32 = 3.0;

/// The server side of one [`ThrownFrag`].
#[derive(Component)]
struct FragSim {
    body: FragBody,
    owner: PeerId,
    lobby: Entity,
    /// Seconds of its fuse left (held while paused).
    fuse: f32,
}

/// The server side of one [`FragDrop`].
#[derive(Component)]
struct DropSim {
    lobby: Entity,
    age: f32,
}

/// When each player last threw one (`Time::elapsed_secs`).
#[derive(Resource, Default)]
struct LastThrow(HashMap<PeerId, f32>);

pub struct FragsPlugin;

impl Plugin for FragsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastThrow>()
            .add_observer(on_throw_frag)
            .add_observer(on_pick_up_frag)
            .add_systems(FixedUpdate, step_frags.before(crate::pvp::apply_player_hits))
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

/// A client's frag left their hand (or went off in it): validate, then
/// launch it with the fuse it has left. (How many they have left is
/// client-side, like the rest of their ammo.)
fn on_throw_frag(
    trigger: Trigger<RemoteTrigger<ThrowFrag>>,
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
    if !dir.is_finite() || (!req.in_hand && dir.length_squared() < 1e-6) {
        return;
    }
    let mut origin = Vec3::from_array(req.origin);
    if !origin.is_finite() || origin.distance(pose.translation) > MAX_ORIGIN_DRIFT {
        origin = pose.translation;
    }
    last.0.insert(peer, now);

    let (body, fuse) = if req.in_hand {
        (FragBody::in_hand(origin), 0.0)
    } else {
        let fuse = if req.fuse.is_finite() { req.fuse.clamp(0.0, FUSE_SECS) } else { FUSE_SECS };
        (FragBody::thrown(origin, dir), fuse)
    };
    let members = lobby.real_peers();
    commands.spawn((
        Name::from("ThrownFrag"),
        ThrownFrag {
            owner: peer,
            pos: body.pos,
            rot: body.rot,
        },
        FragSim {
            body,
            owner: peer,
            lobby: lobby_e,
            fuse,
        },
        Replicate::to_clients(NetworkTarget::Only(members.clone())),
        InterpolationTarget::to_clients(NetworkTarget::Only(members)),
    ));
    if req.in_hand {
        info!("{peer:?} cooked a frag off in their hand");
    } else {
        info!("{peer:?} threw a frag ({fuse:.2} s left)");
    }
}

/// Fly every frag one tick, run its fuse (held while paused) and blow it up
/// when it's out.
fn step_frags(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<&Lobby>,
    mut frags: Query<(Entity, &mut FragSim, &mut ThrownFrag)>,
    mut blasts: EventWriter<BombBlast>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut sim, mut replicated) in &mut frags {
        let Ok(lobby) = lobbies.get(sim.lobby) else {
            continue; // `cull_orphans` removes it
        };
        if lobby.paused || endings.is_ending(sim.lobby) {
            continue;
        }
        sim.fuse -= dt;
        if sim.fuse <= 0.0 {
            commands.entity(entity).try_despawn();
            blasts.write(BombBlast {
                cause: crate::pvp::HitCause::Frag,
                lobby: sim.lobby,
                feet: sim.body.pos,
                by: sim.owner,
                phd: false,
                frag: true,
            });
            info!("{:?}'s frag went off", sim.owner);
            continue;
        }
        sim.body.step(dt, &colliders.for_lobby(lobby));
        if sim.body.lost() {
            commands.entity(entity).try_despawn();
            continue;
        }
        let next = ThrownFrag {
            owner: sim.owner,
            pos: sim.body.pos,
            rot: sim.body.rot,
        };
        if *replicated != next {
            *replicated = next;
        }
    }
}

/// Leave `count` dropped frags spread around `feet` — the ones a player
/// carried when they swapped to another lethal.
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
            Name::from("FragDrop"),
            FragDrop {
                pos: drop_spot(feet, seed, world),
                yaw: seed * std::f32::consts::TAU,
            },
            DropSim { lobby: lobby_e, age: 0.0 },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
    }
}

/// A zombie a player killed sometimes drops a frag beside its body.
fn roll_drops(
    mut kills: EventReader<ZombieKilled>,
    colliders: Res<MapColliders>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (i, ev) in kills.read().enumerate() {
        let Ok(lobby) = lobbies.get(ev.lobby) else { continue };
        let seed = (ev.feet.x.to_bits() as u64).rotate_left(17)
            ^ (ev.feet.z.to_bits() as u64).rotate_left(41)
            ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ 0x6672_6167;
        if shared::bots::rand01(seed) >= DROP_CHANCE {
            continue;
        }
        let side = shared::bots::rand01(seed ^ 0x2545_F491_4F6C_DD1D);
        let pos = drop_spot(ev.feet, side, &colliders.for_lobby(lobby));
        let yaw = shared::bots::rand01(seed.rotate_left(29)) * std::f32::consts::TAU;
        commands.spawn((
            Name::from("FragDrop"),
            FragDrop { pos, yaw },
            DropSim { lobby: ev.lobby, age: 0.0 },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
        info!("lobby {:?}: a zombie dropped a frag", ev.lobby);
    }
}

/// A client asked to pick up the dropped frag nearest them — dropping what
/// they carried of another kind.
#[allow(clippy::too_many_arguments)]
fn on_pick_up_frag(
    trigger: Trigger<RemoteTrigger<PickUpFrag>>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    drops: Query<(Entity, &DropSim, &FragDrop)>,
    mut commands: Commands,
    mut quotes: EventWriter<crate::quotes::SayQuote>,
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
        shared::lethal::LethalKind::Frag,
        &colliders.for_lobby(lobby),
    );
    if let Err(e) = sender.send::<_, GameChannel>(&FragPickedUp, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send frag pickup: {e:?}");
    }
    quotes.write(crate::quotes::SayQuote::by(lobby_e, peer, shared::quotes::Quote::PickupEquipment));
    info!("{peer:?} picked up a frag");
}

/// Age the dropped frags (not while paused) and remove old ones.
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

/// Remove frags and drops whose lobby's game has ended or gone.
fn cull_orphans(
    lobbies: Query<&Lobby>,
    frags: Query<(Entity, &FragSim)>,
    drops: Query<(Entity, &DropSim)>,
    mut commands: Commands,
) {
    let ended = |lobby: Entity| lobbies.get(lobby).map(|l| !l.started).unwrap_or(true);
    let doomed: Vec<Entity> = frags
        .iter()
        .filter(|(_, s)| ended(s.lobby))
        .map(|(e, _)| e)
        .chain(drops.iter().filter(|(_, s)| ended(s.lobby)).map(|(e, _)| e))
        .collect();
    for e in doomed {
        commands.entity(e).try_despawn();
    }
}
