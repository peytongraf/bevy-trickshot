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

use crate::weapon::{SlotWeapon, WeaponId};

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
    /// **Zombies** — Call of Duty zombies style co-op: every lobby member
    /// against round after round of bot "zombies" that rise out of the ground
    /// around them, more (and better) each round. [`ZOMBIE_KILL_POINTS`] per
    /// kill; the game is over the moment any member dies.
    Zombies,
}

/// Points a lobby member scores for each zombie they kill ([`GameMode::Zombies`]).
pub const ZOMBIE_KILL_POINTS: u32 = 100;
/// Extra points on top of [`ZOMBIE_KILL_POINTS`] for a critical kill — a
/// headshot or a knife stab.
pub const ZOMBIE_CRITICAL_POINTS: u32 = 50;

/// Most damage one zombie hit does to a player — the sniper one-shots at 200,
/// which with "anyone dies, the game's over" would end a game on a single hit.
/// Three hits (with health regenerating in between) instead.
pub const ZOMBIE_HIT_DAMAGE: f32 = 34.0;

impl GameMode {
    /// Whether players pick their primary in a loadout
    /// ([`LobbyMember::loadout`]) — `Freestyle` is always the sniper.
    pub fn has_loadout(self) -> bool {
        matches!(self, GameMode::FreeForAll | GameMode::Zombies)
    }

    pub fn label(self) -> &'static str {
        match self {
            GameMode::Freestyle => "FREESTYLE",
            GameMode::FreeForAll => "FREE FOR ALL",
            GameMode::Zombies => "ZOMBIES",
        }
    }
}

/// Which map a lobby plays on. New maps slot in here; the client branches on
/// the one the [`Lobby`] carries to decide which scene to load (see
/// `client::CurrentMap`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MapId {
    /// `models/maps/basic_map.glb` — cubes, ramps and a bridge.
    #[default]
    BasicMap,
    /// `models/maps/shipment.glb` — a Call-of-Duty-style Shipment recreation:
    /// ground plane plus shipping-container walls.
    Shipment,
    /// The same `models/maps/shipment.glb` geometry as [`MapId::Shipment`], set in
    /// bright, clear daytime instead of dark, foggy, rainy dusk — only the
    /// client's fog/sky/lighting/weather differ; collision, spawns and bounds
    /// are identical (see [`MapId::is_shipment`]).
    ShipmentDay,
    /// `models/maps/break_point_map.glb` — a walled compound of low buildings and
    /// blocks, under a clear mid-day sky. Like [`MapId::BasicMap`] it has no separate
    /// visual model: the collision mesh is what's rendered.
    BreakPoint,
    /// The same `models/maps/break_point_map.glb` as [`MapId::BreakPoint`] at full
    /// night, with every player's gun flashlight on — only the client's
    /// fog/sky/lighting (and the flashlights) differ; collision, spawns,
    /// bounds and everything `Zombies` (perk machines, Pack-a-Punch, power
    /// switch) are identical (see [`MapId::is_break_point`]).
    BreakPointNight,
    /// `models/maps/ashes_of_the_damned_map.glb` — raised platforms and blocks
    /// over deep chasms, at night. Like [`MapId::BreakPoint`] the collision
    /// mesh is what's rendered, and it borrows [`MapId::BreakPointNight`]'s
    /// fog, sky and flashlights.
    AshesOfTheDamned,
}

impl MapId {
    /// Its full name, time of day included where it has one.
    pub fn label(self) -> &'static str {
        match self {
            MapId::BasicMap => "BASIC MAP",
            MapId::Shipment => "SHIPMENT NIGHT",
            MapId::ShipmentDay => "SHIPMENT DAY",
            MapId::BreakPoint => "BREAK POINT DAY",
            MapId::BreakPointNight => "BREAK POINT NIGHT",
            MapId::AshesOfTheDamned => "ASHES OF THE DAMNED",
        }
    }

    /// Just the place, without the time of day — what the lobby's map
    /// buttons say.
    pub fn place_label(self) -> &'static str {
        match self {
            MapId::BasicMap => "BASIC MAP",
            MapId::Shipment | MapId::ShipmentDay => "SHIPMENT",
            MapId::BreakPoint | MapId::BreakPointNight => "BREAK POINT",
            MapId::AshesOfTheDamned => "ASHES OF THE DAMNED",
        }
    }

    /// Every place a lobby can pick (one variant each — see
    /// [`Self::with_night`] for the time of day).
    pub const PLACES: [MapId; 4] = [MapId::BasicMap, MapId::Shipment, MapId::BreakPoint, MapId::AshesOfTheDamned];

    /// Whether the place comes in a day and a night version.
    pub fn has_time_of_day(self) -> bool {
        self.is_shipment() || self.is_break_point()
    }

    /// Whether this is the night version of a place with both.
    pub fn is_night(self) -> bool {
        matches!(self, MapId::Shipment | MapId::BreakPointNight)
    }

    /// The same place at night (`true`) or by day (`false`) — itself for a
    /// place with only the one.
    pub fn with_night(self, night: bool) -> MapId {
        match (self, night) {
            (MapId::Shipment | MapId::ShipmentDay, true) => MapId::Shipment,
            (MapId::Shipment | MapId::ShipmentDay, false) => MapId::ShipmentDay,
            (MapId::BreakPoint | MapId::BreakPointNight, true) => MapId::BreakPointNight,
            (MapId::BreakPoint | MapId::BreakPointNight, false) => MapId::BreakPoint,
            (other, _) => other,
        }
    }

    /// `true` for either variant built on `shipment.glb` (night or day) —
    /// same model, collision, walls and spawn bounds, so anything keyed to
    /// the geometry rather than the time of day should test this instead of
    /// `== MapId::Shipment`.
    pub fn is_shipment(self) -> bool {
        matches!(self, MapId::Shipment | MapId::ShipmentDay)
    }

    /// `true` for either variant built on `break_point_map.glb` (day or
    /// night) — the same idea as [`Self::is_shipment`].
    pub fn is_break_point(self) -> bool {
        matches!(self, MapId::BreakPoint | MapId::BreakPointNight)
    }
}

/// Which connected peer owns a player entity. Replicated once, never changes.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerId(pub PeerId);

/// A player's health, replicated from the server (`server::pvp` owns it — the
/// client never changes it, only displays it: the damage overlay and
/// heartbeat). `0` while dead; back to [`crate::health::FULL_HEALTH`] on
/// respawn.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerHealth(pub f32);

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
    /// Whether the owner is mid knife stab, copied from
    /// `PlayerInput::stabbing` so remote avatars can play the melee
    /// animation. Discrete, same as `crouching`; sustained for the whole
    /// stab rather than the one-tick `PlayerInput::melee` pulse, for the
    /// same reason as `jumping`.
    pub stabbing: bool,
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
    /// What a `Zombies` zombie is doing, for its animation — set server-side
    /// from its brain (`server::zombies::publish_zombie_anims`), not from
    /// input. [`ZombieAnim::None`] on everyone who isn't a zombie, which is
    /// also how a client tells a zombie apart. Discrete, same as `crouching`.
    pub zombie: ZombieAnim,
}

/// A `Zombies` zombie's animation state ([`PlayerPose::zombie`]). Its death
/// isn't one — that's `PlayerPose::alive`, same as anyone's.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZombieAnim {
    /// Not a zombie.
    #[default]
    None,
    /// Standing: looking around.
    Idle,
    /// Climbing up out of the ground (a fresh spawn) — animates as `Idle`,
    /// and the client breaks the ground open around it.
    Rising,
    /// Shambling along, arms down.
    Walk,
    /// Shambling along, arms up — a walker close to who it's after.
    WalkArmsUp,
    /// Running, arms up (a runner, later rounds).
    Run,
    /// Swinging at someone (see `crate::zombies::ZOMBIE_ATTACK_SECS`).
    Attack,
    /// A dog-round hellhound (`crate::dogs`) — always running; the client
    /// matches its legs to how fast it's really going.
    Dog,
    /// A boss (`crate::boss`), standing...
    BossIdle,
    /// ...walking (the client matches its legs to its real speed)...
    BossWalk,
    /// ...smashing someone close ([`crate::boss::ATTACK_SECS`] long)...
    BossSmash,
    /// ...or winding up a blast — the same clip, with a ball of energy
    /// forming overhead until it's hurled.
    BossBlast,
}

impl ZombieAnim {
    /// A zombie — or a hellhound, which counts as one for everything but
    /// its look.
    pub fn is_zombie(self) -> bool {
        self != ZombieAnim::None
    }

    pub fn is_dog(self) -> bool {
        self == ZombieAnim::Dog
    }

    pub fn is_boss(self) -> bool {
        matches!(
            self,
            ZombieAnim::BossIdle | ZombieAnim::BossWalk | ZombieAnim::BossSmash | ZombieAnim::BossBlast
        )
    }
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
            stabbing: false,
            alive: true,
            zombie: ZombieAnim::None,
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
            stabbing: end.stabbing,
            alive: end.alive,
            zombie: end.zombie,
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
    /// Whether a knife stab is playing out this tick (from the stab until
    /// its swing is over) — see `PlayerPose::stabbing`.
    pub stabbing: bool,
    /// Camera Y offset from standing this tick (metres, `<= 0.0` — see the
    /// client's `Slide::drop`), so the kill cam can reproduce crouch / slide /
    /// prone height exactly instead of always replaying at standing height.
    pub crouch_drop: f32,
    /// `Zombies`: the interact key's held down to revive a downed teammate
    /// (`server::revive` picks the nearest one in reach).
    pub revive: bool,
    /// With `fire`: every enemy this client saw the shot strike — tested
    /// against their actual animated models as drawn on its screen, an arm or
    /// a finger included — for the server to check over and apply
    /// ([`crate::ballistics::resolve_claimed_hits`]). Empty: a clean miss.
    /// `None` (a bot's shot, which has no client): the server tests it
    /// against simple capsules itself.
    #[reflect(ignore)]
    pub fire_hits: Option<Vec<ShotClaim>>,
}

/// One enemy a player's shot struck, as their client saw it
/// ([`PlayerInput::fire_hits`]).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct ShotClaim {
    pub target: ClaimTarget,
    /// Which part of them — from the bone the hit landed on
    /// ([`crate::hitbox::zone_of_bone`]).
    pub zone: crate::ballistics::HitZone,
    /// Where on their model.
    pub point: [f32; 3],
}

/// Client → server: one of our projectiles — a Ray Gun bolt, a thrown
/// knife — struck an enemy on our screen, as [`ShotClaim`]s do for bullets
/// (`None`: a bolt reached its surface without striking one, so it can
/// burst there at once).
/// `origin` is where it was fired / thrown from, exactly as we sent it,
/// which picks out which of ours it was. The server checks it over
/// (`server::raygun`, `server::knives`) before anything takes damage.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProjectileClaim {
    pub projectile: Projectile,
    pub origin: [f32; 3],
    pub claim: Option<ShotClaim>,
}

/// A [`ProjectileClaim`]'s projectile.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projectile {
    RayGun,
    Knife,
}

/// Server → the thrower only: where the knife they threw from `origin` (as
/// they sent it) will be while it can still kill —
/// `[x, y, z, seconds since the throw]`
/// ([`crate::throwing_knife::lethal_path`]) — for their client to test it
/// against everyone's models along the way.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KnifeFlight {
    pub origin: [f32; 3],
    pub path: Vec<[f32; 4]>,
}

/// Who a [`ShotClaim`] hit.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum ClaimTarget {
    /// A player entity — another player, a `FreeForAll` bot or a `Zombies`
    /// zombie, hellhound or boss — by its [`PeerId::to_bits`].
    Player(u64),
    /// A `Freestyle` target bot ([`Bot`]), by where its feet were on the
    /// shooter's screen — the server takes the one nearest there.
    Bot([f32; 3]),
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
            stabbing: false,
            crouch_drop: 0.0,
            revive: false,
            fire_hits: None,
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
    /// What fired it ([`WeaponId::as_u8`]) — a Ray Gun's is a green bolt
    /// and burst, not a tracer.
    pub weapon: u8,
}

/// Server → everyone else in the shooter's lobby: a Ray Gun bolt just left
/// `shooter`'s barrel, headed for `end` (the surface it'll burst on if
/// nothing gets in its way). Its [`ShotResolved`] follows once it has
/// really landed — the server flies it at
/// [`crate::weapon::RAYGUN_BOLT_SPEED`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct RayGunFired {
    pub shooter: PeerId,
    pub origin: [f32; 3],
    pub end: [f32; 3],
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

/// How long a `FreeForAll` match's ending freezes play for (seconds): from the
/// moment it ends ([`MatchEnding`]) until the end-of-match replay starts. Fixed
/// and shared, so the server's wait and the client's "VICTORY / DEFEAT" screen
/// always line up — and long enough for the final kill's replay to finish
/// recording (`server::killcam`).
pub const MATCH_END_FREEZE_SECS: f32 = 2.0;

/// Server → everyone in a `FreeForAll` lobby: the match just ended (the kill
/// limit was reached, or time ran out). From here until [`MatchOver`] (about
/// [`MATCH_END_FREEZE_SECS`] later) nothing that happens counts — the server
/// ignores hits, and clients stop taking input and show VICTORY / DEFEAT.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MatchEnding {
    /// Everyone tied for the top score — they all get VICTORY (the results
    /// screen counts a tie for first as a win too).
    pub winners: Vec<PeerId>,
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
    /// The primary held on this frame ([`WeaponId::as_u8`]) — the replay
    /// shows that gun, whatever the viewer carries.
    pub weapon: u8,
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

/// One bot's or other player's state at one moment of a kill-cam window — a
/// compact copy of what the world saw of them (`Bot` / `PlayerPose`), taken
/// every [`ACTOR_STRIDE_TICKS`] server ticks. The replay interpolates between
/// them, so they move and animate as they actually did, not as they are now.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ActorSample {
    /// Server ticks since the start of the replay's first frame.
    pub tick: u16,
    /// Feet for a bot; the *eye* for a player (like [`PlayerPose::translation`]).
    pub pos: [f32; 3],
    pub yaw: f32,
    /// Aim-down-sight amount, `0..=255` for `0.0..=1.0` (players only).
    pub ads: u8,
    /// `ActorSample::ALIVE` and friends.
    pub flags: u8,
}

impl ActorSample {
    pub const ALIVE: u8 = 1;
    pub const CROUCHING: u8 = 1 << 1;
    pub const RELOADING: u8 = 1 << 2;
    pub const JUMPING: u8 = 1 << 3;
    pub const SLIDING: u8 = 1 << 4;
    pub const STABBING: u8 = 1 << 5;

    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    pub fn ads_amount(&self) -> f32 {
        self.ads as f32 / 255.0
    }

    /// A player's pose as a sample (at `tick`).
    pub fn from_pose(tick: u16, pose: &PlayerPose) -> Self {
        let mut flags = 0;
        for (on, bit) in [
            (pose.alive, Self::ALIVE),
            (pose.crouching, Self::CROUCHING),
            (pose.reloading, Self::RELOADING),
            (pose.jumping, Self::JUMPING),
            (pose.sliding, Self::SLIDING),
            (pose.stabbing, Self::STABBING),
        ] {
            if on {
                flags |= bit;
            }
        }
        Self {
            tick,
            pos: pose.translation.to_array(),
            yaw: pose.yaw,
            ads: (pose.ads_t.clamp(0.0, 1.0) * 255.0).round() as u8,
            flags,
        }
    }

    /// A Freestyle bot as a sample (at `tick`).
    pub fn from_bot(tick: u16, bot: &Bot) -> Self {
        Self {
            tick,
            pos: bot.pos.to_array(),
            yaw: bot.yaw,
            ads: 0,
            flags: if bot.alive { Self::ALIVE } else { 0 },
        }
    }
}

/// One bot or other player (all shown as `models/characters/soldier.glb`) across a kill-cam
/// window, oldest sample first. Never the killer themselves: the replay is a
/// first-person fly-through of their own recorded view, so they'd have no
/// body to show. It includes whoever was shot, whose death shows up as their
/// `alive` flag clearing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KillCamActor {
    /// A `Freestyle` target bot (its samples' `pos` is its feet).
    pub bot: bool,
    /// A `FreeForAll` bot player (recorded like any other player) — so the
    /// replay can draw it with the bot look too.
    pub bot_player: bool,
    pub samples: Vec<ActorSample>,
}

/// Every how many server ticks the world's bots and players are sampled for
/// kill cams ([`ActorSample`]) — 16 Hz at the 64 Hz tick, plenty to interpolate
/// walking.
pub const ACTOR_STRIDE_TICKS: u64 = 4;

/// Server → everyone in a lobby: replay the killer's view around a kill (the
/// usual 3 s before to 1.5 s after; longer for a best play, which spans a run
/// of kills). `samples` are oldest-first at `TICK_HZ`; `kill_index` is the frame
/// the shot landed on. `actors` are every bot and other player over the same
/// window, so the replay shows them where they were and doing what they were
/// doing then — not where they are now.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KillCam {
    pub killer_name: String,
    pub samples: Vec<KillCamSample>,
    pub kill_index: u32,
    pub actors: Vec<KillCamActor>,
    /// True only for the single highest-scoring shot of the match, resent
    /// right before [`MatchOver`] once the clock hits zero. The client plays
    /// this one with a slow-mo ramp around the kill and a "BEST PLAY" banner
    /// instead of "KILLCAM", and holds the match-results screen off until it
    /// finishes.
    pub best_play: bool,
    /// Set (together with `best_play`, which makes it play in slow motion and
    /// hold the results screen) when this is a `FreeForAll` match's *final
    /// kill* rather than its best play — only changes the banner text.
    pub final_kill: bool,
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

/// Which replay a [`GameMode::FreeForAll`] lobby plays for everyone when the
/// match ends, whoever won. The leader picks it in the lobby room; every member
/// sees the current choice.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EndCam {
    /// The most kills one player got inside a kill cam's window
    /// (`server::killcam`), replayed with the usual best-play slow-mo.
    #[default]
    BestPlay,
    /// The last kill of the match, in the same ramped slow-mo.
    FinalKill,
}

impl EndCam {
    pub fn label(self) -> &'static str {
        match self {
            EndCam::BestPlay => "BEST PLAY",
            EndCam::FinalKill => "FINAL KILL",
        }
    }
}

/// Client (party leader) → server: pick a `FreeForAll` lobby's [`EndCam`]
/// before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetEndCam {
    pub cam: EndCam,
}

/// Client (party leader) → server: set [`GameMode::Zombies`]'s starting round,
/// starting points and pre-game countdown ([`Lobby::start_round`] /
/// [`Lobby::start_points`] / [`Lobby::countdown_secs`]) before starting —
/// clamped to `crate::zombies::{MAX_START_ROUND, MAX_START_POINTS,
/// MAX_COUNTDOWN_SECS}`.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetZombiesStart {
    pub round: u32,
    pub points: u32,
    pub countdown: u32,
}

/// Client (party leader) → server: set [`GameMode::FreeForAll`]'s kill limit
/// before starting.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetKillLimit {
    pub kills: u32,
}

/// Client (party leader) → server: add `count` bots at `difficulty` to a
/// `FreeForAll` lobby that hasn't started. Can be sent repeatedly with
/// different difficulties; the lobby holds at most
/// [`crate::bot_players::MAX_BOTS`] bots in total, so a request that would go
/// over is trimmed to fit.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct AddBots {
    pub count: u8,
    pub difficulty: crate::bot_players::BotDifficulty,
}

/// Client (party leader) → server: remove every bot from the lobby.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct ClearBots;

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
    /// Apply as soon as it arrives (the match just started — there's no kill
    /// cam to wait for) rather than after the usual respawn delay.
    pub immediate: bool,
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
    /// `Some` for a computer-controlled bot (`FreeForAll` only — see
    /// [`crate::bot_players`]) at that difficulty; `None` for a real player.
    /// A bot's `peer` is a fake one ([`crate::bot_players::is_bot_peer`]) with
    /// no client behind it.
    pub bot: Option<crate::bot_players::BotDifficulty>,
    /// How many enemies this member has killed this game — zombies, other
    /// players or target bots (`FreeForAll` also counts them in `score`).
    pub kills: u32,
    /// `FreeForAll` / `Freestyle`: how many times this member has died this
    /// game (killed, or a fatal fall) — the leaderboard's DEATHS.
    #[serde(default)]
    pub deaths: u32,
    /// [`GameMode::Zombies`], this game: kills that were critical (a headshot
    /// or a knife kill), teammates revived, and times gone down — the
    /// results screen's other columns.
    pub critical_kills: u32,
    pub revives: u32,
    pub downs: u32,
    /// [`GameMode::Zombies`] perks this member has bought this game.
    pub perks: Vec<crate::perks::Perk>,
    /// [`GameMode::Zombies`] Pack-a-Punch level of each of this member's
    /// weapons this game ([`BuyPap`]).
    pub pap: crate::pap::PapLevels,
    /// The primary this member picked in the loadout ([`SetLoadout`]) — one
    /// of [`crate::weapon::LOADOUT_WEAPONS`], for the modes that have one
    /// ([`GameMode::has_loadout`]). Kept between games. What they spawn with
    /// next — see `primary` for what they're carrying now.
    pub loadout: WeaponId,
    /// Who this member plays as in [`GameMode::Zombies`] ([`SetOperator`]) —
    /// whose voice their quotes are in. Kept between games.
    pub operator: crate::operator::Operator,
    /// The primary this member is actually carrying this life (server-set):
    /// `loadout` at the start of a game and on every respawn, or straight
    /// away on a `FreeForAll` change made within
    /// [`crate::weapon::LOADOUT_SWAP_GRACE_SECS`] of spawning, before firing.
    /// Always the sniper in `Freestyle`. In `Zombies` the gun most recently
    /// taken (a wall buy or a pickup) — `weapons` is everything carried.
    pub primary: WeaponId,
    /// The two weapon slots this member carries (server-set): `primary` and
    /// the knife at the start of a game, then in `Zombies` whatever wall buys
    /// ([`BuyWallWeapon`]) and pickups ([`PickUpWeapon`]) swap in — never two
    /// of the same.
    pub weapons: [SlotWeapon; 2],
    /// [`GameMode::Zombies`]: this member's armor ([`crate::armor`]) —
    /// server-owned, cleared whenever a game starts and when they bleed out.
    #[serde(default)]
    pub armor: crate::armor::Armor,
    /// [`GameMode::Zombies`]: this member's field upgrade (the Aether
    /// Shroud, [`crate::field_upgrade`]) — its stored charges, the next one
    /// building, and the one in use. Server-owned, cleared whenever a game
    /// starts or ends.
    #[serde(default)]
    pub field_upgrade: crate::field_upgrade::FieldUpgrade,
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
    /// [`GameMode::FreeForAll`]'s end-of-match replay. Unused by `Freestyle`,
    /// which always replays its best-scoring shot.
    pub end_cam: EndCam,
    /// [`GameMode::Zombies`]: the round being played (1 = the first), kept
    /// after the game ends for the results screen. `0` before a game.
    pub round: u32,
    /// [`GameMode::Zombies`]: enemies still to kill this round (not yet
    /// spawned + alive).
    pub enemies_left: u32,
    /// [`GameMode::Zombies`]: of those, how many are spawned in right now
    /// (rising or up and about).
    pub enemies_active: u32,
    /// The party leader paused the running game ([`SetPaused`]): the server
    /// freezes the clock, bots, zombies, knives and shots, and every member's
    /// client stops taking gameplay input. Cleared whenever a game starts or
    /// ends, and when the leader leaves.
    pub paused: bool,
    /// Debug (the leader's egui panel, [`SetBotsPassive`]): bots — `Zombies`
    /// zombies and `FreeForAll` bots — still move and chase but never fire,
    /// for testing without dying. Kept for the lobby's lifetime, like its
    /// other settings; a new lobby starts with it off.
    pub bots_passive: bool,
    /// Debug (the leader's egui panel, [`SetBotsFrozen`]): bots — `Zombies`
    /// zombies, `FreeForAll` bots and `Freestyle` targets — stand where they
    /// are and do nothing: no moving, chasing, firing or swiping. Kept for
    /// the lobby's lifetime, like `bots_passive`.
    pub bots_frozen: bool,
    /// Debug (the leader's egui panel, [`SetBombTest`]): every zombie kill
    /// in this lobby explodes like a Bomb Shot trickshot, without the perk
    /// or the trickshot. Kept for the lobby's lifetime, like `bots_passive`.
    pub bomb_test: bool,
    /// Debug (the leader's egui panel, [`SetPowerUpTest`]): every zombie
    /// kill drops a power-up. Kept for the lobby's lifetime, like
    /// `bots_passive`.
    pub power_up_test: bool,
    /// Debug (the leader's egui panel, [`SetMolotovTest`]): every zombie
    /// kill drops a molotov. Kept for the lobby's lifetime, like
    /// `bots_passive`.
    pub molotov_test: bool,
    /// [`GameMode::Zombies`]: the timed power-ups running
    /// ([`crate::power_ups::PowerUp::timed`]) and their whole seconds left,
    /// in the order they started — grabbing one again restarts its timer
    /// but keeps its place. Server-owned (`server::power_ups`); cleared
    /// whenever a game starts or ends.
    pub active_power_ups: Vec<(crate::power_ups::PowerUp, u16)>,
    /// [`GameMode::Zombies`]: the round a game starts on (1 = the normal
    /// start), set by the leader before starting ([`SetZombiesStart`]).
    pub start_round: u32,
    /// [`GameMode::Zombies`]: the points every member starts a game with.
    pub start_points: u32,
    /// [`GameMode::Zombies`]: which perks the machines sell — the custom
    /// ones or Call of Duty's ([`SetPerkSet`]).
    pub perk_set: crate::perks::PerkSet,
    /// [`GameMode::Zombies`]: seconds of free roaming before the first round
    /// (`0` = none), set by the leader ([`SetZombiesStart`]).
    pub countdown_secs: u32,
    /// [`GameMode::Zombies`]: whole seconds of that countdown still to go in
    /// the running game (server-owned; `0` once the rounds have begun).
    pub countdown_left: u32,
    /// [`GameMode::Zombies`]: someone threw the power switch
    /// ([`TurnOnPower`]) — the map's lights are on. Cleared whenever a game
    /// starts or ends.
    pub power_on: bool,
    /// [`GameMode::Zombies`]: the Mystery Box's spin, while there is one
    /// ([`crate::mystery_box`]). Server-owned; cleared whenever a game starts
    /// or ends.
    #[serde(default)]
    pub mystery_box: Option<crate::mystery_box::MysteryBoxSpin>,
    /// [`GameMode::Zombies`]: the exfil ([`crate::exfil`]) — server-owned
    /// (`server::exfil`). Reset whenever a game starts; kept after one ends,
    /// for the results screen.
    #[serde(default)]
    pub exfil: crate::exfil::Exfil,
    /// [`GameMode::Zombies`]: the Rampage Inducer's on ([`ToggleRampage`],
    /// [`crate::rampage`]) — zombies faster, spawning quicker. Cleared
    /// whenever a game starts or ends.
    #[serde(default)]
    pub rampage: bool,
    pub members: Vec<LobbyMember>,
}

impl Lobby {
    pub fn has(&self, peer: PeerId) -> bool {
        self.members.iter().any(|m| m.peer == peer)
    }

    /// Whether this lobby's bots and zombies hold off attacking right now:
    /// the leader's debug "bots don't attack" ([`Self::bots_passive`]), or
    /// an exfil being called ([`crate::exfil::Exfil::Calling`]).
    pub fn bots_hold_fire(&self) -> bool {
        self.bots_passive || self.exfil == crate::exfil::Exfil::Calling
    }

    /// Whether the timed power-up `p` is running in this lobby's game.
    pub fn power_up_active(&self, p: crate::power_ups::PowerUp) -> bool {
        self.active_power_ups.iter().any(|(q, _)| *q == p)
    }

    /// The members with a real client behind them — the only ones a message can
    /// be sent to.
    pub fn real_peers(&self) -> Vec<PeerId> {
        self.members
            .iter()
            .filter(|m| m.bot.is_none())
            .map(|m| m.peer)
            .collect()
    }

    /// Every member tied for the highest score.
    pub fn top_scorers(&self) -> Vec<PeerId> {
        let top = self.members.iter().map(|m| m.score).max().unwrap_or(0);
        self.members
            .iter()
            .filter(|m| m.score == top)
            .map(|m| m.peer)
            .collect()
    }

    /// How many real players are in the lobby.
    pub fn real_count(&self) -> usize {
        self.members.iter().filter(|m| m.bot.is_none()).count()
    }

    /// How many bots are in the lobby.
    pub fn bot_count(&self) -> usize {
        self.members.iter().filter(|m| m.bot.is_some()).count()
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
    /// A lobby member pinged it ([`PingBot`]) within the last
    /// [`crate::bots::PING_SECS`] — every member's client shows a diamond over
    /// it, through walls. Cleared early if it dies.
    pub pinged: bool,
}

impl Ease for Bot {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| Bot {
            pos: Vec3::lerp(start.pos, end.pos, t),
            yaw: lerp_angle(start.yaw, end.yaw, t),
            alive: end.alive,
            fall: start.fall + (end.fall - start.fall) * t,
            pinged: end.pinged,
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
    /// `true` once it has stopped moving — it then lies there, outlined, for
    /// anyone to pick up ([`PickUpKnife`]) until it's removed.
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

/// A molotov in flight, replicated (interpolated) to every member of its
/// `Zombies` lobby. The server owns the flight (`server::molotovs`,
/// [`crate::molotov`]); clients draw the bottle and its burning rag. It's
/// removed the moment it breaks, and a [`MolotovFire`] takes its place.
///
/// The rotation frame: `Y` runs from the bottle's base to its neck.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrownMolotov {
    pub owner: PeerId,
    pub pos: Vec3,
    pub rot: Quat,
}

impl Ease for ThrownMolotov {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| ThrownMolotov {
            owner: end.owner,
            pos: Vec3::lerp(start.pos, end.pos, t),
            rot: Quat::slerp(start.rot, end.rot, t),
        })
    }
}

/// A patch of fire where a molotov broke, replicated to its lobby for as long
/// as it burns (`crate::molotov::FIRE_SECS`). `spots` are the points on the
/// surfaces it spread to ([`crate::molotov::fire_spots`]) — the flames are
/// drawn there, and the server burns whoever stands in them.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MolotovFire {
    pub owner: PeerId,
    pub center: Vec3,
    pub spots: Vec<Vec3>,
}

/// A monkey bomb in a `Zombies` lobby, flying or `landed` (then clapping
/// and singing, drawing zombies to it, until it blows up). The server owns
/// it (`server::monkey_bombs`, [`crate::monkey_bomb`]); clients draw it and
/// play its sounds from it.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrownMonkey {
    pub owner: PeerId,
    pub pos: Vec3,
    pub rot: Quat,
    pub landed: bool,
    /// Seconds from landing to blowing up — its lobby's at the throw
    /// ([`SetMonkeyFuse`]), so every client ends its sounds on time.
    pub fuse: f32,
}

impl Ease for ThrownMonkey {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| ThrownMonkey {
            owner: end.owner,
            pos: Vec3::lerp(start.pos, end.pos, t),
            rot: Quat::slerp(start.rot, end.rot, t),
            landed: end.landed,
            fuse: end.fuse,
        })
    }
}

/// A frag in a `Zombies` lobby, flying, rolling or at rest until its fuse
/// runs out. The server owns it (`server::frags`, [`crate::frag`]); clients
/// draw it.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrownFrag {
    pub owner: PeerId,
    pub pos: Vec3,
    pub rot: Quat,
}

impl Ease for ThrownFrag {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| ThrownFrag {
            owner: end.owner,
            pos: Vec3::lerp(start.pos, end.pos, t),
            rot: Quat::slerp(start.rot, end.rot, t),
        })
    }
}

/// A frag a zombie dropped (or a player swapped away), lying at `pos`
/// turned `yaw` radians, for any player to pick up ([`PickUpFrag`]).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FragDrop {
    pub pos: Vec3,
    pub yaw: f32,
}

/// A flash bang in a `Zombies` lobby, flying or rolling until it goes off.
/// The server owns it (`server::flash_bangs`, [`crate::flash_bang`]);
/// clients draw it.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrownFlashBang {
    pub owner: PeerId,
    pub pos: Vec3,
    pub rot: Quat,
}

impl Ease for ThrownFlashBang {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| ThrownFlashBang {
            owner: end.owner,
            pos: Vec3::lerp(start.pos, end.pos, t),
            rot: Quat::slerp(start.rot, end.rot, t),
        })
    }
}

/// A flash bang a zombie dropped, lying at `pos` turned `yaw` radians, for
/// any player to pick up ([`PickUpFlashBang`]).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FlashBangDrop {
    pub pos: Vec3,
    pub yaw: f32,
}

/// A monkey bomb a zombie dropped (or a player swapped away), lying at
/// `pos` turned `yaw` radians, for any player to pick up ([`PickUpMonkey`]).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MonkeyDrop {
    pub pos: Vec3,
    pub yaw: f32,
}

/// A molotov a zombie dropped, lying on its side at `pos` (turned `yaw`
/// radians about the vertical) for any player to pick up ([`PickUpMolotov`]).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MolotovDrop {
    pub pos: Vec3,
    pub yaw: f32,
}

/// A weapon a `Zombies` player dropped (swapping it for a wall buy or a
/// pickup), lying at `pos` turned `yaw` radians, for anyone to pick up
/// ([`PickUpWeapon`]) — with its Pack-a-Punch level and, for a gun, the
/// rounds it had.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WeaponDrop {
    pub weapon: SlotWeapon,
    pub pap: u8,
    pub mag: u32,
    pub reserve: u32,
    pub pos: Vec3,
    pub yaw: f32,
}

/// Client → server: buy `weapon` at its `Zombies` wall buy
/// ([`crate::wall_buy`]), in place of the one in weapon slot `slot` (the one
/// in the buyer's hands), which is dropped with `mag` / `reserve` rounds
/// (ammo is client-side). The server checks the sign's in reach, the weapon
/// isn't carried already and the points, and answers [`WallWeaponBought`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct BuyWallWeapon {
    pub weapon: WeaponId,
    pub slot: u8,
    pub mag: u32,
    pub reserve: u32,
}

/// Client → server (debug, `Zombies`): put `weapon` in slot `slot` (the one
/// in hand, dropped with `mag` / `reserve` rounds) for free — answered like
/// a wall buy, with [`WallWeaponBought`]. For trying out guns there's no
/// other way to get yet (the Ray Gun).
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct GiveWeapon {
    pub weapon: WeaponId,
    pub slot: u8,
    pub mag: u32,
    pub reserve: u32,
}

/// Server → the buyer only: their [`BuyWallWeapon`] went through — `weapon`
/// is now in slot `slot`, with a full load.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WallWeaponBought {
    pub weapon: WeaponId,
    pub slot: u8,
}

/// Client → server: buy armor up to `level` at the `Zombies` armor station
/// ([`crate::armor`]) — paying for every level on the way, and filling every
/// plate. The server checks it's in reach and the points.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct BuyArmor {
    pub level: u8,
}

/// Client → server: use a stored field upgrade charge (the Aether Shroud,
/// [`crate::field_upgrade`]). The server checks there's one, that none is
/// in use, and that the member's up in a running `Zombies` game.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct UseFieldUpgrade;

/// Client → server (debug): the party leader's field upgrade gets every
/// charge at once. Only in a running `Zombies` game; ignored from anyone
/// else.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct FillFieldUpgrade;

/// Client → server: fill every armor plate this member owns back up at the
/// station, for [`crate::armor::REFILL_COST`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct RefillArmor;

/// Client → server: spin the `Zombies` Mystery Box ([`crate::mystery_box`]).
/// Ammo and lethals are client-side, so the client says which lethal it's
/// carrying a full load of — the box won't land on that (nor on a gun the
/// server knows it carries). The server checks it's in reach, idle, and the
/// points ([`crate::mystery_box::COST`]), then starts the spin
/// ([`Lobby::mystery_box`]).
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SpinMysteryBox {
    pub knives_full: bool,
    pub molotovs_full: bool,
    pub monkeys_full: bool,
    #[serde(default)]
    pub frags_full: bool,
    #[serde(default)]
    pub flashes_full: bool,
}

/// Client → server: take the Mystery Box's prize this player spun for,
/// while it's on offer. A gun goes in slot `slot` (the one in hand), whose
/// weapon is dropped with `mag` / `reserve` rounds; a lethal fills them up
/// with it, dropping another kind they carry (`dropping`). Answered with
/// [`BoxPrizeTaken`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct TakeBoxPrize {
    pub slot: u8,
    pub mag: u32,
    pub reserve: u32,
    /// Another kind of lethal carried, dropped for a lethal prize.
    pub dropping: crate::lethal::Carried,
}

/// Server → the taker only: their [`TakeBoxPrize`] went through — a gun's
/// now in slot `slot` (full), or they carry a full load of the lethal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BoxPrizeTaken {
    pub prize: crate::mystery_box::BoxPrize,
    pub slot: u8,
}

/// Client → server: pick up the dropped weapon ([`WeaponDrop`]) nearest
/// this player, in place of the one in slot `slot`, which is dropped with
/// `mag` / `reserve` rounds. Answered with [`WeaponPickedUp`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PickUpWeapon {
    pub slot: u8,
    pub mag: u32,
    pub reserve: u32,
}

/// Server → the picker only: their [`PickUpWeapon`] worked — `weapon` is now
/// in slot `slot`, packed to `pap`, with the rounds it was dropped with.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WeaponPickedUp {
    pub weapon: SlotWeapon,
    pub slot: u8,
    pub pap: u8,
    pub mag: u32,
    pub reserve: u32,
}

/// Client → server: the local player just landed after falling `distance`
/// metres (apex to landing) at `speed` m/s. Movement is client-authoritative,
/// so the client is the one who knows it landed — but the *server* turns the
/// distance into damage (`shared::health::fall_damage`) and, if it kills,
/// answers with [`FallDeath`]. Only sent for falls of at least
/// `shared::health::FALL_REPORT_MIN_DISTANCE`.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct FallLanded {
    pub distance: f32,
    pub speed: f32,
}

/// Client → server: this player's client has just respawned (teleported to its
/// spawn point, after its kill cam played or was skipped). The server brings
/// the player back to life at once — full health, targetable, able to fire —
/// instead of leaving them a ghost until the respawn timer runs out, which a
/// skipped kill cam would otherwise outrun. (The timer stays as the fallback.)
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct RespawnReady;

/// Server → the victim only: that landing killed you — play the fall-death
/// effect. `speed` is the landing speed the client reported.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FallDeath {
    pub speed: f32,
}

/// Server → the attacker only: their shot / stab / thrown knife hurt a bot or
/// player (`kill: false` — a white X and the hit-marker sound) or killed one
/// (`kill: true` — a red X; the kill sound comes from its own feedback).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct HitMarker {
    pub kill: bool,
    /// Who was hurt, when it's a player-entity hit (a `FreeForAll` player or
    /// a `Zombies` zombie) — the attacker shows that zombie's health bar.
    /// `None` for a `Freestyle` target bot.
    pub victim: Option<PeerId>,
}

/// Server → the attacker only, in `Zombies`: one of their hits (any weapon —
/// shot, stab, thrown knife, fire, blast) took `damage` off a zombie, at
/// `point` (where it landed, or the middle of the body when there's no one
/// spot). The client floats the number up from there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ZombieDamaged {
    pub point: [f32; 3],
    pub damage: u32,
    /// A headshot or a knife — drawn yellow.
    pub critical: bool,
}

/// Server → every member of a `Zombies` lobby: `speaker`'s operator says
/// `line` (of [`crate::quotes::lines`] for them and `quote`) — from where
/// they are, for everyone else; in their own head, for them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct QuoteSaid {
    pub speaker: PeerId,
    pub quote: crate::quotes::Quote,
    pub line: u16,
}

/// Client → server: something only the client knows of happened to us —
/// down to our last magazine, or out of ammo ([`crate::quotes::Quote::LowAmmo`]
/// / [`crate::quotes::Quote::OutOfAmmo`], all it may ask for) — for our
/// operator to say so, if the server lets them.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct RequestQuote {
    pub quote: crate::quotes::Quote,
}

/// Client → server (the debug panel's "Quotes"): how long (s) after a dog
/// round begins, and after the exfil's called, someone says so. Only the
/// leader's counts; kept for the lobby's lifetime.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SetQuoteDelays {
    pub dog_round: f32,
    pub exfil: f32,
}

/// Server → every member of a lobby whose leader just left the game with the
/// party ([`EndGame`]): the game's over — back to the lobby room, together.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ReturnToLobby;

/// Server → the victim only: a boss's smash just landed on them — they're
/// thrown back at `velocity` (m/s, horizontal), which their client bleeds
/// off over a moment, sliding along any wall rather than through it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KnockedBack {
    pub velocity: [f32; 3],
}

/// Server → the victim only: someone (another player, a bot, a zombie,
/// hellhound or boss) just hurt them from `from` — where the attacker stood
/// (or the hit landed) — for the damage indicator that points there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct DamageTaken {
    pub from: [f32; 3],
}

/// Server → everyone in a `Zombies` lobby: a player's shot, stab or thrown
/// knife hit a zombie, hellhound or boss at `point`, travelling along `dir`
/// (unit) — every client squirts blood out from there, a bigger burst when
/// it was the `kill`ing blow. (Fire and blasts don't land anywhere in
/// particular, so they don't bleed.)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ZombieBlood {
    pub point: [f32; 3],
    pub dir: [f32; 3],
    pub kill: bool,
    pub target: BloodTarget,
}

/// What a [`ZombieBlood`] came out of — each has its own look.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BloodTarget {
    Zombie,
    Dog,
    Boss,
}

/// Server → everyone in a lobby: a thrown knife just killed someone (a bot in
/// `Freestyle`, another player in `FreeForAll`) at `point` — every client
/// plays the hit sound from there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrowingKnifeHit {
    pub point: [f32; 3],
}

/// Server → everyone in a lobby: a thrown knife struck a surface (a wall, the
/// ground, a crate — not a bot or player) at `point`. Every client plays one of
/// its impact sounds from there; `variant` is the server's random pick, so the
/// whole lobby hears the same clip (`variant % clips`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ThrowingKnifeImpact {
    pub point: [f32; 3],
    pub variant: u8,
}

/// Server → everyone in a lobby: a player attacked with the (regular) knife.
/// `stab` is whether it landed on a valid target (a bot in `Freestyle`, another
/// player in `FreeForAll`): then `point` is where on the victim, and clients
/// play one of the stab sounds from there; otherwise it's a swing and `point`
/// is the attacker's position, for one of the swing sounds. `variant` is the
/// server's random pick (`variant % clips`), so the whole lobby hears the same
/// clip.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KnifeAttackSound {
    pub point: [f32; 3],
    pub stab: bool,
    pub variant: u8,
}

/// Server → every member of a `Zombies` lobby: a Bomb Shot went off with
/// its base at `feet` (the dead zombie's feet) — show the explosion and play
/// its sound from there. `variant` is the server's random pick of sound
/// (`variant % clips`), so the whole lobby hears the same one. The blast
/// damage is the server's (`server::pvp::apply_bomb_blasts`). `phd` marks
/// PhD Flopper's purple one instead (a slide into an enemy, or a big drop).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BombExplosion {
    pub feet: [f32; 3],
    pub variant: u8,
    pub phd: bool,
    /// A frag going off ([`crate::frag`]): the same blast, with the frag's
    /// own explosion sound.
    #[serde(default)]
    pub frag: bool,
}

/// Server → every member of a `Zombies` lobby: a hellhound is about to
/// appear at `at` (the ground) — lightning strikes there, with the pre-spawn
/// sound, for `crate::dogs::DOG_PRE_SPAWN_SECS`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct DogLightning {
    pub at: [f32; 3],
}

/// Server → every member of a `Zombies` lobby: a hellhound just appeared at
/// `at` (its feet) — a flash and its spawn sound there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct DogSpawned {
    pub at: [f32; 3],
}

/// Server → every member of a `Zombies` lobby: hellhound `dog` (its bot peer
/// id) just blew up at `at` (its feet) — on a player, or killed. It's gone
/// at once; clients swap its body for the explosion.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct DogExploded {
    pub dog: PeerId,
    pub at: [f32; 3],
}

/// Server → every member of a `Zombies` lobby: a boss is about to appear at
/// `at` (the ground) — orange lightning strikes there, with its spawn sound,
/// for [`crate::boss::BOSS_PRE_SPAWN_SECS`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BossLightning {
    pub at: [f32; 3],
}

/// Server → every member of a `Zombies` lobby: a boss just appeared at `at`
/// (its feet) — a flash there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BossSpawned {
    pub at: [f32; 3],
}

/// Server → every member of a `Zombies` lobby: boss `boss` (its bot peer id)
/// just hurled blast `id` from `from`, flying along `dir` (unit) at `speed`
/// (m/s) until it blows up ([`BossBlastExploded`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BossBlastLaunched {
    pub id: u32,
    pub boss: PeerId,
    pub from: [f32; 3],
    pub dir: [f32; 3],
    pub speed: f32,
}

/// Server → every member of a `Zombies` lobby: blast `id` blew up at `at`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BossBlastExploded {
    pub id: u32,
    pub at: [f32; 3],
}

/// Client (debug) → server: the party leader strikes a boss in 15 m in
/// front of them, as a boss round would. Only in a running `Zombies` game;
/// ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SpawnBoss;

/// Client (party leader) → server: which perks the `Zombies` machines sell
/// ([`Lobby::perk_set`]) — before starting only.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SetPerkSet {
    pub set: crate::perks::PerkSet,
}

/// Client → server: the sender, sliding with PhD Flopper, just slid into an
/// enemy — the server checks the perk, its cooldown and that an enemy really
/// is close, then sets off the explosion at their feet.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PhdSlam;

/// Server → every member of a `Zombies` lobby: a zombie's swipe just landed
/// on the player standing at `at` — everyone plays the hit sound from there
/// (only the server knows a swing connected; the rest of the zombies' sounds
/// — moans, spawns, deaths — each client works out for itself from what it
/// sees).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ZombieSwipeLanded {
    pub at: [f32; 3],
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

/// Client → server: pick up the stopped throwing knife nearest this player
/// (pressed the interact key while its prompt was showing). The server finds
/// it, checks the range (`shared::throwing_knife::in_pickup_range`), removes
/// it and answers with [`KnifePickedUp`].
///
/// A player carries one kind of lethal at a time: swapping from another
/// kind, the client says what it's carrying ([`crate::lethal::Carried`]) and
/// the server drops that many around them (only if the pickup worked).
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PickUpKnife {
    pub dropping: crate::lethal::Carried,
}

/// Server → the picker only: their [`PickUpKnife`] worked — one more
/// throwing knife, and it's now their only lethal (any molotovs were dropped).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KnifePickedUp;

/// Client → server: the player's throw animation reached the point where the
/// molotov leaves their hand (`Zombies` only) — see [`ThrowKnife`].
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct ThrowMolotov {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
}

/// Client → server: pick up the dropped molotov nearest this player. The
/// server checks the range and answers with [`MolotovPickedUp`]. Swapping
/// from another kind, what's carried of it is dropped (see [`PickUpKnife`]).
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PickUpMolotov {
    pub dropping: crate::lethal::Carried,
}

/// Client → server: the player's prime ran out and their throw reached the
/// release point — the monkey bomb leaves their hand (`Zombies` only, see
/// [`crate::monkey_bomb`] and [`ThrowKnife`]).
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct ThrowMonkey {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
}

/// Client → server: pick up the dropped monkey bomb nearest this player,
/// dropping what's carried of another kind (see [`PickUpKnife`]). Answered
/// with [`MonkeyPickedUp`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PickUpMonkey {
    pub dropping: crate::lethal::Carried,
}

/// Client → server (debug): how long (s) a monkey bomb thrown in this
/// member's lobby takes from landing to blowing up (default
/// [`crate::monkey_bomb::FUSE_SECS`]). Kept for the lobby's lifetime.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SetMonkeyFuse {
    pub secs: f32,
}

/// Client → server: a frag leaves the player's hand (`Zombies` only, see
/// [`crate::frag`]) — thrown from `origin` along `dir` with `fuse` seconds
/// of its fuse left (the pin came out at the lethal key's press), or, with
/// `in_hand`, cooked off before it could be: it goes off at `origin`.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct ThrowFrag {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
    pub fuse: f32,
    pub in_hand: bool,
}

/// Client → server: pick up the dropped frag nearest this player, dropping
/// what's carried of another kind (see [`PickUpKnife`]). Answered with
/// [`FragPickedUp`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PickUpFrag {
    pub dropping: crate::lethal::Carried,
}

/// Server → the picker only: their [`PickUpFrag`] worked — one more frag,
/// and it's now their only lethal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FragPickedUp;

/// Client → server: a flash bang leaves the player's hand (`Zombies` only,
/// see [`crate::flash_bang`]), from `origin` along `dir`.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct ThrowFlashBang {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
}

/// Client → server: pick up the dropped flash bang nearest this player,
/// dropping the other tactical it carries, if any (see [`PickUpKnife`]).
/// Answered with [`FlashBangPickedUp`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PickUpFlashBang {
    pub dropping: crate::lethal::Carried,
}

/// Client → server: craft one `kind` at the crafting table
/// ([`crate::crafting`]), dropping what's carried of another kind of the
/// same sort (lethal / tactical — see [`PickUpKnife`]). How many are carried
/// is client-side, so the client only asks when it isn't full; the server
/// checks the player's at the table, alive, and has the points
/// ([`crate::crafting::cost`]), then answers with [`EquipmentBought`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct BuyEquipment {
    pub kind: crate::lethal::LethalKind,
    pub dropping: crate::lethal::Carried,
}

/// Server → the buyer only: their [`BuyEquipment`] went through — one more
/// `kind`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct EquipmentBought {
    pub kind: crate::lethal::LethalKind,
}

/// Server → the picker only: their [`PickUpFlashBang`] worked — one more
/// flash bang.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FlashBangPickedUp;

/// Server → everyone in a `Zombies` lobby: `owner`'s flash bang went off at
/// `at` — the flash and its sound from there for everyone, and the white-out
/// for `owner` alone if they were close enough to see it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FlashBangDetonated {
    pub owner: PeerId,
    pub at: [f32; 3],
}

/// Server → the picker only: their [`PickUpMonkey`] worked — one more
/// monkey bomb, and it's now their only lethal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MonkeyPickedUp;

/// Server → everyone in a `Zombies` lobby: a thrown molotov broke at
/// `point` — every client plays the burst sound from there.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MolotovBurst {
    pub point: [f32; 3],
}

/// Server → the picker only: their [`PickUpMolotov`] worked — one more
/// molotov, and it's now their only lethal (any knives were dropped).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MolotovPickedUp;

/// Client → server (debug): the party leader turns [`Lobby::molotov_test`]
/// on or off. Ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetMolotovTest {
    pub on: bool,
}

/// Client → server: buy a sniper ammo refill at the `Zombies` ammo crate
/// (`crate::ammo`). The client checks it's at the crate and not already
/// full; the server checks the game, that they're alive, and the points,
/// then takes them and answers with [`AmmoBought`].
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct BuyAmmo;

/// Server → the buyer only: their [`BuyAmmo`] went through — fill the sniper.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AmmoBought;

/// A `Zombies` power-up lying where a zombie dropped it, replicated to the
/// lobby's members. `pos` is its spot on the ground (the client floats and
/// spins the model above it); `blinking` is set for the last
/// `crate::power_ups::BLINK_SECS` before it's gone. Server-owned
/// (`server::power_ups`), removed when grabbed, when it runs out, or when
/// the game ends.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PowerUpDrop {
    pub kind: crate::power_ups::PowerUp,
    pub pos: Vec3,
    pub blinking: bool,
}

/// Server → everyone in a `Zombies` lobby: `by` walked into a `kind` drop
/// at `pos` and set it off — every client plays its announcer sound (and
/// Max Ammo fills everyone's ammo); `by` also plays the grab sound.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PowerUpGrabbed {
    pub kind: crate::power_ups::PowerUp,
    pub by: PeerId,
}

/// Server → everyone in a `Zombies` lobby: a Nuke just killed the zombie
/// `peer` (one at a time, over [`crate::power_ups::NUKE_KILL_SECS`]) — every
/// client sets its body alight. Just a look: the fire hurts no one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ZombieNuked {
    pub peer: PeerId,
}

/// Client → server (debug): the party leader turns [`Lobby::power_up_test`]
/// on or off. Ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetPowerUpTest {
    pub on: bool,
}

/// Client → server: the sender picks `weapon` (one of
/// [`crate::weapon::LOADOUT_WEAPONS`]) as their primary
/// ([`LobbyMember::loadout`]). Any time between games; mid-game only in
/// `FreeForAll` (it swaps their gun at once if they're still within the
/// spawn grace — see [`LobbyMember::primary`] — otherwise on their next
/// spawn). Ignored once a `Zombies` game has started.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SetLoadout {
    pub weapon: WeaponId,
}

/// Client → server: the sender plays as `operator`
/// ([`LobbyMember::operator`]). Any time between games; ignored once a
/// `Zombies` game has started.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SetOperator {
    pub operator: crate::operator::Operator,
}

/// Client → server: the sender just went prone at `perk`'s machine. The
/// first in the lobby to do so at each machine gets
/// [`crate::perks::PRONE_BONUS_POINTS`] ([`ProneBonus`]); anything else is
/// ignored.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProneAtPerk {
    pub perk: crate::perks::Perk,
}

/// Server → the one player only: their [`ProneAtPerk`] paid out `points`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ProneBonus {
    pub points: u32,
}

/// Client → server (debug): the party leader drops a `kind` power-up at
/// their own feet — picked up at once, so it goes off for real. Only in a
/// running `Zombies` game; ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct DropPowerUp {
    pub kind: crate::power_ups::PowerUp,
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

/// Client → server: ping this `Freestyle` bot (the client checked it was
/// under its crosshair) so every member of the lobby sees it marked. `bot` is
/// the confirmed entity as the client knows it; lightyear maps it to the
/// server's on arrival (`add_map_entities`).
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct PingBot {
    pub bot: Entity,
}

impl MapEntities for PingBot {
    fn map_entities<M: EntityMapper>(&mut self, mapper: &mut M) {
        self.bot = mapper.get_mapped(self.bot);
    }
}

/// Client → server: buy `perk` from its machine ([`GameMode::Zombies`]) — or,
/// with `wunderfizz`, from Der Wunderfizz ([`crate::wunderfizz`]). The server
/// checks the sender is next to it (and that it's selling), can afford it
/// and doesn't already have it.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct BuyPerk {
    pub perk: crate::perks::Perk,
    pub wunderfizz: bool,
}

/// Client → server: pack `weapon` (the one in the sender's hands) up to
/// `level` at the Pack-a-Punch machine ([`GameMode::Zombies`]). The server
/// checks the power's on, the sender is at the machine, `level` is the one
/// after theirs, and they can afford it.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct BuyPap {
    pub weapon: crate::pap::PapWeapon,
    pub level: u8,
}

/// Client → server: pay to turn the power on ([`GameMode::Zombies`]). The
/// server checks the sender is at the switch (`crate::power`), can afford it
/// and that it isn't already on.
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct TurnOnPower;

/// Client → server: call the exfil — sent once interact's been held at the
/// radio for [`crate::exfil::HOLD_SECS`]. The server checks the sender is at
/// the radio, alive, and that it can be called ([`crate::exfil::available`]).
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct CallExfil;

/// Client → server: turn the Rampage Inducer on (`on`) or off — sent once
/// interact's been held at it for [`crate::rampage::HOLD_SECS`]. The server
/// checks the sender is at it and alive, and that it isn't already that way
/// ([`Lobby::rampage`]).
#[derive(Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ToggleRampage {
    pub on: bool,
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
/// party ("leave with party") — the game's called off (every player entity
/// despawned) but the lobby stays together, with the same leader and the
/// same settings, and every member is sent back to its room
/// ([`ReturnToLobby`]). (A leader leaving *without* the party, or any
/// non-leader leaving, sends [`LeaveLobby`] instead: that pulls just the one
/// player and, for the leader, promotes a replacement.)
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct EndGame;

/// Client → server (debug): the party leader turns [`Lobby::bots_passive`] on
/// or off. Ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetBotsPassive {
    pub passive: bool,
}

/// Client → server (debug): the party leader turns [`Lobby::bots_frozen`] on
/// or off. Ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetBotsFrozen {
    pub frozen: bool,
}

/// Client → server (debug): the party leader turns [`Lobby::bomb_test`] on
/// or off. Ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetBombTest {
    pub on: bool,
}

/// Client → server: the party leader pauses / resumes the running game for
/// the whole party ([`Lobby::paused`]). Ignored from anyone else.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct SetPaused {
    pub paused: bool,
}

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
        app.add_message::<RayGunFired>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<LobbyError>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<TrickScore>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<MatchOver>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<MatchEnding>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<KillCam>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<RemoteSound>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<PlayerRespawn>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<PlayerKilledBy>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<HitMarker>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ZombieDamaged>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ZombieBlood>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<KnifeFlight>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<DamageTaken>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<KnockedBack>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ReturnToLobby>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<QuoteSaid>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_trigger::<RequestQuote>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetQuoteDelays>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_message::<BombExplosion>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<DogLightning>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<DogSpawned>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<DogExploded>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<BossLightning>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<BossSpawned>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<BossBlastLaunched>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<BossBlastExploded>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ZombieSwipeLanded>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<FallDeath>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ThrowingKnifeHit>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ThrowingKnifeImpact>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<KnifeAttackSound>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<KnifePickedUp>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<MolotovPickedUp>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<MonkeyPickedUp>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<FragPickedUp>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<FlashBangPickedUp>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<FlashBangDetonated>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<EquipmentBought>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<MolotovBurst>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<AmmoBought>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<WallWeaponBought>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<BoxPrizeTaken>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<WeaponPickedUp>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<PowerUpGrabbed>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ZombieNuked>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<ProneBonus>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<crate::revive::PlayerWentDown>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_message::<crate::revive::PlayerRevived>()
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
        app.add_trigger::<SetPaused>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetBotsPassive>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetBotsFrozen>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetPowerUpTest>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<DropPowerUp>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SpawnBoss>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetBombTest>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetTimeLimit>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetGameMode>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetMap>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetKillLimit>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetZombiesStart>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetEndCam>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<AddBots>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ClearBots>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<FellToDeath>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ThrowKnife>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PickUpKnife>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ThrowMolotov>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PickUpMolotov>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ThrowMonkey>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PickUpMonkey>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetMonkeyFuse>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ThrowFrag>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PickUpFrag>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ThrowFlashBang>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PickUpFlashBang>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<BuyEquipment>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<BuyWallWeapon>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SpinMysteryBox>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<BuyArmor>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<RefillArmor>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<UseFieldUpgrade>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<FillFieldUpgrade>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<TakeBoxPrize>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PickUpWeapon>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<GiveWeapon>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetMolotovTest>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<FallLanded>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<RespawnReady>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<BuyPerk>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<BuyPap>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<TurnOnPower>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<CallExfil>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ToggleRampage>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<BuyAmmo>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ProneAtPerk>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetPerkSet>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PhdSlam>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetLoadout>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<SetOperator>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<PingBot>()
            .add_map_entities()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_trigger::<ProjectileClaim>()
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

        // Replicated as-is (no prediction / interpolation): the client only
        // ever reads it, off the replicated entity.
        app.register_component::<PlayerHealth>();
        // Same: only changes on events (`crate::revive`).
        app.register_component::<crate::revive::Downed>();

        app.register_component::<Bot>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();

        // Static once dropped (only `blinking` flips), so no interpolation.
        app.register_component::<PowerUpDrop>();

        app.register_component::<ThrownKnife>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();

        app.register_component::<ThrownMolotov>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();

        app.register_component::<ThrownMonkey>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();
        app.register_component::<MonkeyDrop>();
        app.register_component::<ThrownFrag>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();
        app.register_component::<FragDrop>();
        app.register_component::<ThrownFlashBang>()
            .add_interpolation(InterpolationMode::Full)
            .add_linear_interpolation_fn();
        app.register_component::<FlashBangDrop>();

        // Static once spawned, so no interpolation.
        app.register_component::<MolotovFire>();
        app.register_component::<MolotovDrop>();
        app.register_component::<WeaponDrop>();

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
