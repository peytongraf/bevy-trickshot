//! Code compiled into **both** the client and the server.
//!
//! * [`protocol`] — the lightyear wire protocol (replicated components, the
//!   per-tick input packet, gameplay messages, channels).
//! * [`ballistics`] / [`hitbox`] / [`weapon`] — the deterministic shot-resolution
//!   math the server runs authoritatively and the client can reuse to predict.
//! * [`map`] — the small trait surface the server needs from your (future) map
//!   geometry and navmesh.
//!
//! Keeping all of this in one crate is the whole point of pinning the server to
//! the same Bevy version as the client: there is exactly one definition of the
//! protocol and one definition of "did this shot hit".

pub mod ballistics;
pub mod bots;
pub mod hitbox;
pub mod map;
pub mod protocol;
pub mod scoring;
pub mod weapon;

use bevy::prelude::*;

pub use protocol::{
    Bot, CreateLobby, EndGame, GameChannel, GameMode, JoinLobby, KillCam, KillCamBot, KillCamSample,
    LeaveLobby, Lobby, LobbyChannel, LobbyError, LobbyMember, MatchOver, PlayerId, PlayerInput,
    PlayerName, PlayerPose, ProtocolPlugin, RemoteSound, ScoreLine, SetTimeLimit, ShotOutcome,
    ShotResolved, StartGame, TrickScore,
};

/// Simulation tick rate (Hz). The client and server must agree on this.
pub const TICK_HZ: f64 = 64.0;

/// How often the server flushes replication updates to each client (ms).
pub const REPLICATION_INTERVAL_MS: u64 = 50;

/// Netcode protocol id. Bump this on any breaking change to [`protocol`] so
/// mismatched client/server builds refuse to connect instead of desyncing.
pub const PROTOCOL_ID: u64 = 0x7213_c150_0000_0005;

/// Port the server listens on unless `PORT` says otherwise.
pub const DEFAULT_PORT: u16 = 5000;

/// All-zero netcode key for local development. Production supplies a real key via
/// the `LIGHTYEAR_PRIVATE_KEY` environment variable (see `server`'s `net` module).
pub const DEV_PRIVATE_KEY: [u8; 32] = [0; 32];

/// Added by every peer. Registers the shared protocol so replication, prediction
/// and interpolation line up on both ends.
#[derive(Clone)]
pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(protocol::ProtocolPlugin);
    }
}
