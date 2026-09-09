//! Client-side networking: connect to the authoritative server and expose the
//! connection state to the rest of the client.
//!
//! The address the client dials is the one seam a future matchmaker would touch
//! ([`server_addr`]); everything downstream is identical no matter where that
//! address came from.
//!
//! Responsibilities: connect + hold the [`ReplicationReceiver`]; once
//! `AppState::InGame`, ship the local player's pose in the input packet
//! (`write_input`) and render every other player as a blue capsule
//! (`spawn_remote_avatars` / `follow_remote_avatars`).

use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::net::{SocketAddr, ToSocketAddrs};

use bevy::prelude::*;
use lightyear::prelude::client::{ClientPlugins, NetcodeClient, NetcodeConfig};
use lightyear::prelude::input::client::InputSet;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::*;

use shared::{
    Bot, KillCam, MatchOver, PlayerId, PlayerInput, PlayerPose, ShotOutcome, ShotResolved,
    TrickScore,
};

use crate::killcam::{self, ActiveKillCam, ReplaySoundBits};
use crate::{
    Ads, AppState, GroundImpact, MatchEndedEvent, PendingShot, Player, PlayerHead, PlayerPhysics,
    TrickScoredEvent, TrickState, WorldModelCamera, NOSCOPE_ADS_MAX,
};

/// Where a shipped build connects when `TRICKSHOT_SERVER` is unset and we're not
/// running under `cargo`. Mirrors `updater.rs`'s `DEFAULT_REPO` convention.
const DEFAULT_SERVER: &str = "bevy-trickshot-server.fly.dev:5000";

/// Resolve the server address:
/// * `TRICKSHOT_SERVER` env var wins (host:port, resolved via DNS),
/// * else `127.0.0.1:5000` under `cargo run` (dev),
/// * else [`DEFAULT_SERVER`] (shipped build).
pub fn server_addr() -> SocketAddr {
    let fallback = || SocketAddr::from(([127, 0, 0, 1], shared::DEFAULT_PORT));
    let resolve = |s: &str| s.to_socket_addrs().ok().and_then(|mut it| it.next());

    if let Ok(s) = std::env::var("TRICKSHOT_SERVER") {
        if let Some(addr) = resolve(&s) {
            return addr;
        }
        warn!("TRICKSHOT_SERVER='{s}' did not resolve; using the default instead");
    }

    let default = if std::env::var_os("CARGO").is_some() {
        format!("127.0.0.1:{}", shared::DEFAULT_PORT)
    } else {
        DEFAULT_SERVER.to_string()
    };
    resolve(&default).unwrap_or_else(fallback)
}

/// A `u64` that is distinct between two clients launched from the same machine
/// (netcode rejects a second connection with a `client_id` already in use).
fn dev_client_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    ((std::process::id() as u64) << 32) | nanos
}

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        // Order matters: client plugins, then the shared protocol, then (in a
        // `Startup` system below) the `Client` entity.
        app.add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / shared::TICK_HZ),
        });
        app.add_plugins(shared::SharedPlugin);

        app.add_systems(Startup, connect);
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);

        app.add_systems(
            FixedPreUpdate,
            write_input
                .in_set(InputSet::WriteClientInputs)
                .run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            Update,
            (
                mark_local_input,
                spawn_remote_avatars,
                follow_remote_avatars,
                spawn_bot_avatars,
                follow_bot_avatars,
                receive_shots,
                receive_trick_scores,
                receive_match_over,
                receive_killcam,
                dev_auto_fire.run_if(|| std::env::var_os("TRICKSHOT_AUTO_FIRE").is_some()),
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Marker for the single client-connection entity.
#[derive(Component)]
pub struct GameClient;

fn connect(mut commands: Commands) {
    let addr = server_addr();
    let auth = Authentication::Manual {
        server_addr: addr,
        client_id: dev_client_id(),
        private_key: shared::DEV_PRIVATE_KEY,
        protocol_id: shared::PROTOCOL_ID,
    };
    let client = match NetcodeClient::new(auth, NetcodeConfig::default()) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to build netcode client: {e:?}");
            return;
        }
    };

    // Bind the local socket in the same address family as the server, or the
    // first `sendto` fails with EAFNOSUPPORT (fly.io is IPv6-only).
    let local: SocketAddr = if addr.is_ipv6() {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    };

    let entity = commands
        .spawn((
            Name::from("GameClient"),
            GameClient,
            client,
            LocalAddr(local),
            UdpIo::default(),
            ReplicationReceiver::default(),
            PredictionManager::default(),
            InterpolationManager::default(),
        ))
        .id();
    commands.trigger_targets(Connect, entity);
    info!("connecting to server at {addr}");
}

fn on_connected(
    _t: Trigger<OnAdd, Connected>,
    q: Query<&LocalId, (With<GameClient>, With<Connected>)>,
) {
    if let Ok(local) = q.single() {
        info!("connected to server (local peer {:?})", local.0);
    } else {
        info!("connected to server");
    }
}

// --- local input --------------------------------------------------------

/// Attach the `InputMarker` to our own predicted player once it's replicated in,
/// so lightyear ships its `ActionState<PlayerInput>` to the server each tick.
#[allow(clippy::type_complexity)]
fn mark_local_input(
    local: Query<&LocalId, With<GameClient>>,
    predicted: Query<(Entity, &PlayerId), (With<Predicted>, Without<InputMarker<PlayerInput>>)>,
    mut commands: Commands,
) {
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    for (entity, id) in &predicted {
        if id.0 == me {
            commands
                .entity(entity)
                .insert(InputMarker::<PlayerInput>::default());
            info!("local player {entity:?} ready — sending input");
        }
    }
}

/// Copy this frame's local pose (client-authoritative) into the input packet,
/// and — if `weapon_system` pulled the trigger — the fire request too.
#[allow(clippy::too_many_arguments)]
fn write_input(
    player: Query<&Transform, With<Player>>,
    head: Query<&Transform, With<PlayerHead>>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    physics: Query<&PlayerPhysics, With<Player>>,
    anim_players: Query<&AnimationPlayer>,
    view_models: Query<&crate::ViewModelAnimation>,
    ads: Res<Ads>,
    shake: Res<crate::Shake>,
    sway: Res<crate::WeaponSwayState>,
    settings: Res<crate::settings::Settings>,
    mut trick: ResMut<TrickState>,
    mut snd: ResMut<ReplaySoundBits>,
    mut ground_hit: ResMut<killcam::ReplayGroundImpact>,
    mut pending: ResMut<PendingShot>,
    mut q: Query<&mut ActionState<PlayerInput>, With<InputMarker<PlayerInput>>>,
) {
    let (Ok(pt), Ok(ht), Ok(mut action)) = (player.single(), head.single(), q.single_mut()) else {
        return;
    };
    action.translation = pt.translation.to_array();
    action.yaw = pt.rotation.to_euler(EulerRot::YXZ).0;
    action.pitch = ht.rotation.to_euler(EulerRot::YXZ).1;
    action.weapon = shared::weapon::WeaponId::Sniper.as_u8();
    action.fire = false;
    // Kill-cam recording: the camera-shake state, weapon-sway offset and hip
    // FOV, so a replay can reproduce shake / recoil / sway and render at this
    // player's FOV instead of re-deriving an absolute camera transform that
    // would just duplicate `translation` / `yaw` / `pitch` above.
    action.shake_trauma = shake.trauma;
    action.shake_phase = shake.phase;
    action.shake_recoil = shake.recoil;
    action.sway_offset = sway.offset.to_array();
    action.fov_deg = settings.fov;
    action.sound_bits = std::mem::take(&mut snd.0);
    action.anim_time = crate::killcam::viewmodel_anim_time(&anim_players, &view_models);
    action.ads_t = ads.t;
    action.ground_pt = ground_hit.0.take().map(|p| p.to_array());

    // Emit the shot from the world camera's viewpoint. Keep the pending flag if
    // the camera isn't ready yet, rather than dropping the shot.
    if pending.0.is_some() {
        if let Ok(cam) = cam.single() {
            pending.0 = None;
            action.fire = true;
            action.fire_origin = cam.translation().to_array();
            action.fire_dir = cam.forward().as_vec3().to_array();
            // Trick metadata for server-side scoring, then reset for the next shot.
            let grounded = physics.single().map(|p| p.grounded).unwrap_or(true);
            action.spin_deg = trick.total_deg();
            action.airborne = trick.airborne || !grounded;
            action.noscope = ads.t <= NOSCOPE_ADS_MAX;
            trick.reset();
        }
    }
}

/// Server → everyone: a shot scored style points. Pop the yellow stack for our
/// own shooter id.
fn receive_trick_scores(
    local: Query<&LocalId, With<GameClient>>,
    mut receivers: Query<&mut MessageReceiver<TrickScore>>,
    mut scored: EventWriter<TrickScoredEvent>,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if Some(msg.shooter) != me {
                continue;
            }
            scored.write(TrickScoredEvent {
                lines: msg.lines.into_iter().map(|l| (l.label, l.points)).collect(),
            });
        }
    }
}

/// Server → everyone in the lobby: the match clock ran out.
fn receive_match_over(
    mut receivers: Query<&mut MessageReceiver<MatchOver>>,
    mut ended: EventWriter<MatchEndedEvent>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            ended.write(MatchEndedEvent {
                winner: msg.winner_name,
                score: msg.winner_score,
            });
        }
    }
}

/// Server → everyone in the lobby: the killer's last ~3 s to replay.
fn receive_killcam(
    mut receivers: Query<&mut MessageReceiver<KillCam>>,
    mut active: ResMut<ActiveKillCam>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            killcam::begin_from_message(&mut active, msg);
        }
    }
}

/// Drain the server's authoritative [`ShotResolved`] broadcasts. Right now the
/// only thing the client acts on is a `Ground` outcome, which becomes a
/// [`GroundImpact`] event so everyone sees a debris burst at the same spot.
/// Our *own* shots are skipped here — `weapon_system` already spawned that burst
/// locally the instant we fired.
fn receive_shots(
    local: Query<&LocalId, With<GameClient>>,
    mut receivers: Query<&mut MessageReceiver<ShotResolved>>,
    mut impacts: EventWriter<GroundImpact>,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if Some(msg.shooter) == me {
                continue;
            }
            if let ShotOutcome::Ground { point } = msg.outcome {
                impacts.write(GroundImpact(Vec3::from_array(point)));
            }
        }
    }
}

// --- remote players (blue capsules) -----------------------------------

/// A capsule standing in for another player; follows their interpolated pose.
#[derive(Component)]
struct RemoteAvatar {
    src: Entity,
}

/// Eye height of the local rig; the replicated pose is at eye level, the capsule
/// mesh is centred on the body.
const CAPSULE_DROP: f32 = 0.8;

/// Spawn a capsule for every interpolated (i.e. *other*-player) `PlayerPose`
/// entity that doesn't have one yet. Polled rather than an `OnAdd` observer so
/// it doesn't matter whether `Interpolated` or `PlayerPose` lands first.
fn spawn_remote_avatars(
    remotes: Query<Entity, (With<PlayerPose>, With<Interpolated>)>,
    avatars: Query<&RemoteAvatar>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let have: std::collections::HashSet<Entity> = avatars.iter().map(|a| a.src).collect();
    for src in &remotes {
        if have.contains(&src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            RemoteAvatar { src },
            Mesh3d(meshes.add(Capsule3d::new(0.35, 1.1))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.22, 0.48, 1.0),
                perceptual_roughness: 0.7,
                ..default()
            })),
            Transform::default(),
        ));
        info!("remote player {src:?} — spawned capsule");
    }
}

fn follow_remote_avatars(
    poses: Query<&PlayerPose>,
    mut avatars: Query<(Entity, &RemoteAvatar, &mut Transform)>,
    mut commands: Commands,
) {
    for (entity, avatar, mut tf) in &mut avatars {
        match poses.get(avatar.src) {
            Ok(pose) => {
                tf.translation = pose.translation - Vec3::Y * CAPSULE_DROP;
                tf.rotation = Quat::from_rotation_y(pose.yaw);
            }
            Err(_) => {
                commands.entity(entity).try_despawn();
            }
        }
    }
}

// --- bots (orange capsules that topple when shot) --------------------

/// A capsule standing in for a server-owned [`shared::Bot`].
#[derive(Component)]
struct BotAvatar {
    src: Entity,
}

const BOT_H: f32 = 1.8;
const BOT_R: f32 = 0.4;

fn spawn_bot_avatars(
    bots: Query<Entity, (With<Bot>, With<Interpolated>)>,
    avatars: Query<&BotAvatar>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let have: std::collections::HashSet<Entity> = avatars.iter().map(|a| a.src).collect();
    for src in &bots {
        if have.contains(&src) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                BotAvatar { src },
                crate::TargetBotVisual,
                Transform::default(),
                Visibility::default(),
            ))
            .with_child((
                Mesh3d(meshes.add(Capsule3d::new(BOT_R, BOT_H - 2.0 * BOT_R))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.95, 0.55, 0.15),
                    perceptual_roughness: 0.8,
                    ..default()
                })),
                // Lift so the capsule stands on its feet; the parent pivots there.
                Transform::from_xyz(0.0, BOT_H * 0.5, 0.0),
            ));
        info!("bot {src:?} — spawned capsule");
    }
}

/// Dev-only (`TRICKSHOT_AUTO_FIRE` env): aim the local rig at the nearest bot
/// and pull the trigger every ~1.2 s, so the shoot → score → respawn loop can
/// be exercised without a human clicking.
#[allow(clippy::type_complexity)]
fn dev_auto_fire(
    time: Res<Time>,
    mut cooldown: Local<f32>,
    bots: Query<&Bot>,
    mut pending: ResMut<PendingShot>,
    mut player: Query<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Query<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    *cooldown -= time.delta_secs();
    if *cooldown > 0.0 {
        return;
    }
    let (Ok(mut pt), Ok(mut ht)) = (player.single_mut(), head.single_mut()) else {
        return;
    };
    // Nearest alive bot.
    let Some(bot) = bots
        .iter()
        .filter(|b| b.alive)
        .min_by(|a, b| {
            a.pos
                .distance_squared(pt.translation)
                .total_cmp(&b.pos.distance_squared(pt.translation))
        })
        .copied()
    else {
        return;
    };
    let f = (bot.pos + Vec3::Y * (BOT_H * 0.5) - pt.translation).normalize_or_zero();
    if f == Vec3::ZERO {
        return;
    }
    pt.rotation = Quat::from_rotation_y(f.x.atan2(f.z) + core::f32::consts::PI);
    ht.rotation = Quat::from_rotation_x(f.y.clamp(-1.0, 1.0).asin());
    pending.0 = Some(());
    *cooldown = 1.2;
}

fn follow_bot_avatars(
    bots: Query<&Bot>,
    mut avatars: Query<(Entity, &BotAvatar, &mut Transform)>,
    mut commands: Commands,
) {
    use core::f32::consts::FRAC_PI_2;
    for (entity, avatar, mut tf) in &mut avatars {
        match bots.get(avatar.src) {
            Ok(bot) => {
                tf.translation = bot.pos;
                // Yaw to face, then pitch forward about the feet as it dies.
                tf.rotation = Quat::from_rotation_y(bot.yaw)
                    * Quat::from_rotation_x(bot.fall.clamp(0.0, 1.0) * FRAC_PI_2);
            }
            Err(_) => {
                commands.entity(entity).try_despawn();
            }
        }
    }
}

fn on_disconnected(t: Trigger<OnAdd, Disconnected>, q: Query<&Disconnected>) {
    // `NetcodeClient` requires `Disconnected`, so this also fires once at spawn
    // with no reason — that's just "not connected yet", not a real drop.
    match q.get(t.target()).ok().and_then(|d| d.reason.clone()) {
        Some(reason) => warn!("disconnected from server: {reason}"),
        None => debug!("client not connected yet"),
    }
}
