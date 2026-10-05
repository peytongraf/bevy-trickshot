//! Monkey bombs (`Zombies` only, `shared::monkey_bomb`): the server owns the
//! flight, the fuse, the lure and the drops.
//!
//! * A client's [`ThrowMonkey`] (sent once its prime's run out and the throw
//!   reaches the release point) becomes a [`ThrownMonkey`] replicated to the
//!   lobby, stepped every tick — bouncing off walls — until it settles on a
//!   floor.
//! * Down, it's `landed`: every zombie within [`LURE_RADIUS`] goes to it
//!   instead of a player ([`MonkeyLures`], read by `ai::drive_bots`), and
//!   after its fuse ([`FUSE_SECS`] unless the debug panel's set the lobby's) it blows up —
//!   a Bomb Shot's blast ([`BombBlast`]), credited to its thrower.
//! * A zombie a player kills sometimes drops one ([`MonkeyDrop`]); any
//!   living player in reach can pick it up ([`PickUpMonkey`]).
//!
//! Everything is removed once its lobby's game isn't running, and the lures
//! are rebuilt every tick from the monkeys there are.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::molotov::{drop_spot, DROP_LINGER_SECS};
use shared::monkey_bomb::{MonkeyBody, DROP_CHANCE, FUSE_SECS, LURE_RADIUS, MAX_FUSE_SECS, MIN_FUSE_SECS};
use shared::throwing_knife::in_pickup_range;
use shared::{
    GameChannel, GameMode, Lobby, MonkeyDrop, MonkeyPickedUp, PickUpMonkey, PlayerId, PlayerPose, SetMonkeyFuse,
    ThrowMonkey, ThrownMonkey,
};

use crate::collision::MapColliders;
use crate::pvp::{BombBlast, PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;

/// Least time (s) between one player's throws.
const MIN_THROW_INTERVAL_SECS: f32 = 0.5;
/// A throw's reported origin further than this (m) from the player's eye is
/// replaced by the eye.
const MAX_ORIGIN_DRIFT: f32 = 3.0;

/// The server side of one [`ThrownMonkey`].
#[derive(Component)]
struct MonkeySim {
    body: MonkeyBody,
    owner: PeerId,
    lobby: Entity,
    /// Seconds since it landed (not counting pauses), once it has.
    landed: Option<f32>,
    /// Seconds from landing to blowing up.
    fuse: f32,
}

/// Each lobby's monkey bomb fuse, when the debug panel's changed it from
/// [`FUSE_SECS`] ([`SetMonkeyFuse`]). Gone with the lobby.
#[derive(Resource, Default)]
struct MonkeyFuses(HashMap<Entity, f32>);

/// The server side of one [`MonkeyDrop`].
#[derive(Component)]
struct DropSim {
    lobby: Entity,
    age: f32,
}

/// Every landed monkey bomb — its lobby and where it stands — for the
/// zombies to go to (`ai::drive_bots`).
#[derive(Resource, Default)]
pub(crate) struct MonkeyLures(Vec<(Entity, Vec3)>);

impl MonkeyLures {
    /// A landed monkey in `lobby` at `at` (for tests).
    #[cfg(test)]
    pub(crate) fn add(&mut self, lobby: Entity, at: Vec3) {
        self.0.push((lobby, at));
    }

    /// The landed monkey in `lobby` nearest `feet`, if one's within
    /// [`LURE_RADIUS`].
    pub(crate) fn nearest(&self, lobby: Entity, feet: Vec3) -> Option<Vec3> {
        self.0
            .iter()
            .filter(|(l, at)| *l == lobby && at.distance(feet) <= LURE_RADIUS)
            .map(|(_, at)| *at)
            .min_by(|a, b| a.distance_squared(feet).total_cmp(&b.distance_squared(feet)))
    }
}

/// When each player last threw one (`Time::elapsed_secs`).
#[derive(Resource, Default)]
struct LastThrow(HashMap<PeerId, f32>);

pub struct MonkeyBombsPlugin;

impl Plugin for MonkeyBombsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastThrow>()
            .init_resource::<MonkeyLures>()
            .init_resource::<MonkeyFuses>()
            .add_observer(on_set_monkey_fuse)
            .add_observer(on_throw_monkey)
            .add_observer(on_pick_up_monkey)
            .add_systems(FixedUpdate, step_monkeys.before(crate::pvp::apply_player_hits))
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

/// Debug: a member sets how long their lobby's monkey bombs take to blow up.
fn on_set_monkey_fuse(
    trigger: Trigger<RemoteTrigger<SetMonkeyFuse>>,
    lobbies: Query<(Entity, &Lobby)>,
    mut fuses: ResMut<MonkeyFuses>,
) {
    let peer = trigger.from;
    let secs = trigger.trigger.secs;
    if !secs.is_finite() {
        return;
    }
    if let Some((lobby_e, _)) = lobbies.iter().find(|(_, l)| l.has(peer)) {
        let secs = secs.clamp(MIN_FUSE_SECS, MAX_FUSE_SECS);
        fuses.0.insert(lobby_e, secs);
        info!("{peer:?} set the monkey bomb fuse to {secs:.2} s (debug)");
    }
}

/// A client threw a monkey bomb: validate, then launch it. (How many they
/// have left is client-side, like the rest of their ammo.)
fn on_throw_monkey(
    trigger: Trigger<RemoteTrigger<ThrowMonkey>>,
    time: Res<Time>,
    mut last: ResMut<LastThrow>,
    fuses: Res<MonkeyFuses>,
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

    let body = MonkeyBody::thrown(origin, dir);
    let fuse = fuses.0.get(&lobby_e).copied().unwrap_or(FUSE_SECS);
    let members = lobby.real_peers();
    commands.spawn((
        Name::from("ThrownMonkey"),
        ThrownMonkey {
            owner: peer,
            pos: body.pos,
            rot: body.rot,
            landed: false,
            fuse,
        },
        MonkeySim {
            body,
            owner: peer,
            lobby: lobby_e,
            landed: None,
            fuse,
        },
        Replicate::to_clients(NetworkTarget::Only(members.clone())),
        InterpolationTarget::to_clients(NetworkTarget::Only(members)),
    ));
    info!("{peer:?} threw a monkey bomb");
}

/// Fly every monkey one tick until it lands; run each landed one's fuse
/// (held while paused) and blow it up at the end; and list where the landed
/// ones stand, for the zombies.
fn step_monkeys(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<&Lobby>,
    mut lures: ResMut<MonkeyLures>,
    mut monkeys: Query<(Entity, &mut MonkeySim, &mut ThrownMonkey)>,
    mut blasts: EventWriter<BombBlast>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    lures.0.clear();
    for (entity, mut sim, mut replicated) in &mut monkeys {
        let Ok(lobby) = lobbies.get(sim.lobby) else {
            continue; // `cull_orphans` removes it
        };
        if lobby.paused || endings.is_ending(sim.lobby) {
            if sim.landed.is_some() {
                lures.0.push((sim.lobby, sim.body.pos));
            }
            continue;
        }
        match sim.landed {
            Some(secs) => {
                let secs = secs + dt;
                sim.landed = Some(secs);
                if secs >= sim.fuse {
                    commands.entity(entity).try_despawn();
                    blasts.write(BombBlast {
                        lobby: sim.lobby,
                        feet: sim.body.pos,
                        by: sim.owner,
                        phd: false,
                    });
                    info!("{:?}'s monkey bomb went off", sim.owner);
                    continue;
                }
                lures.0.push((sim.lobby, sim.body.pos));
            }
            None => {
                let world = &colliders.for_lobby(lobby);
                if let Some((at, facing)) = sim.body.step(dt, world) {
                    sim.body.pos = at;
                    sim.body.rot = facing;
                    sim.landed = Some(0.0);
                    lures.0.push((sim.lobby, at));
                } else if sim.body.lost() {
                    commands.entity(entity).try_despawn();
                    continue;
                }
            }
        }
        let next = ThrownMonkey {
            owner: sim.owner,
            pos: sim.body.pos,
            rot: sim.body.rot,
            landed: sim.landed.is_some(),
            fuse: sim.fuse,
        };
        if *replicated != next {
            *replicated = next;
        }
    }
}

/// Leave `count` dropped monkey bombs spread around `feet` — the ones a
/// player carried when they swapped to another lethal.
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
            Name::from("MonkeyDrop"),
            MonkeyDrop {
                pos: drop_spot(feet, seed, world),
                yaw: seed * std::f32::consts::TAU,
            },
            DropSim { lobby: lobby_e, age: 0.0 },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
    }
}

/// A zombie a player killed sometimes drops a monkey bomb beside its body.
fn roll_drops(
    mut kills: EventReader<ZombieKilled>,
    colliders: Res<MapColliders>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (i, ev) in kills.read().enumerate() {
        let Ok(lobby) = lobbies.get(ev.lobby) else { continue };
        let seed = (ev.feet.x.to_bits() as u64).rotate_left(13)
            ^ (ev.feet.z.to_bits() as u64).rotate_left(37)
            ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ 0x6D6F_6E6B_6579;
        if shared::bots::rand01(seed) >= DROP_CHANCE {
            continue;
        }
        let side = shared::bots::rand01(seed ^ 0x2545_F491_4F6C_DD1D);
        let pos = drop_spot(ev.feet, side, &colliders.for_lobby(lobby));
        let yaw = shared::bots::rand01(seed.rotate_left(29)) * std::f32::consts::TAU;
        commands.spawn((
            Name::from("MonkeyDrop"),
            MonkeyDrop { pos, yaw },
            DropSim { lobby: ev.lobby, age: 0.0 },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
        info!("lobby {:?}: a zombie dropped a monkey bomb", ev.lobby);
    }
}

/// A client asked to pick up the dropped monkey bomb nearest them —
/// dropping what they carried of another kind.
#[allow(clippy::too_many_arguments)]
fn on_pick_up_monkey(
    trigger: Trigger<RemoteTrigger<PickUpMonkey>>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<(Entity, &Lobby)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    drops: Query<(Entity, &DropSim, &MonkeyDrop)>,
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
    crate::lethals::drop_carried(
        &mut commands,
        lobby_e,
        lobby,
        peer,
        feet,
        trigger.trigger.dropping,
        shared::lethal::LethalKind::MonkeyBomb,
        &colliders.for_lobby(lobby),
    );
    if let Err(e) = sender.send::<_, GameChannel>(&MonkeyPickedUp, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send monkey bomb pickup: {e:?}");
    }
    info!("{peer:?} picked up a monkey bomb");
}

/// Age the dropped monkey bombs (not while paused) and remove old ones.
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

/// Remove monkeys and drops whose lobby's game has ended or gone.
fn cull_orphans(
    lobbies: Query<&Lobby>,
    monkeys: Query<(Entity, &MonkeySim)>,
    drops: Query<(Entity, &DropSim)>,
    mut fuses: ResMut<MonkeyFuses>,
    mut commands: Commands,
) {
    fuses.0.retain(|lobby, _| lobbies.contains(*lobby));
    let ended = |lobby: Entity| lobbies.get(lobby).map(|l| !l.started).unwrap_or(true);
    let doomed: Vec<Entity> = monkeys
        .iter()
        .filter(|(_, s)| ended(s.lobby))
        .map(|(e, _)| e)
        .chain(drops.iter().filter(|(_, s)| ended(s.lobby)).map(|(e, _)| e))
        .collect();
    for e in doomed {
        commands.entity(e).try_despawn();
    }
}
