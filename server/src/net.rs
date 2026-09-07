//! Transport + connection lifecycle: bind the socket and accept clients. Player
//! entities are created by [`crate::lobby`] when a game starts, not here.

use bevy::prelude::*;
use core::net::{Ipv6Addr, SocketAddr};
use core::time::Duration;

use lightyear::connection::client::Connected;
use lightyear::netcode::{NetcodeServer, PRIVATE_KEY_BYTES};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{DEFAULT_PORT, DEV_PRIVATE_KEY, PROTOCOL_ID};

pub struct ServerNetPlugin;

impl Plugin for ServerNetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_server, start_listening).chain());
        app.add_observer(on_client_link);
        app.add_observer(on_client_connected);
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

fn spawn_server(mut commands: Commands) {
    // Bind `[::]` — dual-stack on Linux, so this serves the free dedicated IPv6
    // that fly.io routes UDP over, and still works if an IPv4 is added later.
    let addr = SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), listen_port());
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

