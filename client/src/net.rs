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

use shared::{PlayerId, PlayerInput, PlayerPose};

use crate::{AppState, Player, PlayerHead};

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

/// Copy this frame's local pose (client-authoritative) into the input packet.
fn write_input(
    player: Query<&Transform, With<Player>>,
    head: Query<&Transform, With<PlayerHead>>,
    mut q: Query<&mut ActionState<PlayerInput>, With<InputMarker<PlayerInput>>>,
) {
    let (Ok(pt), Ok(ht), Ok(mut action)) = (player.single(), head.single(), q.single_mut()) else {
        return;
    };
    action.translation = pt.translation.to_array();
    action.yaw = pt.rotation.to_euler(EulerRot::YXZ).0;
    action.pitch = ht.rotation.to_euler(EulerRot::YXZ).1;
    action.fire = false; // shot replication is out of scope for now
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

fn on_disconnected(t: Trigger<OnAdd, Disconnected>, q: Query<&Disconnected>) {
    // `NetcodeClient` requires `Disconnected`, so this also fires once at spawn
    // with no reason — that's just "not connected yet", not a real drop.
    match q.get(t.target()).ok().and_then(|d| d.reason.clone()) {
        Some(reason) => warn!("disconnected from server: {reason}"),
        None => debug!("client not connected yet"),
    }
}
