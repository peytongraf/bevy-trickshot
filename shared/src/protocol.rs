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

/// Fallback scope magnification for a fresh `PlayerInput`, before the client
/// fills in its real `Settings::scope_zoom`. Mirrors the client's default
/// `ScopeZoom`.
const DEFAULT_SCOPE_ZOOM: f32 = 11.0;

/// The game mode a lobby plays. New modes slot in here; both ends branch on the
/// one the [`Lobby`] carries.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    /// **Freestyle** — free-for-all against bots: rack up the most style
    /// points (kills, spins, no-scopes) before the clock runs out. Highest
    /// score wins.
    #[default]
    Freestyle,
    /// **Free For All** — real PvP, Call-of-Duty style: first player to the
    /// lobby's kill limit wins, or whoever has the most kills when the clock
    /// runs out. No bots.
    FreeForAll,
}

impl GameMode {
    pub fn label(self) -> &'static str {
        match self {
            GameMode::Freestyle => "FREESTYLE",
            GameMode::FreeForAll => "FREE FOR ALL",
        }
    }
}

/// Which map a lobby plays on. New maps slot in here; the client branches on
/// the one the [`Lobby`] carries to decide which scene to load (see
/// `client::CurrentMap`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MapId {
    /// `models/basic_map.glb` — cubes, ramps and a bridge.
    #[default]
    BasicMap,
    /// `models/shipment.glb` — a Call-of-Duty-style Shipment recreation:
    /// ground plane plus shipping-container walls.
    Shipment,
    /// The same `models/shipment.glb` geometry as [`MapId::Shipment`], set in
    /// bright, clear daytime instead of dark, foggy, rainy dusk — only the
    /// client's fog/sky/lighting/weather differ; collision, spawns and bounds
    /// are identical (see [`MapId::is_shipment`]).
    ShipmentDay,
}

impl MapId {
    pub fn label(self) -> &'static str {
        match self {
            MapId::BasicMap => "BASIC MAP",
            MapId::Shipment => "SHIPMENT",
            MapId::ShipmentDay => "SHIPMENT DAY",
        }
    }

    /// `true` for either variant built on `shipment.glb` (night or day) —
    /// same model, collision, walls and spawn bounds, so anything keyed to
    /// the geometry rather than the time of day should test this instead of
    /// `== MapId::Shipment`.
    pub fn is_shipment(self) -> bool {
        matches!(self, MapId::Shipment | MapId::ShipmentDay)
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
    /// Aim-down-sight amount (`0.0` at the hip … `1.0` fully scoped), copied
    /// from the owner's `PlayerInput::ads_t` so remote avatars can play an
    /// aiming animation.
    pub ads_t: f32,
    /// Whether the owner's stance is `Crouching`, copied from
    /// `PlayerInput::crouching` so remote avatars can play a crouch
    /// animation. Discrete, so `Ease` below just takes `end.crouching`
    /// rather than blending it, same as `Bot::alive`.
    pub crouching: bool,
    /// Whether the owner's weapon is mid-reload, copied from
    /// `PlayerInput::reloading` so remote avatars can play the reload
    /// animation once. Discrete, same as `crouching`.
    pub reloading: bool,
    /// Whether the owner is mid-jump (from launch until landing), copied from
    /// `PlayerInput::jumping` so remote avatars can play the jump animation.
    /// Discrete, same as `crouching`. Sustained across the whole jump arc
    /// rather than a single-tick pulse — a value that's only ever true for
    /// one tick can get silently collapsed away by interpolation catch-up on
    /// other clients before they ever see it.
    pub jumping: bool,
    /// Whether the owner's stance is `Sliding`, copied from
    /// `PlayerInput::sliding` so remote avatars can play the crouch
    /// animation during a slide (there's no dedicated slide clip). Discrete,
    /// same as `crouching`; kept separate from it (rather than folded in)
    /// since a slide should always show the static crouch pose, never
    /// `crouchWalk`, regardless of slide speed.
    pub sliding: bool,
    /// Whether the owner is alive, copied server-side from their
    /// `PlayerCombat::alive` (`server::sim::apply_client_pose`) — unlike
    /// every other field above, this one is **not** taken from the owner's
    /// own `PlayerInput`: a dead client isn't even sending fresh input
    /// (`client::net::write_input` stops while their kill cam plays), so it
    /// has to come from the server's own authoritative combat state
    /// instead. `true` for any player with no `PlayerCombat` at all
    /// (`Freestyle` mode never adds one — see that component's doc comment
    /// — so those players are always "alive"). Lets every client, not just
    /// the victim, see a remote avatar play its death animation and freeze
    /// on the last frame until this flips back to `true` on respawn.
    /// Discrete, same as `crouching`.
    pub alive: bool,
}

impl Default for PlayerPose {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            ads_t: 0.0,
            crouching: false,
            reloading: false,
            jumping: false,
            sliding: false,
            alive: true,
        }
    }
}

impl Ease for PlayerPose {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| PlayerPose {
            translation: Vec3::lerp(start.translation, end.translation, t),
            yaw: lerp_angle(start.yaw, end.yaw, t),
            pitch: start.pitch + (end.pitch - start.pitch) * t,
            ads_t: start.ads_t + (end.ads_t - start.ads_t) * t,
            crouching: end.crouching,
            reloading: end.reloading,
            jumping: end.jumping,
            sliding: end.sliding,
            alive: end.alive,
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
    /// Whether the player stabbed with the knife this tick. Independent of
    /// `fire` (the sniper's trigger); reuses `fire_origin` / `fire_dir` for
    /// the stab's eye position and aim direction, and the server resolves it
    /// with [`crate::melee::resolve_melee`].
    pub melee: bool,
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
    /// Magnification of the scope the player has equipped (`3.0`, `8.0`, `11.0`
    /// — `Settings::scope_zoom`), so the kill cam's ADS zoom matches the
    /// shooter's scope.
    pub scope_zoom: f32,
    /// One-shot sounds the player triggered this tick, as a bitmask —
    /// replayed in the kill cam, and also broadcast live to the rest of the
    /// lobby (see `RemoteSound`) so they hear this player's actions
    /// positionally. Bit meanings are client-internal (`killcam::SND_*`).
    /// `u16` rather than `u8` since all 8 low bits are already spoken for.
    pub sound_bits: u16,
    /// Playhead (seconds) of the first-person weapon's baked animation clip this
    /// tick, so the kill cam can pose the gun exactly as the player saw it.
    pub anim_time: f32,
    /// Playhead (seconds) of the first-person *knife's* baked animation clip
    /// this tick — the knife's counterpart of `anim_time`, so the kill cam
    /// can replay its slices too.
    pub knife_anim_time: f32,
    /// Aim-down-sight amount this tick (`0.0` at the hip … `1.0` fully scoped),
    /// recorded so the kill cam replays the exact scope-in / scope-out timing.
    pub ads_t: f32,
    /// World-space impact point of a shot that struck the ground this tick, so
    /// the kill cam can re-emit the dust / rock burst. `None` otherwise.
    pub ground_pt: Option<[f32; 3]>,
    /// World-space point a bot was hit this tick, so the kill cam can re-emit
    /// the blood squirt. `None` otherwise.
    pub blood_pt: Option<[f32; 3]>,
    /// `[start, end]` world points of a shot's tracer fired this tick, so the
    /// kill cam re-draws it along its true path at the replayed shot moment
    /// rather than leaving the live one hanging in the world. `None` otherwise.
    pub tracer: Option<[[f32; 3]; 2]>,
    /// Whether the first-person weapon model is shown this tick (`false` while
    /// holstered for the secondary slot, or snapped away for a throwing-knife
    /// hold), so the kill cam can reproduce weapon swaps and knife-hides
    /// instead of freezing whatever was on screen when the replay started.
    pub weapon_visible: bool,
    /// Whether the throwing-knife key is held this tick, so the kill cam can
    /// show its crosshair over the recorded window.
    pub knife_active: bool,
    /// Whether the sniper (rather than the empty-handed secondary slot) is
    /// the active weapon this tick, so the kill cam shows the centre dot only
    /// over the frames where the shooter actually had it.
    pub sniper_active: bool,
    /// Whether the first-person melee knife model is shown this tick. Recorded
    /// on its own — it can no longer be derived from `weapon_visible`, since
    /// with the throwing arms up *both* weapon models are hidden.
    pub knife_visible: bool,
    /// How far the throwing arms have slid into view this tick (`0.0` hidden
    /// below the screen … `1.0` in place) — see the client's
    /// `ThrowingKnife::slide`.
    pub arms_slide: f32,
    /// Playhead (seconds) of the throwing arms' throw clip this tick.
    pub arms_anim_time: f32,
    /// Whether the knife is showing in the throwing arms' hand this tick (it
    /// vanishes when the throw clip starts).
    pub arms_knife_in_hand: bool,
    /// Whether the player's stance is `Crouching` this tick.
    pub crouching: bool,
    /// Whether the weapon is mid-reload this tick.
    pub reloading: bool,
    /// Whether the player is mid-jump (from launch until landing) this tick.
    pub jumping: bool,
    /// Whether the player's stance is `Sliding` this tick.
    pub sliding: bool,
    /// Camera Y offset from standing this tick (metres, `<= 0.0` — see the
    /// client's `Slide::drop`), so the kill cam can reproduce crouch / slide /
    /// prone height exactly instead of always replaying at standing height.
    pub crouch_drop: f32,
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
            melee: false,
            weapon: WeaponId::Sniper.as_u8(),
            spin_deg: 0.0,
            airborne: false,
            noscope: false,
            shake_trauma: 0.0,
            shake_phase: 0.0,
            shake_recoil: 0.0,
            sway_offset: [0.0; 2],
            fov_deg: DEFAULT_FOV_DEG,
            scope_zoom: DEFAULT_SCOPE_ZOOM,
            sound_bits: 0,
            anim_time: 0.0,
            knife_anim_time: 0.0,
            ads_t: 0.0,
            ground_pt: None,
            blood_pt: None,
            tracer: None,
            weapon_visible: true,
            knife_active: false,
            sniper_active: true,
            knife_visible: false,
            arms_slide: 0.0,
            arms_anim_time: 0.0,
            arms_knife_in_hand: false,
            crouching: false,
            reloading: false,
            jumping: false,
            sliding: false,
            crouch_drop: 0.0,
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
    /// The shot hit nothing but the world — the ground, or the first wall /
    /// crate / container it ran into (bullets stop at the first surface; there
    /// are no wallbangs). Every client spawns a rock/dust burst at `point`
    /// (see the client's `spawn_ground_impact`).
    Ground {
        /// World-space impact point on the surface.
        point: [f32; 3],
        /// Unit surface normal at the impact, facing the shooter — where the
        /// clients stick the bullet-hole decal, flat on the surface.
        normal: [f32; 3],
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

/// Server → everyone else in the lobby: a player triggered one or more
/// one-shot sounds (`killcam::SND_*`) this tick, so it can be played back
/// positionally at `position` on every other client. Never sent to the
/// player who triggered it — they already hear their own local, non-spatial
/// version of these sounds.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct RemoteSound {
    pub player: PeerId,
    pub bits: u16,
    /// World point the sound should play from (the player's pose position).
    pub position: [f32; 3],
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
    /// True if a best-play [`KillCam`] was also sent this tick. `GameChannel`
    /// is unordered, so the client can't infer this from arrival order —
    /// without an explicit flag it could show the results screen before (or
    /// instead of) a best-play replay that's still in flight.
    pub best_play_sent: bool,
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
    /// The shooter's scope magnification, so the replay zooms exactly as theirs did.
    pub scope_zoom: f32,
    /// One-shot sounds triggered on this frame (`killcam::SND_*`).
    pub sound_bits: u16,
    /// First-person weapon animation playhead (seconds) on this frame.
    pub anim_time: f32,
    /// First-person knife animation playhead (seconds) on this frame.
    pub knife_anim_time: f32,
    /// Aim-down-sight amount on this frame (`0.0` hip … `1.0` fully scoped).
    pub ads_t: f32,
    /// World point of a ground burst that fired on this frame, if any.
    pub ground_pt: Option<[f32; 3]>,
    /// World point of a blood squirt (bot hit) on this frame, if any.
    pub blood_pt: Option<[f32; 3]>,
    /// `[start, end]` world points of a shot tracer fired on this frame, if any.
    pub tracer: Option<[[f32; 3]; 2]>,
    /// Whether the first-person weapon model was shown on this frame.
    pub weapon_visible: bool,
    /// Whether the throwing-knife key was held on this frame.
    pub knife_active: bool,
    /// Whether the sniper was the active weapon on this frame.
    pub sniper_active: bool,
    /// Whether the first-person melee knife model was shown on this frame.
    pub knife_visible: bool,
    /// Throwing-arms slide amount on this frame (`0.0` hidden … `1.0` in place).
    pub arms_slide: f32,
    /// Throwing-arms throw-clip playhead (seconds) on this frame.
    pub arms_anim_time: f32,
    /// Whether the knife was showing in the throwing arms' hand on this frame.
    pub arms_knife_in_hand: bool,
    /// The killer's own thrown knives in flight (or lying still) on this
    /// frame, stamped server-side (`server::killcam::record_frames`) since the
    /// server owns their simulation. At most
    /// [`crate::throwing_knife::MAX_KNIVES_PER_PLAYER`], in a stable order.
    pub thrown_knives: [Option<KnifeSample>; crate::throwing_knife::MAX_KNIVES_PER_PLAYER],
    /// Camera Y offset from standing this frame — see
    /// `PlayerInput::crouch_drop`.
    pub crouch_drop: f32,
}

/// One thrown knife's pose on a kill-cam frame — the replicated
/// [`ThrownKnife`] state, in wire-friendly arrays.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KnifeSample {
    pub pos: [f32; 3],
    /// Orientation quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
}

/// A target bot as it stood the moment the kill landed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KillCamBot {
    pub pos: [f32; 3],
    pub yaw: f32,
    /// This is the bot that was shot.
    pub killed: bool,
}

/// Another player (not the killer) as they stood the moment the kill landed —
/// frozen for the replay the same way bots are, since there's no per-tick
/// recording of every *other* player's pose to draw a full timeline from.
/// Never includes the killer themselves (the replay is a first-person fly-
/// through of the killer's own recorded view, so they'd have no body to show).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KillCamPlayer {
    pub pos: [f32; 3],
    pub yaw: f32,
    /// This is the player that was shot (a `FreeForAll` kill's victim) —
    /// the replay plays their death animation once the playhead reaches the
    /// kill, the same way it does for a shot bot ([`KillCamBot::killed`]).
    pub killed: bool,
}

/// Server → everyone in a lobby: replay the
/// killer's last ~3 s. `samples` are oldest-first at `TICK_HZ`; `kill_index` is
/// the frame the shot landed on (2 s in, 1 s of follow-through after). `bots`
/// are the targets frozen at the kill moment so the replay can show the one
/// that was hit toppling over. `players` are every other lobby member frozen
/// the same way, so the replay doesn't leave their live remote avatars
/// wandering through what's meant to be a snapshot of the past.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KillCam {
    pub killer_name: String,
    pub samples: Vec<KillCamSample>,
    pub kill_index: u32,
    pub bots: Vec<KillCamBot>,
    pub players: Vec<KillCamPlayer>,
    /// True only for the single highest-scoring shot of the match, resent
    /// right before [`MatchOver`] once the clock hits zero. The client plays
    /// this one with a slow-mo ramp around the kill and a "BEST PLAY" banner
    /// instead of "KILLCAM", and holds the match-results screen off until it
    /// finishes.
    pub best_play: bool,
}

/// Client (party leader) → server: set the match length before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetTimeLimit {
    pub secs: u32,
}

/// Client (party leader) → server: pick the lobby's game mode before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetGameMode {
    pub mode: GameMode,
}

/// Client (party leader) → server: pick the lobby's map before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetMap {
    pub map: MapId,
}

/// Client (party leader) → server: set [`GameMode::FreeForAll`]'s kill limit
/// before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetKillLimit {
    pub kills: u32,
}

/// Server → client: only sent to the player who needs to respawn, once
/// they're allowed to — a spawn point their client should teleport its
/// player rig to. Sent for a [`GameMode::FreeForAll`] PvP kill, or (either
/// game mode) [`FellToDeath`]. See `client::net::flush_pending_respawn`,
/// which waits for a paired kill cam (see [`KillCam`]) — sent only for a
/// kill, never a fall — to finish playing before applying it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerRespawn {
    pub pos: [f32; 3],
    pub yaw: f32,
}

/// Server → client: only sent to the victim of a [`GameMode::FreeForAll`]
/// kill, the instant the fatal hit lands — well before [`PlayerRespawn`] or
/// the (deliberately delayed, see [`KillCam`]) kill cam. Lets the victim's
/// client immediately snap its view toward the killer for the brief
/// death-effect window (see `client::death_effect`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerKilledBy {
    pub killer_pos: [f32; 3],
}

/// Client → server: the local player (client-authoritative movement — same
/// trust model as [`PlayerInput`]) fell to their death, either dropping below
/// the map's fall-safety floor or landing after a lethal fall. Works in
/// either game mode: the server marks a `FreeForAll` player dead (see
/// `PlayerCombat`) or, in `Freestyle` (no health concept — always "alive"),
/// just repositions them; either way it answers with [`PlayerRespawn`] like a
/// PvP kill would, but never writes `PlayerKilled` — there's no killer, so no
/// kill cam plays. See `client::fall_death`.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct FellToDeath;

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
    /// `Freestyle`: style points from bots this member has shot. `FreeForAll`:
    /// this member's kill count. Whichever the lobby's `mode` is.
    pub score: u32,
    /// Whether this member's client has finished loading the match's assets
    /// (the map, currently — see [`AssetsReady`]) since the lobby last
    /// started. Reset to `false` on `StartGame`; irrelevant, and left
    /// whatever it was, while `Lobby::started` is `false`.
    pub loaded: bool,
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
    /// The map this lobby will play on.
    pub map: MapId,
    /// Once `true` the members are being moved into a game; the lobby stops
    /// showing in the browser.
    pub started: bool,
    /// Match length the leader picked (seconds). UI clamps to 60..=2700.
    pub time_limit_secs: u32,
    /// Seconds left in the running match; the server counts it down.
    pub time_left_secs: u32,
    /// [`GameMode::FreeForAll`]'s win condition: first member to this many
    /// kills (`LobbyMember::score`) ends the match. Unused by `Freestyle`.
    pub kill_limit: u32,
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

/// A server-owned target bot. Replicated to the members of one lobby's game so
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

/// A throwing knife in flight (or lying where it stopped), replicated to
/// every member of the lobby whose game it belongs to. The server owns the
/// whole simulation — flight, bounces off the map's collision mesh, hits — and
/// just publishes the result here (see `server::knives` and
/// [`crate::throwing_knife`]); clients only draw it, interpolated. Rotation is
/// simulated server-side too (the knife tumbles end over end, then settles
/// flat when it stops), so every player sees the same spin.
///
/// The rotation frame: `-Z` is the blade tip, `Y` the flat face's normal.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrownKnife {
    /// Who threw it.
    pub owner: PeerId,
    pub pos: Vec3,
    pub rot: Quat,
    /// `true` once it has stopped moving (it's removed a moment later).
    pub resting: bool,
}

impl Ease for ThrownKnife {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| ThrownKnife {
            owner: end.owner,
            pos: Vec3::lerp(start.pos, end.pos, t),
            rot: Quat::slerp(start.rot, end.rot, t),
            resting: end.resting,
        })
    }
}

/// Client → server: the player's throw animation reached the point where the
/// knife leaves their hand. `origin` is their eye position and `dir` the
/// aim direction at that moment; the server checks them against the player's
/// real pose and starts the flight ([`ThrownKnife`]) if the throw is allowed.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct ThrowKnife {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
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

/// Client → server: the sender has finished loading this match's assets (see
/// [`LobbyMember::loaded`]) — sent once, right after `Lobby::started` flips
/// true and the client's own map load finishes. Ignored if the sender isn't
/// in a started lobby.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct AssetsReady;

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
        app.add_message::<RemoteSound>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<PlayerRespawn>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<PlayerKilledBy>()
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
        app.add_trigger::<AssetsReady>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<EndGame>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetTimeLimit>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetGameMode>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetMap>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetKillLimit>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<FellToDeath>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ThrowKnife>()
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

        app.register_component::<ThrownKnife>()
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
