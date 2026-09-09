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

/// Fallback hip FOV (degrees) for a fresh `PlayerInput` before the client's
/// first tick fills in its real `Settings::fov`. Mirrors the client's own
/// `FOV_DEFAULT`.
const DEFAULT_FOV_DEG: f32 = 90.0;

/// The game mode a lobby plays. New modes slot in here; both ends branch on the
/// one the [`Lobby`] carries.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    /// **Freestyle** — free-for-all: rack up the most style points (kills,
    /// spins, no-scopes) before the clock runs out. Highest score wins.
    #[default]
    Freestyle,
}

impl GameMode {
    pub fn label(self) -> &'static str {
        match self {
            GameMode::Freestyle => "FREESTYLE",
        }
    }
}

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
    /// Degrees the shooter had spun (one continuous trick) at fire time — the
    /// server turns this into style points on a confirmed bot kill.
    pub spin_deg: f32,
    /// Whether any of that spin happened airborne.
    pub airborne: bool,
    /// Whether the shooter was fully un-scoped at fire time.
    pub noscope: bool,
    /// Camera-shake state this tick (`Shake::trauma` / `phase` / `recoil` on the
    /// client) — small and fully reproduces the positional jitter, view punch
    /// and forward recoil kick when replayed through the same formula the live
    /// game uses. Recorded for the kill-cam replay; `translation` / `yaw` /
    /// `pitch` above already carry the base pose, so this only needs to add the
    /// shake/recoil *delta* rather than a redundant absolute camera transform.
    pub shake_trauma: f32,
    pub shake_phase: f32,
    pub shake_recoil: f32,
    /// Weapon-sway offset this tick (`WeaponSwayState::offset`: yaw, pitch) —
    /// replayed the same way, so the gun's turn-lag is exact rather than just
    /// easing back to neutral during playback.
    pub sway_offset: [f32; 2],
    /// The player's hip field-of-view setting this tick, so the kill cam renders
    /// at the FOV the shooter actually had, not the viewer's own.
    pub fov_deg: f32,
    /// One-shot sounds the player triggered this tick, as a bitmask — replayed
    /// in the kill cam. Bit meanings are client-internal (`killcam::SND_*`).
    pub sound_bits: u8,
    /// Playhead (seconds) of the first-person weapon's baked animation clip this
    /// tick, so the kill cam can pose the gun exactly as the player saw it.
    pub anim_time: f32,
    /// Aim-down-sight amount this tick (`0.0` at the hip … `1.0` fully scoped),
    /// recorded so the kill cam replays the exact scope-in / scope-out timing.
    pub ads_t: f32,
    /// World-space impact point of a shot that struck the ground this tick, so
    /// the kill cam can re-emit the dust / rock burst. `None` otherwise.
    pub ground_pt: Option<[f32; 3]>,
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
            spin_deg: 0.0,
            airborne: false,
            noscope: false,
            shake_trauma: 0.0,
            shake_phase: 0.0,
            shake_recoil: 0.0,
            sway_offset: [0.0; 2],
            fov_deg: DEFAULT_FOV_DEG,
            sound_bits: 0,
            anim_time: 0.0,
            ads_t: 0.0,
            ground_pt: None,
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
    /// The shot hit nothing but the ground. Every client spawns a rock/dust
    /// burst at `point` (see the client's `spawn_ground_impact`).
    Ground {
        /// World-space impact point on the ground plane.
        point: [f32; 3],
    },
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
    /// Muzzle/eye world point the shot was fired from — tracer start.
    pub origin: [f32; 3],
    /// World point the shot's visual tracer should end at: the true impact
    /// point (even for a bot kill, which `outcome` reports as `Miss` since
    /// bots aren't a valid `ShotOutcome::Hit` target), or a point at the
    /// weapon's max range for a clean miss.
    pub tracer_end: [f32; 3],
}

/// One line of a scored shot's breakdown, e.g. `+50  360° SPIN`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScoreLine {
    pub label: String,
    pub points: u32,
}

/// Server → everyone: a shot scored style points. The shooter's client pops the
/// yellow stack; other clients can build a kill feed from it later.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrickScore {
    pub shooter: PeerId,
    pub total: u32,
    pub lines: Vec<ScoreLine>,
}

/// Server → everyone in a lobby: the match clock hit zero.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MatchOver {
    pub winner_name: String,
    pub winner_score: u32,
}

/// One recorded frame of a kill-cam replay (server tick rate). Self-contained:
/// the base pose (`translation` / `yaw` / `pitch`) plus the shake / recoil /
/// sway state needed to reconstruct the exact camera + weapon transforms the
/// shooter saw, by running the same pose formulas the live game uses.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KillCamSample {
    /// Player-root world position this frame.
    pub translation: [f32; 3],
    /// Player-root (body) yaw.
    pub yaw: f32,
    /// Head pitch.
    pub pitch: f32,
    /// Camera-shake state (see `PlayerInput::shake_trauma` and friends) —
    /// reproduces the positional jitter, view punch and recoil kick exactly.
    pub shake_trauma: f32,
    pub shake_phase: f32,
    pub shake_recoil: f32,
    /// Weapon-sway offset (yaw, pitch) this frame.
    pub sway_offset: [f32; 2],
    /// The shooter's hip FOV setting, so the replay renders at their FOV.
    pub fov_deg: f32,
    /// One-shot sounds triggered on this frame (`killcam::SND_*`).
    pub sound_bits: u8,
    /// First-person weapon animation playhead (seconds) on this frame.
    pub anim_time: f32,
    /// Aim-down-sight amount on this frame (`0.0` hip … `1.0` fully scoped).
    pub ads_t: f32,
    /// World point of a ground burst that fired on this frame, if any.
    pub ground_pt: Option<[f32; 3]>,
}

/// A target bot as it stood the moment the kill landed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KillCamBot {
    pub pos: [f32; 3],
    pub yaw: f32,
    /// This is the bot that was shot.
    pub killed: bool,
}

/// Server → everyone in a lobby (and built locally in Practice): replay the
/// killer's last ~3 s. `samples` are oldest-first at `TICK_HZ`; `kill_index` is
/// the frame the shot landed on (2 s in, 1 s of follow-through after). `bots`
/// are the targets frozen at the kill moment so the replay can show the one
/// that was hit toppling over.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KillCam {
    pub killer_name: String,
    pub samples: Vec<KillCamSample>,
    pub kill_index: u32,
    pub bots: Vec<KillCamBot>,
}

/// Client (party leader) → server: set the match length before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetTimeLimit {
    pub secs: u32,
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
    /// Bots this member has shot during the current game.
    pub score: u32,
}

/// A lobby, spawned on the server and replicated to **every** client so the
/// browser and the lobby room update live. `leader` is the party leader — the
/// creator, or a promoted member if the creator left.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lobby {
    pub name: String,
    pub leader: PeerId,
    /// The mode this lobby will play.
    pub mode: GameMode,
    /// Once `true` the members are being moved into a game; the lobby stops
    /// showing in the browser.
    pub started: bool,
    /// Match length the leader picked (seconds). UI clamps to 60..=2700.
    pub time_limit_secs: u32,
    /// Seconds left in the running match; the server counts it down.
    pub time_left_secs: u32,
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

/// A server-owned practice bot. Replicated to the members of one lobby's game so
/// everyone sees the same bots in the same spots, and sees the same one tip over
/// when it's shot.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Bot {
    /// Feet position, on the ground.
    pub pos: Vec3,
    /// Facing (radians); also the direction it topples.
    pub yaw: f32,
    /// `true` until shot.
    pub alive: bool,
    /// `0.0` upright … `1.0` flat on the ground. Ramps up after death.
    pub fall: f32,
}

impl Ease for Bot {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| Bot {
            pos: Vec3::lerp(start.pos, end.pos, t),
            yaw: lerp_angle(start.yaw, end.yaw, t),
            alive: end.alive,
            fall: start.fall + (end.fall - start.fall) * t,
        })
    }
}

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

/// Client → server: the party leader ends the running game for the **whole**
/// party — every player entity is despawned and the lobby is disbanded, so all
/// members drop back to the main menu. (A leader leaving *without* the party, or
/// any non-leader leaving, sends [`LeaveLobby`] instead: that pulls just the one
/// player and, for the leader, promotes a replacement.)
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct EndGame;

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
        app.add_message::<TrickScore>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<MatchOver>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<KillCam>()
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
        app.add_trigger::<EndGame>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetTimeLimit>()
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

        app.register_component::<Bot>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();

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
