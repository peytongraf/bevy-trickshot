//! Code compiled into **both** the client and the server.
//!
//! * [`protocol`] — the lightyear wire protocol (replicated components, the
//!   per-tick input packet, gameplay messages, channels).
//! * [`ballistics`] / [`hitbox`] / [`weapon`] — the deterministic shot-resolution
//!   math the server runs authoritatively and the client can reuse to predict.
//! * [`map`] — where each map's model sits, plus the collision trait the
//!   server's map meshes implement.
//! * [`throwing_knife`] — the throwing knife's flight / bounce physics.
//!
//! Keeping all of this in one crate is the whole point of pinning the server to
//! the same Bevy version as the client: there is exactly one definition of the
//! protocol and one definition of "did this shot hit".

pub mod ballistics;
pub mod bots;
pub mod hitbox;
pub mod map;
pub mod melee;
pub mod protocol;
pub mod scoring;
pub mod spawns;
pub mod throwing_knife;
pub mod weapon;

use bevy::prelude::*;

pub use protocol::{
    AssetsReady, Bot, CreateLobby, EndGame, FellToDeath, GameChannel, GameMode, JoinLobby, KillCam,
    HitMarker, KillCamBot, KillCamPlayer, KillCamSample, KnifeAttackSound, KnifeSample, LeaveLobby,
    Lobby, LobbyChannel, LobbyError,
    LobbyMember, MapId, MatchOver, PlayerId, PlayerInput, PlayerKilledBy, PlayerName, PlayerPose,
    PlayerRespawn, ProtocolPlugin, RemoteSound, ScoreLine, SetGameMode, SetKillLimit, SetMap,
    SetTimeLimit, ShotOutcome, ShotResolved, StartGame, ThrowKnife, ThrownKnife, ThrowingKnifeHit,
    ThrowingKnifeImpact, TrickScore,
};

/// Simulation tick rate (Hz). The client and server must agree on this.
pub const TICK_HZ: f64 = 64.0;

/// How often the server flushes replication updates to each client (ms).
pub const REPLICATION_INTERVAL_MS: u64 = 50;

/// Netcode protocol id. Bump this on any breaking change to [`protocol`] so
/// mismatched client/server builds refuse to connect instead of desyncing.
pub const PROTOCOL_ID: u64 = 0x7213_c150_0000_000f;

/// Port the server listens on unless `PORT` says otherwise.
pub const DEFAULT_PORT: u16 = 5000;

/// All-zero netcode key for local development. Any self-hosted server (a
/// friend running `cargo run --bin server` without setting the env var below)
/// also falls back to this, so it doubles as the "not the official prod
/// server" key.
pub const DEV_PRIVATE_KEY: [u8; 32] = [0; 32];

/// Netcode key for `bevy-trickshot-server.fly.dev`, matching that deploy's
/// `LIGHTYEAR_PRIVATE_KEY` secret (see `server`'s `net` module) — rotate both
/// together, or every client still on the old key fails the netcode handshake
/// (`Crypto(Failed(Error))` server-side) even once its packets reach the
/// server. The client picks this over `DEV_PRIVATE_KEY` only when it's
/// actually targeting this address (see `client::net::server_addr`).
pub const PROD_PRIVATE_KEY: [u8; 32] = [
    15, 212, 122, 25, 119, 203, 128, 136, 123, 64, 154, 38, 234, 218, 121, 61, 202, 93, 167, 175,
    10, 10, 135, 238, 95, 188, 195, 182, 142, 14, 90, 174,
];

/// Added by every peer. Registers the shared protocol so replication, prediction
/// and interpolation line up on both ends.
#[derive(Clone)]
pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(protocol::ProtocolPlugin);
    }
}
