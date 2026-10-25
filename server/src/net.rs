//! Transport + connection lifecycle: bind the socket and accept clients. Player
//! entities are created by [`crate::lobby`] when a game starts, not here.
//!
//! Every client says its version on connecting ([`shared::ClientHello`]);
//! only one on exactly this server's ([`shared::GAME_VERSION`]) goes into
//! [`VerifiedPeers`], and only those can create or join a lobby
//! (`lobby::on_create` / `on_join`) — so an out-of-date client can't play
//! even if its own update check was skipped. One on another version is told
//! so ([`shared::VersionMismatch`]) and its client shows why.
//!
//! With `IDLE_SHUTDOWN_MINS` set (prod, `server/fly.toml`), the server exits
//! once nobody's been connected for that long ([`shut_down_when_idle`]), so
//! the fly machine stops and costs nothing until a client wakes it again
//! (`shared::wake`).

use bevy::prelude::*;
use core::net::{Ipv6Addr, SocketAddr};
use core::time::Duration;
use std::net::ToSocketAddrs;

use std::collections::HashSet;

use lightyear::connection::client::{Connected, Disconnected};
use lightyear::input::input_buffer::InputBuffer;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::input::InputSet;
use lightyear::netcode::{NetcodeServer, PRIVATE_KEY_BYTES};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{ClientHello, GameChannel, VersionMismatch, DEFAULT_PORT, DEV_PRIVATE_KEY, GAME_VERSION, PROTOCOL_ID, PlayerInput};

pub struct ServerNetPlugin;

impl Plugin for ServerNetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_server, start_listening).chain());
        app.add_systems(PreUpdate, reset_runaway_input_buffers.before(InputSet::ReceiveInputs));
        if let Some(limit) = idle_shutdown_limit() {
            info!("shutting down after {}s with nobody connected", limit.as_secs());
            app.insert_resource(IdleShutdown { limit, idle: Duration::ZERO });
            app.add_systems(Update, shut_down_when_idle);
        }
        app.init_resource::<VerifiedPeers>();
        app.add_observer(on_client_link);
        app.add_observer(on_client_connected);
        app.add_observer(on_client_hello);
        app.add_observer(on_client_gone);
    }
}

/// The connected clients whose [`ClientHello`] named this server's version —
/// the only ones `lobby` lets create or join a lobby.
#[derive(Resource, Default)]
pub(crate) struct VerifiedPeers(pub(crate) HashSet<PeerId>);

/// A client's version: on this server's, it's let in; on any other, it's
/// told ([`VersionMismatch`]) and kept out.
fn on_client_hello(
    trigger: Trigger<RemoteTrigger<ClientHello>>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut verified: ResMut<VerifiedPeers>,
) {
    let peer = trigger.from;
    let version = trigger.trigger.version.trim();
    if version == GAME_VERSION {
        verified.0.insert(peer);
        return;
    }
    verified.0.remove(&peer);
    warn!("client {peer:?} is on v{version}, not v{GAME_VERSION} — keeping it out of lobbies");
    let msg = VersionMismatch {
        server_version: GAME_VERSION.to_string(),
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send version mismatch: {e:?}");
    }
}

/// Forget a client's version once it's gone.
fn on_client_gone(
    trigger: Trigger<OnAdd, Disconnected>,
    clients: Query<&RemoteId, With<ClientOf>>,
    mut verified: ResMut<VerifiedPeers>,
) {
    if let Ok(peer) = clients.get(trigger.target()) {
        verified.0.remove(&peer.0);
    }
}

/// Most ticks of input a player's server-side buffer should ever hold —
/// normally it's only the few the client runs ahead by.
const MAX_INPUT_BUFFER_TICKS: usize = 256;

/// Throw away a player's input buffer that's run away (a client input tick
/// far off the server's), before lightyear adds to it: past 32 767 ticks
/// its `len() as i16 - 1` overflows and panics the whole server
/// (`lightyear_inputs::input_buffer::InputBuffer::set_raw`). The next input
/// message starts it afresh.
fn reset_runaway_input_buffers(
    server: Query<&LocalTimeline, With<Server>>,
    mut buffers: Query<(Entity, &mut InputBuffer<ActionState<PlayerInput>>)>,
) {
    for (entity, mut buffer) in &mut buffers {
        if buffer.buffer.len() <= MAX_INPUT_BUFFER_TICKS {
            continue;
        }
        warn!(
            "resetting runaway input buffer on {entity:?}: {} ticks from {:?} (server tick {:?})",
            buffer.buffer.len(),
            buffer.start_tick,
            server.iter().next().map(|t| t.tick()),
        );
        *buffer = InputBuffer::default();
    }
}

/// `IDLE_SHUTDOWN_MINS`: how long the server may sit with nobody connected
/// before exiting. Unset (or not a positive number) means never — a local
/// dev server stays up.
fn idle_shutdown_limit() -> Option<Duration> {
    let mins: f64 = std::env::var("IDLE_SHUTDOWN_MINS").ok()?.trim().parse().ok()?;
    (mins > 0.0).then(|| Duration::from_secs_f64(mins * 60.0))
}

#[derive(Resource)]
struct IdleShutdown {
    limit: Duration,
    /// How long nobody's been connected; reset whenever anyone is.
    idle: Duration,
}

/// Exit once nobody's been connected for [`IdleShutdown::limit`]. Exiting
/// cleanly (code 0) stops the fly machine for good — its restart policy only
/// restarts on a failure — until a client's TCP knock has fly start it again.
/// The clock starts at boot too, so a machine woken by a knock nobody
/// followed up on stops again.
fn shut_down_when_idle(
    time: Res<Time<Real>>,
    clients: Query<(), (With<ClientOf>, With<Connected>)>,
    mut idle: ResMut<IdleShutdown>,
    mut exit: EventWriter<AppExit>,
) {
    if !clients.is_empty() {
        idle.idle = Duration::ZERO;
        return;
    }
    idle.idle += time.delta();
    if idle.idle >= idle.limit {
        info!("nobody connected for {}s — shutting down", idle.limit.as_secs());
        exit.write(AppExit::Success);
    }
}

/// The port to listen on: `PORT` env var, else [`shared::DEFAULT_PORT`].
pub fn listen_port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Netcode key: from `LIGHTYEAR_PRIVATE_KEY` ("1,2,3,...", 32 bytes) in
/// production, otherwise the all-zero dev key.
fn private_key() -> [u8; PRIVATE_KEY_BYTES] {
    let Ok(raw) = std::env::var("LIGHTYEAR_PRIVATE_KEY") else {
        return DEV_PRIVATE_KEY;
    };
    let bytes: Vec<u8> = raw
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    match <[u8; PRIVATE_KEY_BYTES]>::try_from(bytes) {
        Ok(key) => {
            info!("using LIGHTYEAR_PRIVATE_KEY from environment");
            key
        }
        Err(_) => {
            error!("LIGHTYEAR_PRIVATE_KEY must be exactly {PRIVATE_KEY_BYTES} comma-separated bytes; falling back to dev key");
            DEV_PRIVATE_KEY
        }
    }
}

/// Where to bind the game UDP socket:
/// * On fly.io (`FLY_APP_NAME` is set for every deployed app) — the special
///   `fly-global-services` address. Fly's UDP routing only ever works over a
///   *dedicated IPv4* (never IPv6, despite this crate having previously bound
///   `[::]` and relied on the free dedicated IPv6 — see
///   <https://fly.io/docs/networking/udp-and-tcp/>: "You need a dedicated
///   IPv4 address. You can't use... an IPv6 address for UDP"), and a
///   wildcard bind (`0.0.0.0`/`[::]`) makes Linux pick the wrong outbound
///   source address for replies, which fly's eBPF NAT then can't rewrite
///   back to the public IP — every reply silently vanishes even though
///   inbound packets arrive fine. `fly-global-services` is the one address
///   that resolves (only inside fly's network) to something replies route
///   correctly from.
/// * Locally (`cargo run --bin server`) — the plain IPv6 wildcard, still
///   dual-stack for whoever's connecting from the same machine/LAN.
fn bind_addr() -> SocketAddr {
    let port = listen_port();
    if std::env::var_os("FLY_APP_NAME").is_some() {
        ("fly-global-services", port)
            .to_socket_addrs()
            .expect("fly-global-services must resolve on fly.io")
            .next()
            .expect("fly-global-services resolved to no addresses")
    } else {
        SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port)
    }
}

fn spawn_server(mut commands: Commands) {
    let addr = bind_addr();
    commands.spawn((
        Name::from("GameServer"),
        NetcodeServer::new(NetcodeConfig {
            protocol_id: PROTOCOL_ID,
            private_key: private_key(),
            ..default()
        }),
        LocalAddr(addr),
        ServerUdpIo::default(),
    ));
    info!("bevy-trickshot server binding udp://{addr}");
}

fn start_listening(mut commands: Commands, server: Single<Entity, With<Server>>) {
    commands.trigger_targets(Start, server.into_inner());
}

/// A fresh link: give it a replication sender so we can stream entities to it.
fn on_client_link(trigger: Trigger<OnAdd, LinkOf>, mut commands: Commands) {
    commands.entity(trigger.target()).insert((
        ReplicationSender::new(
            Duration::from_millis(shared::REPLICATION_INTERVAL_MS),
            SendUpdatesMode::SinceLastAck,
            false,
        ),
        Name::from("Client"),
    ));
}

/// The link is confirmed connected. Players are no longer spawned here — a
/// player entity is created only when its lobby's leader starts the game
/// (see [`crate::lobby`]).
fn on_client_connected(
    trigger: Trigger<OnAdd, Connected>,
    clients: Query<&RemoteId, With<ClientOf>>,
) {
    if let Ok(peer) = clients.get(trigger.target()) {
        info!("client {:?} connected", peer.0);
    }
}

