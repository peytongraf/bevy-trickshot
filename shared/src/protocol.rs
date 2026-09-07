//! The lightyear protocol: what gets replicated, what the client sends every
//! tick, and what the server sends back.
//!
//! Authority split:
//! * **Movement is client-authoritative.** The owning client fills in
//!   `translation` / `yaw` / `pitch` on its [`PlayerInput`] each tick; the server
//!   copies those straight onto the replicated [`PlayerPose`] with no correction.
//! * **Shots are server-authoritative.** The `fire_*` fields of [`PlayerInput`]
//!   are only a *request*; the server ray-casts them and answers with a
//!   [`ShotResolved`] message.

use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::math::Curve;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::weapon::WeaponId;

/// Which connected peer owns a player entity. Replicated once, never changes.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerId(pub PeerId);

/// A player's pose in the world. Written by the server from the owner's
/// client-authoritative input, replicated to everyone, shown interpolated on
/// non-owning clients.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerPose {
    pub translation: Vec3,
    /// Yaw around +Y, radians.
    pub yaw: f32,
    /// Look pitch, radians.
    pub pitch: f32,
}

impl Default for PlayerPose {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

impl Ease for PlayerPose {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| PlayerPose {
            translation: Vec3::lerp(start.translation, end.translation, t),
            yaw: lerp_angle(start.yaw, end.yaw, t),
            pitch: start.pitch + (end.pitch - start.pitch) * t,
        })
    }
}

/// Shortest-arc angle lerp, so a yaw that wraps past ±π interpolates the short
/// way instead of spinning all the way around.
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    use core::f32::consts::{PI, TAU};
    let mut diff = (b - a) % TAU;
    if diff > PI {
        diff -= TAU;
    } else if diff < -PI {
        diff += TAU;
    }
    a + diff * t
}

/// The full input packet every client sends each tick.
///
/// Vectors are plain `[f32; 3]` so the wire format never depends on a specific
/// `glam`/`bevy_math` version — handy when the client and server drift apart.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Reflect)]
pub struct PlayerInput {
    /// Owner-authoritative position this tick.
    pub translation: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// Whether the trigger was pulled this tick.
    pub fire: bool,
    /// Muzzle position of the shot the client is requesting.
    pub fire_origin: [f32; 3],
    /// Aim direction of that shot (server normalises it).
    pub fire_dir: [f32; 3],
    /// Selected weapon, as [`WeaponId::as_u8`].
    pub weapon: u8,
}

impl Default for PlayerInput {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            fire: false,
            fire_origin: [0.0; 3],
            fire_dir: [0.0, 0.0, -1.0],
            weapon: WeaponId::Sniper.as_u8(),
        }
    }
}

impl MapEntities for PlayerInput {
    fn map_entities<M: EntityMapper>(&mut self, _mapper: &mut M) {}
}

/// What the server decided a fire request did.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ShotOutcome {
    Miss,
    Hit {
        /// `PeerId::to_bits()` of the player that was hit.
        target: u64,
        headshot: bool,
        /// World-space impact point.
        point: [f32; 3],
        damage: f32,
    },
}

/// Server → client: the authoritative result of a validated shot.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ShotResolved {
    pub shooter: PeerId,
    /// Server tick the shot resolved on (wrapping u16).
    pub tick: u16,
    pub outcome: ShotOutcome,
}

/// Reliable, unordered server → client channel for gameplay events.
pub struct GameChannel;

// ---------------------------------------------------------------------------
// Lobbies
// ---------------------------------------------------------------------------

/// One member of a [`Lobby`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LobbyMember {
    pub peer: PeerId,
    pub name: String,
}

/// A lobby, spawned on the server and replicated to **every** client so the
/// browser and the lobby room update live. `leader` is the party leader — the
/// creator, or a promoted member if the creator left.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lobby {
    pub name: String,
    pub leader: PeerId,
    /// Once `true` the members are being moved into a game; the lobby stops
    /// showing in the browser.
    pub started: bool,
    pub members: Vec<LobbyMember>,
}

impl Lobby {
    pub fn has(&self, peer: PeerId) -> bool {
        self.members.iter().any(|m| m.peer == peer)
    }
}

/// A player's display name, replicated onto their in-world player entity so
/// other clients can label the capsule.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerName(pub String);

/// Client → server: create a new lobby and join it as leader.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct CreateLobby {
    pub name: String,
    pub player_name: String,
}

/// Client → server: join an existing lobby. `lobby` is the entity as the client
/// knows it; lightyear maps it to the server's entity on arrival
/// (`add_map_entities`).
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct JoinLobby {
    pub lobby: Entity,
    pub player_name: String,
}

impl MapEntities for JoinLobby {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.lobby = mapper.get_mapped(self.lobby);
    }
}

/// Client → server: leave whatever lobby the sender is in (server derives it).
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct LeaveLobby;

/// Client → server: the party leader starts the game for their lobby.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct StartGame;

/// Server → client: a lobby request could not be honoured.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LobbyError {
    pub reason: String,
}

/// Reliable channel for lobby traffic, both directions.
pub struct LobbyChannel;

/// Registers everything above. Added by [`crate::SharedPlugin`] on both ends.
#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PlayerInput>();

        // messages
        app.add_message::<ShotResolved>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<LobbyError>()
            .add_direction(NetworkDirection::ServerToClient);

        // lobby actions (client -> server, as triggers so the server sees `from`)
        app.add_trigger::<CreateLobby>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<JoinLobby>()
            .add_map_entities()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<LeaveLobby>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<StartGame>()
            .add_direction(NetworkDirection::ClientToServer);

        // inputs (client -> server)
        app.add_plugins(input::native::InputPlugin::<PlayerInput>::default());

        // replicated components
        app.register_component::<PlayerId>()
            .add_prediction(PredictionMode::Once)
            .add_interpolation(InterpolationMode::Once);

        app.register_component::<PlayerPose>()
            .add_prediction(PredictionMode::Full)
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();

        app.register_component::<PlayerName>()
            .add_interpolation(InterpolationMode::Once);

        app.register_component::<Lobby>();

        // channels
        app.add_channel::<GameChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);

        app.add_channel::<LobbyChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::ClientToServer)
        .add_direction(NetworkDirection::ServerToClient);
    }
}
