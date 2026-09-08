//! bevy-trickshot — first-person prototype.
//!
//! Loads `assets/sniper.glb` and parents it to the player camera as a view
//! model. The glTF ships a single baked clip (`allanims`) that packs every
//! action back to back; `SEGMENTS` slices it into the individual animations by
//! frame range.
//!
//! Controls (all rebindable — see `keybinds.rs`; defaults shown):
//!   * `W` / `A` / `S` / `D` — move
//!   * `Left Shift`          — sprint (drops when you stop moving)
//!   * `C`                   — crouch (still) / slide (moving); jump cancels a slide
//!   * `Left Ctrl`           — prone (still) / dolphin dive (moving)
//!   * `B`                   — jump
//!   * `T`                   — teleport back onto the building
//!   * mouse                 — look around
//!   * right mouse (hold)    — aim down sight
//!   * left mouse            — fire
//!   * `R`                   — reload
//!   * `L`                   — lock / unlock the cursor (debug mode)
//!   * `Esc`                 — open / close the settings menu (see `menu.rs`)
//!
//! Sensitivity, FOV, username and keybinds are configured in the `Esc` menu and
//! persisted (`settings.rs`). Turn on **Debug Mode** there to show the egui
//! tuning panels (`ads_tuning_ui`, top-right) for the ADS pose, muzzle flash,
//! smoke, movement, weapon sway, camera shake and the sky.

mod keybinds;
mod killcam;
mod lobby_ui;
mod menu;
mod net;
mod practice;
mod settings;
mod ui;
mod updater;

/// Top-level screen the client is on. Gameplay + world rendering only run in
/// [`AppState::InGame`]; in `MainMenu` / `InLobby` the opaque lobby UI covers
/// the (always-loaded) world and the gameplay systems are gated off.
#[derive(States, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    /// Main menu / lobby browser.
    #[default]
    MainMenu,
    /// In a lobby room, waiting for the leader to start.
    InLobby,
    /// In the shared world (or solo Practice).
    InGame,
}

use std::f32::consts::{FRAC_PI_2, PI};

use keybinds::KeyBindings;
use settings::Settings;

use bevy::{
    animation::RepeatAnimation,
    audio::Volume,
    core_pipeline::bloom::Bloom,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    input::mouse::AccumulatedMouseMotion,
    math::{Affine2, FloatExt},
    pbr::{CascadeShadowConfigBuilder, DistanceFog, FogFalloff, NotShadowCaster},
    prelude::*,
    render::{
        render_asset::RenderAssetUsages,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::{NoFrustumCulling, RenderLayers},
    },
    scene::SceneInstanceReady,
    ui::IsDefaultUiCamera,
    window::{CursorGrabMode, PrimaryWindow},
};
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

/// Render layer used by the view model (the gun + arms) and its dedicated
/// camera, so the weapon never clips into world geometry.
const VIEW_MODEL_RENDER_LAYER: usize = 1;
/// Render layer for the reticle quad — only the scope camera renders it, so the
/// crosshair lives inside the scope image and nowhere else.
const SCOPE_OVERLAY_LAYER: usize = 2;
/// Empty render layer for the HUD camera, which renders after everything else
/// (including the view-model camera) so the UI is never covered by the gun.
const UI_LAYER: usize = 4;

/// Radius of the sky sphere. Kept inside the camera far plane; the sphere
/// follows the camera so the player never reaches its edge.
const SKY_RADIUS: f32 = 900.0;

/// A ~3-storey box to shoot from. Its top face is at `BUILDING_CENTER.y +
/// BUILDING_SIZE.y / 2`.
const BUILDING_SIZE: Vec3 = Vec3::new(8.0, 10.0, 8.0);
const BUILDING_CENTER: Vec3 = Vec3::new(0.0, 5.0, 8.0);
/// Where the player spawns / `T` teleports to: on the roof, at eye height.
const SPAWN_POS: Vec3 = Vec3::new(0.0, 11.7, 8.0);
/// Player camera height above the feet — used to test the feet against surfaces.
const EYE_HEIGHT: f32 = 1.7;
/// Defaults for the "Movement" panel section (all live-adjustable).
const WALK_SPEED: f32 = 6.0;
const SPRINT_SPEED: f32 = 10.5;
const GRAVITY: f32 = 22.0;
const JUMP_SPEED: f32 = 8.0;
/// Feet within this distance above a surface still count as standing on it.
const GROUND_SNAP: f32 = 0.5;

/// Defaults for the "Slide" / "Dive & prone" panel sections.
const CROUCH_DROP: f32 = 0.8;
const CROUCH_SPEED: f32 = 2.5;
const SLIDE_DUCK_SPEED: f32 = 16.0;
const SLIDE_STAND_SPEED: f32 = 7.0;
const SLIDE_SPEED: f32 = 9.0;
const SLIDE_SPRINT_BONUS: f32 = 4.5;
const SLIDE_FRICTION: f32 = 7.5;
const SLIDE_MIN_SPEED: f32 = 1.6;
const SLIDE_MAX_TIME: f32 = 1.6;
const PRONE_DROP: f32 = 1.35;
const PRONE_SPEED: f32 = 0.7;
const DIVE_SPEED: f32 = 8.5;
const DIVE_JUMP: f32 = 8.5;
const DIVE_TUCK_SPEED: f32 = 25.0;
/// A dolphin dive also counts as "landed" once the tucked camera drops within
/// this of the surface — the belly hits before the standing feet would.
const DIVE_CLEARANCE: f32 = 0.3;

const MOUSE_SENSITIVITY: Vec2 = Vec2::new(0.003, 0.002);
const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.02;

/// Seconds to go from hip to full aim-down-sight (and back).
const ADS_DURATION: f32 = 0.26;
// Hip FOV is now a player setting (`Settings::fov`, default 90°). Everything on
// screen — inside the scope or not — is drawn at the current FOV, so shrinking it
// toward `AdsTuning::fov_deg` is the scope "magnification".
/// Starting ADS FOV; lower is more zoom. Adjustable live in the tuning panel.
const ADS_FOV_DEG: f32 = 9.5;

// --- scope (render-to-texture) -------------------------------------------------
// A second camera renders the world only (no view model) through a narrow FOV
// into `SCOPE_RT_SIZE`² and that image is shown on the scope's rear lens, so the
// player looks *through* the scope instead of down the tube. The scope camera is
// only active while `ads.t > 0`, so the extra pass costs nothing at the hip.
/// Render-target resolution for the scope view.
const SCOPE_RT_SIZE: u32 = 512;
/// Scope camera FOV (degrees). Much narrower than the main camera = the
/// magnification. Adjustable live in the tuning panel.
const SCOPE_FOV_DEG: f32 = 6.5;
/// Distance (metres) the reticle quad sits in front of the scope camera.
const RETICLE_DIST: f32 = 0.2;
/// ADS amount below which the scope camera is switched off, so no stale frame
/// is rendered at the hip. The lens itself stays visible either way — see below.
const SCOPE_SHOW_AT: f32 = 0.02;

// The rear lens is one lit `StandardMaterial`. Off-aim it reads as a smooth,
// strongly reflective coated-glass disc catching the sun; as `Ads::t` → 1 the
// mirror sheen is dialled out and the render-to-texture sight picture (carried
// on `emissive`) fades in, so glare can't wash out the shot.
/// Cool anti-reflective-coating tint of the glass when not aiming.
const LENS_TINT: (f32, f32, f32) = (0.14, 0.21, 0.34);
/// Lens surface roughness at the hip (low = tight, mirror-like highlight) and
/// while fully scoped (higher = the sheen spreads out and dims).
const LENS_ROUGHNESS_HIP: f32 = 0.04;
const LENS_ROUGHNESS_ADS: f32 = 0.55;
/// Metalness / reflectance of the glass at the hip; both lerp toward a plain
/// dielectric backing as the player scopes in.
const LENS_METALLIC_HIP: f32 = 0.65;
const LENS_REFLECTANCE_HIP: f32 = 1.0;
/// Mouse sensitivity is scaled by this at full ADS so the zoomed view isn't
/// twitchy.
const ADS_SENSITIVITY_SCALE: f32 = 0.4;

// --- daylight look (bright, mostly-sunny midday) -----------------------------
// These are calibrated against Bevy's *default* camera exposure — a genuinely
// physical sun (~100k lux) would also need a `Exposure` retune on all three
// cameras, which is a separate pass. Tune these in-game.
/// Directional "sun" strength (lux).
const SUN_LUX: f32 = 28_000.0;
/// Sun colour — a faint warm (~6000 K).
const SUN_COLOR: Color = Color::srgb(1.0, 0.95, 0.88);
/// Ambient fill, tinted like a clear sky so shadows read blue rather than black.
const SKY_AMBIENT_COLOR: Color = Color::srgb(0.60, 0.73, 0.92);
const SKY_AMBIENT_LUX: f32 = 200.0;
/// Distance haze — pale sky blue; this is "sunny with air", not "foggy".
const FOG_COLOR: Color = Color::srgb(0.72, 0.80, 0.90);
/// Roughly the distance (m) at which geometry fades fully into the haze.
const FOG_VISIBILITY_M: f32 = 1300.0;

/// Frame rate the `allanims` clip was baked at in Blender. If the segment cuts
/// below look off, check the console on startup: the game logs the clip's real
/// duration and the fps a 156-frame timeline would imply.
const ANIM_FPS: f32 = 24.0;

/// Magazine capacity and the total number of magazines the player carries
/// (current mag + reserve = `MAG_SIZE * TOTAL_MAGS`).
const MAG_SIZE: u32 = 5;
#[allow(dead_code)] // TEMP: unused while reserve is hard-coded for reload testing
const TOTAL_MAGS: u32 = 6;

/// Indices into `SEGMENTS`.
const SEG_SHOOT: usize = 0;
const SEG_RECHAMBER: usize = 1;
const SEG_RELOAD: usize = 2;
const SEG_HIDE: usize = 3;
const SEG_SHOW: usize = 4;

/// Movement is this much faster in every stance while the (model-less) secondary
/// is equipped — the knife is lighter than the sniper.
const SECONDARY_MOVE_MULT: f32 = 1.12;

/// Camera shake: one shot adds `SHAKE_ADD` trauma (capped at 1), which decays at
/// `SHAKE_DECAY` per second. The visible offset scales with `trauma²`, so it is
/// violent immediately and gone in a fraction of a second.
///
/// Two things ride on trauma, both on the `CameraShake` node (gun + cameras
/// together, so the gun stays locked to the screen while the world swings):
/// * a small positional up/down + side/side jitter (`SHAKE_POS_MAX`), and
/// * a **view punch** — a directional pitch-up (`SHAKE_VIEW_PUNCH_DEG`) that
///   recovers with trauma, plus rotational chaos (`SHAKE_VIEW_JITTER_DEG`) on
///   top. Scaled down while scoped so ADS stays controllable.
///
/// The gun *also* shudders relative to the camera — `weapon_recoil_shudder`
/// punches the view model back toward the eye (`SHAKE_WEAPON_KICK`) and climbs
/// the muzzle (`SHAKE_WEAPON_KICK_DEG`) as trauma decays. That one never touches
/// aim.
///
/// The forward / back move is separate again: a single backward *kick* on each
/// shot, on the `CameraRecoil` node, which carries only the cameras (not the
/// gun). It snaps the eye back and eases home over ~a tenth of a second, so the
/// scope's rear lens — which the fire animation yanks toward the face — stays in
/// front of the eye instead of sliding past it. All of this is live-tunable
/// (`ShakeSettings`, "Camera shake" panel section).
const SHAKE_ADD: f32 = 0.85;
const SHAKE_DECAY: f32 = 2.0;
const SHAKE_FREQ: f32 = 75.0;
/// Peak camera translation (world units) on the X and Y axes at full trauma.
const SHAKE_POS_MAX: f32 = 0.075;
/// Peak upward view-punch pitch (degrees) at full trauma — the CoD "kick".
const SHAKE_VIEW_PUNCH_DEG: f32 = 6.6;
/// Amplitude (degrees) of the random yaw / pitch / roll chaos on top of the
/// punch, scaled by `trauma²`.
const SHAKE_VIEW_JITTER_DEG: f32 = 2.9;
/// Metres the view model shudders back toward the eye at full trauma.
const SHAKE_WEAPON_KICK: f32 = 0.2;
/// Muzzle-climb rotation (degrees) applied to the view model at full trauma.
const SHAKE_WEAPON_KICK_DEG: f32 = 7.5;
/// Metres the camera (not the gun) snaps backward on each shot.
const SHAKE_RECOIL_KICK: f32 = 0.0;
/// How fast that backward kick eases back to zero (larger = snappier return).
const SHAKE_RECOIL_RETURN: f32 = 8.5;

/// Seconds for the muzzle flash to go from full to gone (it pops on instantly).
const MUZZLE_FLASH_TIME: f32 = 0.06;

/// Hard cap on live smoke particles, and the most that can spawn in one frame.
const SMOKE_MAX: usize = 500;
const SMOKE_MAX_PER_FRAME: f32 = 8.0;

/// Hard cap on live bullet-impact particles (rocks + dust together).
const IMPACT_MAX: usize = 400;

/// `rocks.png` is a loose grid of rocks; each rock sprite shows one cell of this
/// many columns × rows so it's a single rock, not the whole sheet.
const ROCK_COLS: u32 = 3;
const ROCK_ROWS: u32 = 4;

/// Cheap deterministic hash → a float in `[0, 1)`.
fn rand01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    x ^= x >> 22;
    x as f32 / u32::MAX as f32
}

/// Cheap deterministic hash → a roll angle in `[-PI, PI)`.
pub(crate) fn rand_roll(seed: u32) -> f32 {
    rand01(seed) * (PI * 2.0) - PI
}

/// The individual animations packed into the single `allanims` clip, as
/// `[start, end)` frame ranges lifted straight from the Blender timeline.
/// Adjust these until every section is exactly right, then rebuild.
const SEGMENTS: [AnimationSegment; 7] = [
    AnimationSegment::new("Shoot", 0.0, 9.0),
    AnimationSegment::new("Rechamber", 9.0, 48.0),
    AnimationSegment::new("Reload", 48.0, 92.0),
    AnimationSegment::new("Hide", 92.0, 101.0),
    AnimationSegment::new("Show", 101.0, 113.0),
    AnimationSegment::new("Adjust Grip", 113.0, 132.0),
    AnimationSegment::new("Melee", 132.0, 156.0),
];

#[derive(Clone, Copy)]
struct AnimationSegment {
    name: &'static str,
    start_frame: f32,
    end_frame: f32,
}

impl AnimationSegment {
    const fn new(name: &'static str, start_frame: f32, end_frame: f32) -> Self {
        Self {
            name,
            start_frame,
            end_frame,
        }
    }

    fn start_secs(&self) -> f32 {
        self.start_frame / ANIM_FPS
    }

    fn end_secs(&self) -> f32 {
        self.end_frame / ANIM_FPS
    }
}

/// Play `seg` from its start frame at normal speed, not looping.
fn play_segment(player: &mut AnimationPlayer, node: AnimationNodeIndex, seg: AnimationSegment) {
    let active = player.play(node);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(1.0);
    active.replay();
    active.seek_to(seg.start_secs());
    active.resume();
}

fn main() {
    // Check for a newer release and, if there is one, replace this executable and
    // relaunch before Bevy starts. No-op under `cargo run`. See src/updater.rs.
    updater::bootstrap();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy-trickshot".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .init_state::<AppState>()
        .enable_state_scoped_entities::<AppState>()
        // `ClientNetPlugin` pulls in lightyear's client plugins + the shared
        // protocol; add it before anything that spawns a `Client`.
        .add_plugins(net::ClientNetPlugin)
        .add_plugins((
            ui::UiKitPlugin,
            settings::SettingsPlugin,
            menu::MenuPlugin,
            lobby_ui::LobbyUiPlugin,
            practice::PracticePlugin,
            killcam::KillCamPlugin,
        ))
        .insert_resource(AmbientLight {
            color: SKY_AMBIENT_COLOR,
            brightness: SKY_AMBIENT_LUX,
            ..default()
        })
        .init_resource::<ViewModelPoses>()
        .init_resource::<Ads>()
        .init_resource::<AdsTuning>()
        .init_resource::<LookDelta>()
        .init_resource::<WeaponSwayState>()
        .init_resource::<WeaponSwaySettings>()
        .init_resource::<Weapon>()
        .init_resource::<PendingShot>()
        .init_resource::<Shake>()
        .init_resource::<ShakeSettings>()
        .init_resource::<AnimationSettings>()
        .init_resource::<MuzzleFlashSettings>()
        .init_resource::<MuzzleFlashState>()
        .init_resource::<SmokeSettings>()
        .init_resource::<SmokeEmission>()
        .init_resource::<RockSettings>()
        .init_resource::<DustSettings>()
        .init_resource::<TrickState>()
        .add_event::<GroundImpact>()
        .add_event::<TrickScoredEvent>()
        .add_event::<MatchEndedEvent>()
        .init_resource::<MovementSettings>()
        .init_resource::<Sprinting>()
        .init_resource::<Slide>()
        .init_resource::<SlideSettings>()
        .init_resource::<SceneTuning>()
        // The world, cameras and HUD are built once at startup — spawning the 3D
        // cameras lazily on `OnEnter(InGame)` left the window with a stale/black
        // swapchain, so instead the menu/lobby screens (opaque `bevy_ui`, drawn
        // by the UI camera on top) simply cover the world until a game starts.
        .add_systems(
            Startup,
            (
                setup_world,
                setup_player,
                setup_audio,
                setup_ui_camera,
                setup_crosshair,
                setup_ammo_ui,
                setup_fps_ui,
            ),
        )
        .add_systems(
            OnEnter(AppState::InGame),
            (grab_cursor, start_ambient, reset_slide, reset_trick, reset_weapon),
        )
        .add_systems(OnEnter(AppState::MainMenu), release_cursor)
        .add_systems(Update, hud_visibility)
        // Dev tuning panels — only in-game, and only while debug mode is on.
        .add_systems(
            EguiPrimaryContextPass,
            ads_tuning_ui.run_if(menu::debug_enabled.and(in_state(AppState::InGame))),
        )
        .add_systems(
            Update,
            update_ads.run_if(in_state(AppState::InGame).and(killcam::no_killcam)),
        )
        .add_systems(
            Update,
            (
                // Gameplay input / simulation — frozen while a menu is open or a
                // kill-cam replay is playing.
                (
                    toggle_sprint,
                    crouch_slide,
                    move_player,
                    teleport_home,
                    jump,
                    apply_gravity,
                )
                    .chain()
                    .run_if(menu::game_active.and(killcam::no_killcam)),
                look_around.run_if(menu::game_active.and(killcam::no_killcam)),
                weapon_system.run_if(menu::game_active.and(killcam::no_killcam)),
                // Visuals / HUD — keep running so shake, smoke and the scope
                // settle even while paused.
                apply_ads,
                update_scope,
                fade_crosshair,
                track_trick.after(look_around).run_if(killcam::no_killcam),
                (spawn_score_popup, update_score_popups),
                sky_follow_camera,
                camera_shake.run_if(killcam::no_killcam),
                update_muzzle_flash,
                // After `look_around` so the smoke uses this frame's aim, not
                // the previous frame's — otherwise a fast turn leaves the
                // sprites angled toward where the player just was.
                (emit_smoke.run_if(menu::game_active), update_smoke).after(look_around),
                (spawn_ground_impact, update_impact_particles).after(look_around),
                update_ammo_ui,
                update_fps_ui,
                apply_scene_tuning,
                debug_cursor_toggle,
            )
                .after(update_ads)
                .run_if(in_state(AppState::InGame)),
        )
        // Weapon sway rides on top of the ADS pose, using this frame's turn;
        // the recoil shudder then rides on top of the sway.
        .add_systems(
            Update,
            (weapon_sway, weapon_recoil_shudder)
                .chain()
                .after(look_around)
                .after(apply_ads)
                .run_if(in_state(AppState::InGame)),
        )
        .run();
}

// ---------------------------------------------------------------------------
// Components / resources
// ---------------------------------------------------------------------------

/// Root of the player rig. Carries position and yaw (horizontal look).
#[derive(Component)]
pub(crate) struct Player;

/// Player physics: horizontal velocity (input-driven on the ground, frozen in
/// the air so a jump carries momentum), falling speed, and whether the feet are
/// resting on a surface.
#[derive(Component, Default)]
pub(crate) struct PlayerPhysics {
    horizontal_velocity: Vec3,
    vertical_velocity: f32,
    pub(crate) grounded: bool,
}

/// Panel-adjustable locomotion + gravity tuning.
#[derive(Resource)]
struct MovementSettings {
    /// Horizontal speed while walking (m/s).
    walk_speed: f32,
    /// Horizontal speed while sprinting (m/s).
    sprint_speed: f32,
    /// Downward acceleration magnitude (m/s²).
    gravity: f32,
    /// Upward launch speed when jumping (m/s).
    jump_speed: f32,
}

impl Default for MovementSettings {
    fn default() -> Self {
        Self {
            walk_speed: WALK_SPEED,
            sprint_speed: SPRINT_SPEED,
            gravity: GRAVITY,
            jump_speed: JUMP_SPEED,
        }
    }
}

/// Sprint toggle state (Left Shift flips it).
#[derive(Resource, Default)]
struct Sprinting(bool);

/// What the player's lower body is doing. `Standing` is the normal state;
/// `Crouching` is a slow ducked walk; `Sliding` is a momentum slide that decays
/// to a stop; `Diving` is the airborne half of a dolphin dive; `Prone` is flat
/// on the ground.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Stance {
    #[default]
    Standing,
    Crouching,
    Sliding,
    Diving,
    Prone,
}

/// Live crouch / slide / dive / prone state. `drop` is the current camera Y
/// offset from standing (metres, ≤ 0); `crouch_slide` eases it toward the target
/// for the stance and writes it onto the head, so every stance change is a
/// smooth move rather than a jerk.
#[derive(Resource, Default)]
struct Slide {
    stance: Stance,
    drop: f32,
    /// Horizontal slide velocity (m/s); only meaningful while `Sliding`.
    velocity: Vec3,
    /// Seconds the current slide has run.
    timer: f32,
    /// The one-shot slide-sound entity, kept so a cancel can cut it short.
    sound: Option<Entity>,
    /// Set for the frame a slide/crouch/prone swallowed the jump press, so
    /// `jump` doesn't also launch.
    ate_jump: bool,
    /// Stance a `Prone` toggle returns to — `Standing` or `Crouching`.
    prone_from: Stance,
}

/// Panel-adjustable crouch / slide / dive tuning.
#[derive(Resource)]
struct SlideSettings {
    /// How far the camera drops when fully crouched (m).
    crouch_drop: f32,
    /// Move speed while crouch-walking (m/s) — slower than `walk_speed`.
    crouch_speed: f32,
    /// How fast the camera ducks down entering a crouch / slide (1/s).
    duck_speed: f32,
    /// How fast the camera rises back up (1/s) — kept lower so it's smooth.
    stand_speed: f32,
    /// Slide launch speed when starting from a walk (m/s).
    slide_speed: f32,
    /// Added to the launch speed when the slide starts from a sprint (m/s).
    sprint_bonus: f32,
    /// Deceleration that bleeds a slide off (m/s²).
    friction: f32,
    /// The slide ends and the player stands once it drops below this (m/s).
    min_speed: f32,
    /// Hard cap on slide duration regardless of friction (s).
    max_time: f32,
    /// How far the camera drops when prone (m).
    prone_drop: f32,
    /// Move speed while crawling prone (m/s).
    prone_speed: f32,
    /// Forward launch speed of a dolphin dive (m/s); a faster approach carries in.
    dive_speed: f32,
    /// Upward hop of a dolphin dive (m/s).
    dive_jump: f32,
    /// How fast the camera tucks toward prone during a dive (1/s).
    dive_tuck_speed: f32,
}

impl Default for SlideSettings {
    fn default() -> Self {
        Self {
            crouch_drop: CROUCH_DROP,
            crouch_speed: CROUCH_SPEED,
            duck_speed: SLIDE_DUCK_SPEED,
            stand_speed: SLIDE_STAND_SPEED,
            slide_speed: SLIDE_SPEED,
            sprint_bonus: SLIDE_SPRINT_BONUS,
            friction: SLIDE_FRICTION,
            min_speed: SLIDE_MIN_SPEED,
            max_time: SLIDE_MAX_TIME,
            prone_drop: PRONE_DROP,
            prone_speed: PRONE_SPEED,
            dive_speed: DIVE_SPEED,
            dive_jump: DIVE_JUMP,
            dive_tuck_speed: DIVE_TUCK_SPEED,
        }
    }
}

/// The daytime look, live-tweakable from the debug panel's "Fog & Sky" section
/// and pushed onto the fog / sun / ambient / bloom by `apply_scene_tuning`.
/// Defaults mirror the `SUN_*` / `SKY_*` / `FOG_*` consts.
#[derive(Resource)]
struct SceneTuning {
    fog_visibility_m: f32,
    fog_color: [f32; 3],
    fog_sun_exponent: f32,
    sun_lux: f32,
    sun_color: [f32; 3],
    ambient_color: [f32; 3],
    ambient_lux: f32,
    bloom_intensity: f32,
}

impl Default for SceneTuning {
    fn default() -> Self {
        Self {
            fog_visibility_m: FOG_VISIBILITY_M,
            fog_color: srgb_parts(FOG_COLOR),
            fog_sun_exponent: 100.0,
            sun_lux: SUN_LUX,
            sun_color: srgb_parts(SUN_COLOR),
            ambient_color: srgb_parts(SKY_AMBIENT_COLOR),
            ambient_lux: SKY_AMBIENT_LUX,
            bloom_intensity: 0.09,
        }
    }
}

fn srgb_parts(c: Color) -> [f32; 3] {
    let s = c.to_srgba();
    [s.red, s.green, s.blue]
}

fn color_from_parts(p: [f32; 3]) -> Color {
    Color::srgb(p[0], p[1], p[2])
}

/// Child of `Player`. Carries pitch (vertical look); cameras and the gun hang
/// off of this so movement stays level with the ground.
#[derive(Component)]
pub(crate) struct PlayerHead;

/// The camera that renders the world (layer 0 only).
#[derive(Component)]
pub(crate) struct WorldModelCamera;

/// The camera that renders the first-person gun (layer 1 only, drawn on top).
#[derive(Component)]
pub(crate) struct ViewModelCamera;

/// Set to `Some` by `weapon_system` on the frame the trigger is pulled; consumed
/// by `net::write_input`, which turns it into the tick's fire request.
#[derive(Resource, Default)]
pub(crate) struct PendingShot(pub Option<()>);

/// Parent of the cameras *and* the view model. Its local transform is
/// overwritten each frame by `camera_shake` with the up/down + side/side shake
/// offset (or identity), so the gun and the view shake together.
#[derive(Component)]
pub(crate) struct CameraShake;

/// Child of [`CameraShake`] that carries only the cameras (not the gun).
/// `camera_shake` sets its local Z to the current backward recoil kick, pulling
/// the eye off the scope's rear lens when a shot's fire animation drags the lens
/// toward the face.
#[derive(Component)]
pub(crate) struct CameraRecoil;

/// The HDR sky sphere; recentred on the camera every frame.
#[derive(Component)]
struct SkySphere;

/// The loaded sniper scene root.
#[derive(Component)]
pub(crate) struct ViewModel;

/// On every target-bot visual (practice-local *and* networked avatars), so the
/// kill cam can hide the live bots and show its own frozen snapshot instead.
#[derive(Component)]
pub(crate) struct TargetBotVisual;

/// Handles + node index for the sniper's single animation clip.
#[derive(Component)]
pub(crate) struct ViewModelAnimation {
    graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// Which weapon slot is up. The knife has no model yet, so `Secondary` just
/// means "sniper hidden, hands empty" (plus a small movement-speed bump).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum WeaponSlot {
    #[default]
    Primary,
    Secondary,
}

/// Ammo counts and the animation the weapon is mid-way through, if any. While
/// `busy` is `Some` neither firing nor reloading is accepted.
#[derive(Resource)]
pub(crate) struct Weapon {
    /// Rounds in the current magazine.
    mag: u32,
    /// Rounds not in the magazine.
    reserve: u32,
    pub(crate) busy: Option<WeaponBusy>,
    /// The slot currently equipped (switched the instant the swap key is
    /// pressed; the Hide / Show animation then plays out via `busy`).
    slot: WeaponSlot,
    /// A reload / rechamber cut short by a weapon switch. Replayed from the top —
    /// animation and sound — once the sniper is next drawn.
    interrupted: Option<WeaponBusy>,
}

pub(crate) struct WeaponBusy {
    /// Segments still to play; `remaining[0]` is the one playing now.
    remaining: Vec<AnimationSegment>,
    /// Clip time (seconds) the current segment ends at.
    seg_end: f32,
    /// What to apply once the whole queue has played out.
    on_finish: WeaponFinish,
}

#[derive(Clone, Copy, PartialEq)]
enum WeaponFinish {
    Nothing,
    Reload,
    /// Hide finished — the sniper is now stowed; drop its model.
    Holster,
    /// Show finished — the sniper is back out; resume any interrupted action.
    Draw,
}

/// Tags the one-shot rechamber / reload audio entities so a weapon switch can
/// cut them off (they otherwise self-despawn when the clip ends).
#[derive(Component)]
struct WeaponActionSound;

impl Default for Weapon {
    fn default() -> Self {
        Self {
            mag: MAG_SIZE,
            reserve: 1000, // TEMP: high reserve for reload-sound testing
            busy: None,
            slot: WeaponSlot::Primary,
            interrupted: None,
        }
    }
}

/// The bottom-right ammo readout (`mag / reserve`).
#[derive(Component)]
struct AmmoText;

/// The top-left FPS readout.
#[derive(Component)]
struct FpsText;

/// The dead-centre white dot; `fade_crosshair` fades it out as the player aims in.
#[derive(Component)]
struct CenterDot;

/// The muzzle-flash sprite quad.
#[derive(Component)]
struct MuzzleFlash;

/// Panel-adjustable placement of the muzzle-flash sprite, relative to the camera
/// rig. Pitch and yaw are fixed at 0 (the sprite always faces the camera); roll
/// is randomised per shot.
#[derive(Resource)]
struct MuzzleFlashSettings {
    translation: Vec3,
    size: Vec2,
}

impl Default for MuzzleFlashSettings {
    fn default() -> Self {
        Self {
            translation: Vec3::new(0.15, -0.07, -1.42),
            size: Vec2::new(0.9, 0.8),
        }
    }
}

/// Live state of the flash: `intensity` snaps to 1 on a shot and decays to 0
/// over `MUZZLE_FLASH_TIME`; `roll` is a fresh random angle per shot.
#[derive(Resource, Default)]
pub(crate) struct MuzzleFlashState {
    pub(crate) intensity: f32,
    pub(crate) roll: f32,
    pub(crate) shots: u32,
}

/// Preloaded sounds. Loaded once at startup so playback has no first-use hitch.
#[derive(Resource)]
pub(crate) struct GameSounds {
    pub(crate) shot: Handle<AudioSource>,
    pub(crate) rechamber: Handle<AudioSource>,
    pub(crate) reload: Handle<AudioSource>,
    ambient: Handle<AudioSource>,
    pub(crate) aim_in: Handle<AudioSource>,
    pub(crate) aim_out: Handle<AudioSource>,
    out_of_ammo: Handle<AudioSource>,
    pub(crate) slide: Handle<AudioSource>,
    pub(crate) dive: Handle<AudioSource>,
}

/// Linear volume of the looping nature ambience.
const AMBIENT_VOLUME: f32 = 0.5;

/// The looping ambient-nature bed. `StateScoped(InGame)`, so it starts when the
/// player enters the world (Practice or a game) and stops on the way out.
#[derive(Component)]
struct AmbientAudio;

/// A single drifting, fading smoke sprite. World-space: once spawned it lives in
/// the world, so the player can walk through it.
#[derive(Component)]
pub(crate) struct Smoke {
    velocity: Vec3,
    age: f32,
    /// Seconds to ramp 0 -> `peak_alpha` before the fade-out begins.
    fade_in: f32,
    /// Total time alive (`fade_in` + fade-out); despawns at this age.
    lifetime: f32,
    roll: f32,
    /// Opacity at the top of the envelope (later spawns in a burst peak dimmer).
    peak_alpha: f32,
}

/// Time (seconds) since the last shot started a smoke burst; `None` when not
/// emitting.
#[derive(Resource, Default)]
pub(crate) struct SmokeEmission(pub(crate) Option<f32>);

/// Shared mesh + texture for smoke particles (each particle still gets its own
/// material so it can fade independently).
#[derive(Resource)]
struct SmokeAssets {
    mesh: Handle<Mesh>,
    texture: Handle<Image>,
}

/// Panel-adjustable smoke emitter settings. `spawn_offset` is in camera space;
/// particles are then released into world space at that point.
#[derive(Resource)]
struct SmokeSettings {
    spawn_offset: Vec3,
    scale: f32,
    rise_rate: f32,
    spread: f32,
    /// Seconds each particle takes to ramp up from invisible to its peak opacity.
    fade_in: f32,
    /// Seconds each particle then takes to fade back to nothing.
    fade_time: f32,
    spawn_rate: f32,
    /// How long the burst keeps emitting after a shot.
    duration: f32,
    /// Peak opacity of a particle spawned the instant a shot fires. Spawns later
    /// in the burst peak proportionally dimmer; nothing ever exceeds this.
    max_opacity: f32,
}

impl Default for SmokeSettings {
    fn default() -> Self {
        Self {
            spawn_offset: Vec3::new(0.15, -0.08, -1.05),
            scale: 0.2,
            rise_rate: 0.25,
            spread: 0.1,
            fade_in: 0.3,
            fade_time: 1.3,
            spawn_rate: 30.0,
            duration: 0.7,
            max_opacity: 0.2,
        }
    }
}

/// Server → everyone: a shot hit the ground at this world point. Consumed by
/// `spawn_ground_impact`, which kicks up a short rock + dust burst there.
#[derive(Event)]
pub(crate) struct GroundImpact(pub(crate) Vec3);

/// One rock or dust sprite from a ground impact. World-space, billboarded at the
/// camera; rocks arc under `gravity`, dust drifts and swells with `drag`.
#[derive(Component)]
pub(crate) struct ImpactParticle {
    velocity: Vec3,
    /// Downward acceleration (m/s²). Rocks fall; dust is ~0.
    gravity: f32,
    /// Per-second velocity damping. Dust high (billows then stalls); rocks ~0.
    drag: f32,
    age: f32,
    lifetime: f32,
    /// Seconds to ramp opacity 0 → `peak_alpha` before the fade-out.
    fade_in: f32,
    roll: f32,
    /// Roll spin (rad/s); rocks tumble, dust doesn't.
    spin: f32,
    /// Sprite size (m) at spawn and at end of life — dust grows, rocks hold.
    scale0: f32,
    scale1: f32,
    peak_alpha: f32,
}

/// Shared quad + the two impact textures (`spawn_ground_impact` clones a fresh
/// material per particle so each fades on its own).
#[derive(Resource)]
struct ImpactAssets {
    quad: Handle<Mesh>,
    dust: Handle<Image>,
    rocks: Handle<Image>,
}

/// Panel-adjustable rock-debris burst for a ground hit.
#[derive(Resource)]
struct RockSettings {
    /// Rocks launched per impact.
    count: u32,
    /// Launch speed (m/s), randomised ±45%.
    speed: f32,
    /// Cone half-angle off straight-up (degrees).
    spread_deg: f32,
    /// Downward acceleration (m/s²).
    gravity: f32,
    /// Peak tumble rate (rad/s).
    spin: f32,
    /// Sprite size (m).
    scale: f32,
    /// Seconds a rock lives, randomised.
    lifetime: f32,
}

impl Default for RockSettings {
    fn default() -> Self {
        Self {
            count: 9,
            speed: 6.0,
            spread_deg: 32.0,
            gravity: 20.0,
            spin: 14.0,
            scale: 0.14,
            lifetime: 1.1,
        }
    }
}

/// Panel-adjustable dust puff for a ground hit.
#[derive(Resource)]
struct DustSettings {
    /// Puffs per impact.
    count: u32,
    /// Initial speed off the cone (m/s).
    speed: f32,
    /// Cone half-angle off straight-up (degrees).
    spread_deg: f32,
    /// Extra straight-up drift added to every puff (m/s).
    rise: f32,
    /// Per-second velocity damping.
    drag: f32,
    /// Sprite size at spawn / at end of life (m) — dust swells.
    start_scale: f32,
    end_scale: f32,
    /// Seconds a puff lives.
    lifetime: f32,
    /// Seconds to fade in.
    fade_in: f32,
    /// Peak opacity.
    opacity: f32,
}

impl Default for DustSettings {
    fn default() -> Self {
        Self {
            count: 12,
            speed: 2.4,
            spread_deg: 58.0,
            rise: 0.6,
            drag: 2.6,
            start_scale: 0.25,
            end_scale: 1.15,
            lifetime: 0.85,
            fade_in: 0.04,
            opacity: 0.5,
        }
    }
}

/// Marks the scope's rear (player-facing) lens mesh; it displays the scope
/// render target and fades in with ADS.
#[derive(Component)]
struct ScopeLens;

/// The second camera that renders the magnified world into the scope texture.
#[derive(Component)]
struct ScopeCamera;

/// The `crosshair.png` quad in front of the scope camera; only that camera sees
/// it, so the reticle appears in the scope image and nowhere else.
#[derive(Component)]
struct ScopeReticle;

/// Handle to the image the scope camera renders into and the lens samples.
#[derive(Resource)]
struct ScopeRenderTarget(Handle<Image>);

/// Aim-down-sight amount: 0 at the hip, 1 looking fully through the scope.
#[derive(Resource, Default)]
pub(crate) struct Ads {
    pub(crate) t: f32,
}

/// `Ads::t` at or below this counts as "no scope" for style points.
pub(crate) const NOSCOPE_ADS_MAX: f32 = 0.05;
/// A yaw run of at least this many degrees before a direction reversal is
/// "banked" toward the trick total (roughly a clean 180).
const TRICK_MIN_RUN_DEG: f32 = 135.0;
/// Per-frame yaw change below this (deg) is treated as not turning.
const TRICK_TURN_EPS_DEG: f32 = 0.05;
/// Seconds of not-really-turning that ends the current trick.
const TRICK_IDLE_RESET_SECS: f32 = 0.4;

/// Tracks the player's spin for style points: degrees turned in the current
/// run, plus banked degrees from earlier runs of the same trick (so a 180 one
/// way then a 180 the other still adds up). Reset on every shot fired and
/// whenever the player stops turning for a moment.
#[derive(Resource, Default)]
pub(crate) struct TrickState {
    /// Yaw last frame (radians), for the per-frame delta.
    last_yaw: f32,
    /// Sign of the current run (+1 / -1 / 0).
    dir: f32,
    /// Unsigned degrees turned in the current run.
    run_deg: f32,
    /// Sum of completed runs (each ≥ `TRICK_MIN_RUN_DEG`) this trick.
    banked_deg: f32,
    /// Any part of this trick happened airborne.
    pub(crate) airborne: bool,
    /// Seconds spent below the turn threshold.
    idle: f32,
}

impl TrickState {
    /// Total spin credited if a shot lands right now.
    pub(crate) fn total_deg(&self) -> f32 {
        self.banked_deg + self.run_deg
    }

    /// Start a fresh trick, keeping only the current yaw reference.
    pub(crate) fn reset(&mut self) {
        let yaw = self.last_yaw;
        *self = Self::default();
        self.last_yaw = yaw;
    }
}

/// Camera-shake state. `trauma` (0..1) is bumped on each shot and decays; it
/// only scales offsets that are rebuilt from zero every frame — so the shake
/// (translation *and* the view-punch rotation) always settles back to exactly
/// identity and can never accumulate into the aim. `phase` just advances the
/// oscillation and resets at rest. `recoil` is the current backward (+Z local)
/// camera offset in metres, snapped up on a shot and eased back to zero.
#[derive(Resource, Default)]
struct Shake {
    trauma: f32,
    phase: f32,
    recoil: f32,
}

/// Panel-adjustable camera-shake tuning. The oscillation + view-punch half
/// mirrors the `SHAKE_*` consts; the recoil half is the forward/back kick that
/// keeps the eye behind the scope lens when firing.
#[derive(Resource)]
struct ShakeSettings {
    /// Trauma added per shot (result capped at 1).
    trauma_per_shot: f32,
    /// Trauma lost per second.
    decay: f32,
    /// Oscillation speed.
    frequency: f32,
    /// Peak up/down + side/side camera translation at full trauma (m).
    pos_max: f32,
    /// Peak upward view-punch pitch at full trauma (°). Rotates gun + cameras
    /// together; scaled down while scoped.
    view_punch_deg: f32,
    /// Amplitude of the random yaw/pitch/roll chaos on the view punch (°).
    view_jitter_deg: f32,
    /// Metres the view model shudders back toward the eye at full trauma.
    weapon_kick: f32,
    /// Muzzle-climb rotation applied to the view model at full trauma (°).
    weapon_kick_deg: f32,
    /// Metres the camera snaps *backward* on each shot (gun stays put).
    recoil_kick: f32,
    /// How fast the backward kick eases back to zero (1/s; larger = snappier).
    recoil_return: f32,
}

impl Default for ShakeSettings {
    fn default() -> Self {
        Self {
            trauma_per_shot: SHAKE_ADD,
            decay: SHAKE_DECAY,
            frequency: SHAKE_FREQ,
            pos_max: SHAKE_POS_MAX,
            view_punch_deg: SHAKE_VIEW_PUNCH_DEG,
            view_jitter_deg: SHAKE_VIEW_JITTER_DEG,
            weapon_kick: SHAKE_WEAPON_KICK,
            weapon_kick_deg: SHAKE_WEAPON_KICK_DEG,
            recoil_kick: SHAKE_RECOIL_KICK,
            recoil_return: SHAKE_RECOIL_RETURN,
        }
    }
}

/// Panel-adjustable playback speeds for the view-model animation segments
/// ("Animations" panel section). `1.0` is the clip's authored speed.
#[derive(Resource)]
struct AnimationSettings {
    /// Speed multiplier for the Rechamber segment played after a shot.
    rechamber_speed: f32,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            rechamber_speed: 1.7,
        }
    }
}

/// Dev-only knobs for dialing in the ADS pose (driven by the egui panel).
#[derive(Resource)]
struct AdsTuning {
    /// Pin ADS to fully aimed regardless of the right mouse button, so the pose
    /// can be tuned with the cursor free.
    force_full: bool,
    /// World-camera FOV (degrees) at full ADS. Lower = more zoom.
    fov_deg: f32,
    /// Scope camera FOV (degrees). Lower = more magnification inside the scope.
    scope_fov_deg: f32,
}

impl Default for AdsTuning {
    fn default() -> Self {
        Self {
            force_full: false,
            fov_deg: ADS_FOV_DEG,
            scope_fov_deg: SCOPE_FOV_DEG,
        }
    }
}

/// The yaw / pitch the view actually rotated by this frame (radians), written by
/// [`look_around`] and consumed once by [`weapon_sway`], which zeroes it again so
/// a frame with no mouse input (or a paused game) reads as "no turn".
#[derive(Resource, Default)]
struct LookDelta {
    /// `x` = yaw (positive = turned left), `y` = pitch (positive = looked up).
    applied: Vec2,
}

/// Running weapon-sway offset (radians), a low-passed lag behind the view.
#[derive(Resource, Default)]
struct WeaponSwayState {
    /// `x` = yaw offset, `y` = pitch offset, applied on top of the ADS pose.
    offset: Vec2,
}

/// Panel-adjustable weapon sway: the view model trails the direction you turn,
/// then springs back to centre. Scaled down as you aim in.
#[derive(Resource)]
struct WeaponSwaySettings {
    /// Seconds of lag at the hip: sway angle ≈ turn rate (rad/s) × this.
    hip_strength: f32,
    /// Seconds of lag at full ADS. The live value lerps `hip → ads` by `Ads::t`,
    /// so 50% aimed is exactly halfway between the two.
    ads_strength: f32,
    /// How fast the weapon catches back up to centre (larger = snappier).
    return_speed: f32,
    /// Hard cap on the sway angle in any direction (degrees).
    max_offset_deg: f32,
}

impl Default for WeaponSwaySettings {
    fn default() -> Self {
        Self {
            hip_strength: 1.1,
            ads_strength: 0.12,
            return_speed: 3.5,
            max_offset_deg: 15.0,
        }
    }
}

/// A local pose for the view model (hip or ADS).
#[derive(Clone, Copy)]
struct ViewModelOffset {
    translation: Vec3,
    yaw: f32,
    pitch: f32,
    scale: f32,
}

impl ViewModelOffset {
    fn transform(&self) -> Transform {
        Transform {
            translation: self.translation,
            rotation: Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0),
            scale: Vec3::splat(self.scale),
        }
    }
}

/// Interpolate between two poses and build the resulting transform.
fn lerp_pose(a: &ViewModelOffset, b: &ViewModelOffset, t: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation: Quat::from_euler(
            EulerRot::YXZ,
            a.yaw.lerp(b.yaw, t),
            a.pitch.lerp(b.pitch, t),
            0.0,
        ),
        scale: Vec3::splat(a.scale.lerp(b.scale, t)),
    }
}

/// The two view-model poses ADS blends between.
#[derive(Resource)]
struct ViewModelPoses {
    hip: ViewModelOffset,
    ads: ViewModelOffset,
}

impl Default for ViewModelPoses {
    fn default() -> Self {
        Self {
            hip: ViewModelOffset {
                translation: Vec3::new(0.16, -0.18, -0.4),
                yaw: PI,
                pitch: 0.0,
                scale: 0.01,
            },
            // Gun pulled onto the camera axis so the sight lines up dead centre.
            // Tuned in-game with the ADS panel sliders.
            ads: ViewModelOffset {
                translation: Vec3::new(-0.00008, -0.1721, -0.18),
                yaw: PI,
                pitch: 0.0,
                scale: 0.01,
            },
        }
    }
}

/// Smoothstep easing for the ADS blend.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

fn setup_world(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Ground: a 200 m plane wrapped in a seamless procedural asphalt texture
    // (see `build_ground_texture`), tiled every ~2 m.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(100.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(build_ground_texture())),
            uv_transform: Affine2::from_scale(Vec2::splat(100.0)),
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));

    // The building the player shoots from (spawns / `T`-teleports onto its roof).
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(
            BUILDING_SIZE.x,
            BUILDING_SIZE.y,
            BUILDING_SIZE.z,
        ))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.4, 0.4, 0.43),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_translation(BUILDING_CENTER),
    ));

    // Sky: the equirectangular HDR mapped onto the inside of a big UV sphere
    // that follows the camera (see `sky_follow_camera`). Not a true cubemap
    // skybox, but it needs no offline conversion and reads fine as a backdrop.
    commands.spawn((
        SkySphere,
        Mesh3d(meshes.add(Sphere::new(SKY_RADIUS).mesh().uv(128, 64))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(asset_server.load("skybox/citrus_orchard_puresky_8k.hdr")),
            unlit: true,
            cull_mode: None,
            ..default()
        })),
        // Bevy's UV sphere has its poles on +Z/-Z; rotate so the equirect map's
        // zenith points up and its nadir points down.
        Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
        NotShadowCaster,
    ));

    // Targets in the world are the bots — server-owned in a lobby game, spawned
    // locally by `spawn_practice_bots` in solo Practice.

    // Sun.
    commands.spawn((
        DirectionalLight {
            illuminance: SUN_LUX,
            color: SUN_COLOR,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, -0.9, -PI / 4.0)),
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 20.0,
            maximum_distance: 120.0,
            ..default()
        }
        .build(),
        // Light both the world and the view model.
        RenderLayers::from_layers(&[0, VIEW_MODEL_RENDER_LAYER]),
    ));
}

fn setup_player(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    poses: Res<ViewModelPoses>,
) {
    // Build a one-clip animation graph for the sniper's baked animation.
    let clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("models/sniper.glb"));
    let (graph, index) = AnimationGraph::from_clip(clip);
    let graph = graphs.add(graph);

    // Image the scope camera renders into and the scope lens samples.
    let mut scope_image = Image::new_fill(
        Extent3d {
            width: SCOPE_RT_SIZE,
            height: SCOPE_RT_SIZE,
            ..default()
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    scope_image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let scope_image = images.add(scope_image);
    commands.insert_resource(ScopeRenderTarget(scope_image.clone()));

    // Reticle quad shown only inside the scope image (`crosshair.png`).
    let reticle_mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let reticle_material = materials.add(StandardMaterial {
        base_color_texture: Some(asset_server.load("textures/crosshair.png")),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    // Muzzle-flash quad (`muzzle-flash.png`).
    let muzzle_mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let muzzle_material = materials.add(StandardMaterial {
        base_color_texture: Some(asset_server.load("textures/muzzle_flash.png")),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    // Shared smoke mesh + texture (`emit_smoke` clones these per particle).
    commands.insert_resource(SmokeAssets {
        mesh: meshes.add(Rectangle::new(1.0, 1.0)),
        texture: asset_server.load("textures/smoke.png"),
    });

    // Bullet-impact debris (`spawn_ground_impact` clones a material per particle).
    commands.insert_resource(ImpactAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        dust: asset_server.load("textures/dust.png"),
        rocks: asset_server.load("textures/rocks.png"),
    });

    commands
        .spawn((
            Player,
            PlayerPhysics::default(),
            Transform::from_translation(SPAWN_POS),
            Visibility::default(),
        ))
        .with_children(|player| {
            player
                .spawn((PlayerHead, Transform::IDENTITY, Visibility::default()))
                .with_children(|head| {
                    // Everything that moves with the view goes under `CameraShake`
                    // — a node whose local transform is *replaced* each frame with
                    // the current shake offset (or identity), so shake can never
                    // accumulate and throw off aim.
                    head.spawn((CameraShake, Transform::IDENTITY, Visibility::default()))
                        .with_children(|rig| {
                            // The cameras — but *not* the gun — hang off
                            // `CameraRecoil`, so the per-shot backward kick pulls
                            // the eye off the scope lens without dragging the
                            // sniper back with it.
                            rig.spawn((CameraRecoil, Transform::IDENTITY, Visibility::default()))
                                .with_children(|cams| {
                                    // World-model camera: renders the default layer 0.
                                    cams.spawn((
                                        WorldModelCamera,
                                        Camera3d::default(),
                                        Camera {
                                            hdr: true,
                                            ..default()
                                        },
                                        Projection::from(PerspectiveProjection {
                                            fov: 90.0_f32.to_radians(),
                                            ..default()
                                        }),
                                        // Thin daytime haze so distance reads and the
                                        // sky sphere's edge isn't a hard line.
                                        DistanceFog {
                                            color: FOG_COLOR,
                                            directional_light_color: SUN_COLOR,
                                            directional_light_exponent: 100.0,
                                            falloff: FogFalloff::from_visibility(FOG_VISIBILITY_M),
                                        },
                                        Bloom {
                                            intensity: 0.09,
                                            ..Bloom::NATURAL
                                        },
                                    ));

                                    // Scope camera: renders the world (layer 0) plus
                                    // the reticle quad (layer 2, no view model)
                                    // through a narrow FOV into `scope_image`.
                                    // `order: -1` so it runs before the main camera;
                                    // inactive until ADS.
                                    cams.spawn((
                                        ScopeCamera,
                                        Camera3d::default(),
                                        Camera {
                                            target: scope_image.clone().into(),
                                            order: -1,
                                            is_active: false,
                                            hdr: true,
                                            clear_color: Color::srgb(0.0, 0.0, 0.0).into(),
                                            ..default()
                                        },
                                        Projection::from(PerspectiveProjection {
                                            fov: SCOPE_FOV_DEG.to_radians(),
                                            ..default()
                                        }),
                                        RenderLayers::from_layers(&[0, SCOPE_OVERLAY_LAYER]),
                                    ))
                                    .with_child((
                                        ScopeReticle,
                                        Mesh3d(reticle_mesh),
                                        MeshMaterial3d(reticle_material),
                                        Transform::from_xyz(0.0, 0.0, -RETICLE_DIST),
                                        RenderLayers::layer(SCOPE_OVERLAY_LAYER),
                                    ));

                                    // View-model camera: renders only layer 1 (the
                                    // gun), on top. A tiny near plane lets the weapon
                                    // come right up to the lens without being clipped.
                                    cams.spawn((
                                        ViewModelCamera,
                                        Camera3d::default(),
                                        Camera {
                                            order: 1,
                                            hdr: true,
                                            ..default()
                                        },
                                        Projection::from(PerspectiveProjection {
                                            fov: 70.0_f32.to_radians(),
                                            near: 0.0001,
                                            ..default()
                                        }),
                                        RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                                    ));
                                });

                            // The sniper view model itself.
                            rig.spawn((
                                ViewModel,
                                ViewModelAnimation { graph, index },
                                SceneRoot(asset_server.load(
                                    GltfAssetLabel::Scene(0).from_asset("models/sniper.glb"),
                                )),
                                poses.hip.transform(),
                                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                            ))
                            .observe(start_view_model_animation);

                            // Muzzle flash sprite. Camera-relative (sits under
                            // `CameraShake`), drawn with the gun on layer 1.
                            // Hidden until a shot; `update_muzzle_flash` pops it
                            // on and fades it out, with a random roll per shot.
                            rig.spawn((
                                MuzzleFlash,
                                Mesh3d(muzzle_mesh),
                                MeshMaterial3d(muzzle_material),
                                Transform::default(),
                                Visibility::Hidden,
                                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                                NoFrustumCulling,
                            ));
                        });
                });
        });
}

/// Once the sniper scene has spawned, drop every entity it created onto the
/// view-model render layer and arm the animation player, parked (paused) on the
/// first frame until `L` is pressed.
fn start_view_model_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    view_models: Query<&ViewModelAnimation>,
    mut players: Query<&mut AnimationPlayer>,
    names: Query<&Name>,
    meshes: Query<(), With<Mesh3d>>,
    scope_rt: Res<ScopeRenderTarget>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let root = trigger.target();
    let Ok(anim) = view_models.get(root) else {
        return;
    };

    let mut lens_count = 0;

    for entity in children.iter_descendants(root) {
        commands.entity(entity).insert((
            RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
            // A skinned mesh is frustum-culled against its *rest-pose* AABB, not
            // the animated one. The re-exported glb's arms rest pose sits behind
            // the near plane, so the arms were being culled from the view-model
            // camera while still showing up in the (light-frustum) shadow pass.
            // The view model is glued to the camera anyway, so culling it per
            // frame buys nothing — just switch it off.
            NoFrustumCulling,
        ));

        let lower_name = names
            .get(entity)
            .ok()
            .map(|n| n.as_str().to_ascii_lowercase());
        let name_has = |needle: &str| lower_name.as_deref().is_some_and(|n| n.contains(needle));
        let is_mesh = meshes.contains(entity);

        // The scope's rear lens ("lens_lens_0" in Blender): a reflective glass
        // disc at the hip, carrying the scope render target on `emissive` so
        // `update_scope` can fade the sight picture in while scoping.
        if is_mesh && name_has("lens") {
            commands.entity(entity).insert((
                ScopeLens,
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(LENS_TINT.0, LENS_TINT.1, LENS_TINT.2),
                    emissive_texture: Some(scope_rt.0.clone()),
                    emissive: LinearRgba::BLACK,
                    // The render target samples V-flipped on the lens; undo it.
                    uv_transform: Affine2::from_scale_angle_translation(
                        Vec2::new(1.0, -1.0),
                        0.0,
                        Vec2::new(0.0, 1.0),
                    ),
                    perceptual_roughness: LENS_ROUGHNESS_HIP,
                    metallic: LENS_METALLIC_HIP,
                    reflectance: LENS_REFLECTANCE_HIP,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })),
                Visibility::Hidden,
            ));
            lens_count += 1;
        }

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anim.index);
            active.set_repeat(RepeatAnimation::Never);
            active.seek_to(0.0);
            active.pause();
            commands
                .entity(entity)
                .insert(AnimationGraphHandle(anim.graph.clone()));
        }
    }

    info!("view model ready: tagged {lens_count} scope-lens mesh(es)");
}

pub(crate) fn grab_cursor(window: Single<&mut Window, With<PrimaryWindow>>) {
    set_cursor_grabbed(&mut window.into_inner(), true);
}

/// Free the cursor when leaving the game for the menus.
pub(crate) fn release_cursor(window: Single<&mut Window, With<PrimaryWindow>>) {
    set_cursor_grabbed(&mut window.into_inner(), false);
}

fn setup_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(GameSounds {
        shot: asset_server.load("audio/sniper_shot.wav"),
        rechamber: asset_server.load("audio/rechamber.wav"),
        reload: asset_server.load("audio/reload.wav"),
        ambient: asset_server.load("audio/ambient_nature.wav"),
        aim_in: asset_server.load("audio/aim-in-sound.mp3"),
        aim_out: asset_server.load("audio/aim-out-sound.mp3"),
        out_of_ammo: asset_server.load("audio/out-of-ammo-sound.mp3"),
        slide: asset_server.load("audio/slide-sound.mp3"),
        dive: asset_server.load("audio/dive-sound.mp3"),
    });
}

/// Start the looping outdoor ambience when the player enters the world.
fn start_ambient(mut commands: Commands, sounds: Res<GameSounds>) {
    commands.spawn((
        AmbientAudio,
        StateScoped(AppState::InGame),
        AudioPlayer::new(sounds.ambient.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(AMBIENT_VOLUME)),
    ));
}

/// The one persistent 2D camera: hosts every `bevy_ui` tree — the main menu and
/// lobby screens as well as the in-game HUD. Drawn last (order 2, no clear) so
/// the view-model gun (view-model camera, order 1) can't render over the HUD.
/// Lives for the whole process; menu screens paint their own opaque background.
/// Show the HUD (crosshair / ammo / FPS) only while actually in a game and no
/// menu overlay is up. The world itself is always loaded but hidden behind the
/// opaque lobby UI outside `InGame`.
fn hud_visibility(
    state: Res<State<AppState>>,
    menu: Res<menu::Menu>,
    killcam: Res<killcam::ActiveKillCam>,
    mut hud: Query<&mut Visibility, With<menu::HudElement>>,
) {
    if !(state.is_changed() || menu.is_changed() || killcam.is_changed()) {
        return;
    }
    let show = *state.get() == AppState::InGame && !menu.is_open() && killcam.0.is_none();
    let want = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut hud {
        if *v != want {
            *v = want;
        }
    }
}

fn setup_ui_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::layer(UI_LAYER),
        IsDefaultUiCamera,
    ));
}

/// A small white dot dead-centre for lining the scope up.
fn setup_crosshair(mut commands: Commands) {
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_child((
            CenterDot,
            Node {
                width: Val::Px(5.0),
                height: Val::Px(5.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::WHITE),
            BorderColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            BorderRadius::MAX,
        ));
}

/// Fade the centre dot out as the player aims down the scope — fully gone at
/// full ADS, fully back at the hip — so it never sits over the sight picture.
fn fade_crosshair(
    ads: Res<Ads>,
    dot: Single<(&mut BackgroundColor, &mut BorderColor), With<CenterDot>>,
) {
    let a = 1.0 - ease(ads.t.clamp(0.0, 1.0));
    let (mut bg, mut border) = dot.into_inner();
    bg.0 = Color::srgba(1.0, 1.0, 1.0, a);
    border.0 = Color::srgba(0.0, 0.0, 0.0, 0.6 * a);
}

/// Server told us a shot scored — the shooter's client pops a CoD-style yellow
/// stack. `lines` are `(label, points)`, top to bottom.
#[derive(Event)]
pub(crate) struct TrickScoredEvent {
    pub(crate) lines: Vec<(String, u32)>,
}

/// Server told us the match clock ran out.
#[derive(Event)]
pub(crate) struct MatchEndedEvent {
    pub(crate) winner: String,
    pub(crate) score: u32,
}

/// The score-popup stack (one per scored shot; a fresh one replaces the last).
#[derive(Component)]
struct ScorePopup {
    age: f32,
}

const SCORE_YELLOW: Color = Color::srgb(1.0, 0.82, 0.1);
const SCORE_POPUP_HOLD: f32 = 1.1;
const SCORE_POPUP_TTL: f32 = 2.6;

/// Spawn the yellow `+N  LABEL` stack, centred a little above the crosshair.
fn spawn_score_popup(
    mut events: EventReader<TrickScoredEvent>,
    existing: Query<Entity, With<ScorePopup>>,
    mut commands: Commands,
) {
    // Only the most recent shot matters if several land in one frame.
    let Some(ev) = events.read().last() else {
        return;
    };
    for e in &existing {
        commands.entity(e).despawn();
    }

    commands
        .spawn((
            ScorePopup { age: 0.0 },
            StateScoped(AppState::InGame),
            GlobalZIndex(9),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(33.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(3.0),
                ..default()
            },
        ))
        .with_children(|col| {
            for (label, points) in &ev.lines {
                col.spawn((
                    Text::new(format!("+{points}  {label}")),
                    TextFont {
                        font_size: 25.0,
                        ..default()
                    },
                    TextColor(SCORE_YELLOW),
                ));
            }
        });
}

/// Hold each popup briefly, then fade its lines out and despawn.
fn update_score_popups(
    time: Res<Time>,
    mut popups: Query<(Entity, &mut ScorePopup, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (entity, mut popup, children) in &mut popups {
        popup.age += time.delta_secs();
        if popup.age >= SCORE_POPUP_TTL {
            commands.entity(entity).despawn();
            continue;
        }
        let a = if popup.age < SCORE_POPUP_HOLD {
            1.0
        } else {
            1.0 - (popup.age - SCORE_POPUP_HOLD) / (SCORE_POPUP_TTL - SCORE_POPUP_HOLD)
        };
        for child in children {
            if let Ok(mut tc) = texts.get_mut(*child) {
                tc.0 = SCORE_YELLOW.with_alpha(a.clamp(0.0, 1.0));
            }
        }
    }
}

/// Bottom-right ammo readout: rounds in the mag, then rounds in reserve.
fn setup_ammo_ui(mut commands: Commands) {
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                bottom: Val::Px(18.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_child((
            AmmoText,
            Text::new(""),
            TextFont {
                font_size: 30.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// Top-left frames-per-second readout.
fn setup_fps_ui(mut commands: Commands) {
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(18.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_child((
            FpsText,
            Text::new(""),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// Accumulator for the FPS readout: frames and seconds since the last update.
#[derive(Default)]
struct FpsAccum {
    elapsed: f32,
    frames: u32,
}

/// Refresh the top-left readout twice a second with the average FPS over each
/// 500 ms window.
fn update_fps_ui(
    time: Res<Time>,
    mut acc: Local<FpsAccum>,
    mut text: Single<&mut Text, With<FpsText>>,
) {
    acc.elapsed += time.delta_secs();
    acc.frames += 1;

    if acc.elapsed >= 0.5 {
        let fps = acc.frames as f32 / acc.elapsed;
        let wanted = format!("{fps:.0} fps");
        if text.0 != wanted {
            text.0 = wanted;
        }
        acc.elapsed = 0.0;
        acc.frames = 0;
    }
}

/// lil-gui-style panel for dialing in the ADS pose. Press `Esc` to free the
/// cursor, drag the sliders, `Esc` again to get back into the game.
fn ads_tuning_ui(
    mut contexts: EguiContexts,
    mut poses: ResMut<ViewModelPoses>,
    mut tuning: ResMut<AdsTuning>,
    mut muzzle: ResMut<MuzzleFlashSettings>,
    mut smoke: ResMut<SmokeSettings>,
    mut rocks: ResMut<RockSettings>,
    mut dust: ResMut<DustSettings>,
    mut movement: ResMut<MovementSettings>,
    mut slide_cfg: ResMut<SlideSettings>,
    mut sway: ResMut<WeaponSwaySettings>,
    mut shake_cfg: ResMut<ShakeSettings>,
    mut anim: ResMut<AnimationSettings>,
    mut scene: ResMut<SceneTuning>,
    binds: Res<KeyBindings>,
    shake: Res<Shake>,
    ads: Res<Ads>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("ADS tuning")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .resizable(false)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label(format!(
                "{}: free / lock the cursor",
                binds.cursor_toggle.label()
            ));
            ui.separator();
            ui.checkbox(&mut tuning.force_full, "Force full ADS (ignore RMB)");
            ui.label(format!("ads.t = {:.2}", ads.t));
            ui.add(
                egui::Slider::new(&mut tuning.fov_deg, 3.0f32..=45.0)
                    .text("main ADS FOV°  (lower = more zoom)"),
            );
            ui.add(
                egui::Slider::new(&mut tuning.scope_fov_deg, 1.0f32..=30.0)
                    .text("scope FOV°  (lower = more magnification)"),
            );
            ui.separator();

            let a = &mut poses.ads;
            ui.add(egui::Slider::new(&mut a.translation.x, -0.4f32..=0.4).text("x  (right +)"));
            ui.add(egui::Slider::new(&mut a.translation.y, -0.4f32..=0.4).text("y  (up +)"));
            ui.add(egui::Slider::new(&mut a.translation.z, -0.8f32..=0.0).text("z  (forward -)"));
            ui.add(
                egui::Slider::new(&mut a.yaw, (-PI)..=PI)
                    .text("yaw")
                    .step_by(0.001),
            );
            ui.add(
                egui::Slider::new(&mut a.pitch, -0.6f32..=0.6)
                    .text("pitch")
                    .step_by(0.001),
            );
            ui.add(
                egui::Slider::new(&mut a.scale, 0.001f32..=0.05)
                    .text("scale")
                    .logarithmic(true),
            );

            ui.separator();
            if ui.button("Copy pose to console").clicked() {
                info!(
                    "ads: ViewModelOffset {{ translation: Vec3::new({:.4}, {:.4}, {:.4}), \
                     yaw: {:.4}, pitch: {:.4}, scale: {:.5} }},",
                    a.translation.x, a.translation.y, a.translation.z, a.yaw, a.pitch, a.scale,
                );
            }
            if ui.button("Reset to default").clicked() {
                *a = ViewModelPoses::default().ads;
            }

            ui.separator();
            ui.collapsing("Muzzle flash", |ui| {
                let m = &mut *muzzle;
                ui.label("position");
                ui.add(egui::Slider::new(&mut m.translation.x, -0.6f32..=0.6).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut m.translation.y, -0.6f32..=0.6).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut m.translation.z, -2.0f32..=0.0).text("z  (forward -)"),
                );
                ui.label("size (m)");
                ui.add(egui::Slider::new(&mut m.size.x, 0.01f32..=2.0).text("width"));
                ui.add(egui::Slider::new(&mut m.size.y, 0.01f32..=2.0).text("height"));

                if ui.button("Copy muzzle flash to console").clicked() {
                    info!(
                        "muzzle: translation Vec3::new({:.4}, {:.4}, {:.4}), \
                         size Vec2::new({:.4}, {:.4})",
                        m.translation.x, m.translation.y, m.translation.z, m.size.x, m.size.y,
                    );
                }
                if ui.button("Reset muzzle flash").clicked() {
                    *m = MuzzleFlashSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Smoke", |ui| {
                let sm = &mut *smoke;
                ui.label("spawn offset (from camera)");
                ui.add(
                    egui::Slider::new(&mut sm.spawn_offset.x, -0.8f32..=0.8).text("x  (right +)"),
                );
                ui.add(egui::Slider::new(&mut sm.spawn_offset.y, -0.8f32..=0.8).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut sm.spawn_offset.z, -3.0f32..=0.0).text("z  (forward -)"),
                );
                ui.add(egui::Slider::new(&mut sm.scale, 0.02f32..=3.0).text("scale (m)"));
                ui.add(egui::Slider::new(&mut sm.rise_rate, 0.0f32..=4.0).text("rise rate (m/s)"));
                ui.add(egui::Slider::new(&mut sm.spread, 0.0f32..=2.0).text("spread (m/s)"));
                ui.add(egui::Slider::new(&mut sm.fade_in, 0.0f32..=5.0).text("fade in (s)"));
                ui.add(egui::Slider::new(&mut sm.fade_time, 0.1f32..=10.0).text("fade out (s)"));
                ui.add(
                    egui::Slider::new(&mut sm.spawn_rate, 0.0f32..=150.0).text("spawn rate (/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut sm.duration, 0.05f32..=5.0).text("burst duration (s)"),
                );
                ui.add(egui::Slider::new(&mut sm.max_opacity, 0.0f32..=1.0).text("max opacity"));

                if ui.button("Copy smoke to console").clicked() {
                    info!(
                        "smoke: spawn_offset Vec3::new({:.4}, {:.4}, {:.4}), scale {:.4}, \
                         rise_rate {:.4}, spread {:.4}, fade_in {:.4}, fade_time {:.4}, \
                         spawn_rate {:.4}, duration {:.4}, max_opacity {:.4}",
                        sm.spawn_offset.x,
                        sm.spawn_offset.y,
                        sm.spawn_offset.z,
                        sm.scale,
                        sm.rise_rate,
                        sm.spread,
                        sm.fade_in,
                        sm.fade_time,
                        sm.spawn_rate,
                        sm.duration,
                        sm.max_opacity,
                    );
                }
                if ui.button("Reset smoke").clicked() {
                    *sm = SmokeSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Impact rocks", |ui| {
                let r = &mut *rocks;
                ui.label("debris kicked up where a shot hits the ground");
                ui.add(egui::Slider::new(&mut r.count, 0u32..=40).text("rocks per hit"));
                ui.add(egui::Slider::new(&mut r.speed, 0.0f32..=20.0).text("launch speed (m/s)"));
                ui.add(egui::Slider::new(&mut r.spread_deg, 0.0f32..=90.0).text("cone spread (°)"));
                ui.add(egui::Slider::new(&mut r.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(egui::Slider::new(&mut r.spin, 0.0f32..=40.0).text("tumble (rad/s)"));
                ui.add(egui::Slider::new(&mut r.scale, 0.01f32..=0.5).text("size (m)"));
                ui.add(egui::Slider::new(&mut r.lifetime, 0.1f32..=4.0).text("lifetime (s)"));
                if ui.button("Reset rocks").clicked() {
                    *r = RockSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Impact dust", |ui| {
                let d = &mut *dust;
                ui.label("dust cloud where a shot hits the ground");
                ui.add(egui::Slider::new(&mut d.count, 0u32..=40).text("puffs per hit"));
                ui.add(egui::Slider::new(&mut d.speed, 0.0f32..=12.0).text("launch speed (m/s)"));
                ui.add(egui::Slider::new(&mut d.spread_deg, 0.0f32..=90.0).text("cone spread (°)"));
                ui.add(egui::Slider::new(&mut d.rise, 0.0f32..=4.0).text("extra rise (m/s)"));
                ui.add(egui::Slider::new(&mut d.drag, 0.0f32..=10.0).text("drag (/s)"));
                ui.add(egui::Slider::new(&mut d.start_scale, 0.02f32..=2.0).text("start size (m)"));
                ui.add(egui::Slider::new(&mut d.end_scale, 0.02f32..=4.0).text("end size (m)"));
                ui.add(egui::Slider::new(&mut d.lifetime, 0.1f32..=4.0).text("lifetime (s)"));
                ui.add(egui::Slider::new(&mut d.fade_in, 0.0f32..=1.0).text("fade in (s)"));
                ui.add(egui::Slider::new(&mut d.opacity, 0.0f32..=1.0).text("opacity"));
                if ui.button("Reset dust").clicked() {
                    *d = DustSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Movement", |ui| {
                let m = &mut *movement;
                ui.add(
                    egui::Slider::new(&mut m.walk_speed, 0.0f32..=20.0).text("walk speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.sprint_speed, 0.0f32..=30.0)
                        .text("sprint speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut m.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(
                    egui::Slider::new(&mut m.jump_speed, 0.0f32..=20.0).text("jump strength (m/s)"),
                );
                if ui.button("Reset movement").clicked() {
                    *m = MovementSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Slide", |ui| {
                let s = &mut *slide_cfg;
                ui.label("crouch = key while still; slide = key while moving; jump cancels");
                ui.add(egui::Slider::new(&mut s.crouch_drop, 0.0f32..=1.5).text("crouch drop (m)"));
                ui.add(
                    egui::Slider::new(&mut s.crouch_speed, 0.0f32..=8.0)
                        .text("crouch-walk speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.slide_speed, 0.0f32..=20.0)
                        .text("slide launch speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.sprint_bonus, 0.0f32..=12.0)
                        .text("sprint slide bonus (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.friction, 0.5f32..=25.0).text("slide friction (m/s²)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.min_speed, 0.1f32..=8.0).text("slide end speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut s.max_time, 0.2f32..=4.0).text("slide time cap (s)"));
                ui.add(
                    egui::Slider::new(&mut s.duck_speed, 2.0f32..=30.0).text("duck-down rate (/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.stand_speed, 2.0f32..=30.0).text("stand-up rate (/s)"),
                );
                if ui.button("Reset slide").clicked() {
                    *s = SlideSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Dive & prone", |ui| {
                let s = &mut *slide_cfg;
                ui.label("prone key while still toggles prone; while moving = dolphin dive");
                ui.add(egui::Slider::new(&mut s.prone_drop, 0.2f32..=1.6).text("prone drop (m)"));
                ui.add(
                    egui::Slider::new(&mut s.prone_speed, 0.0f32..=6.0).text("prone crawl (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.dive_speed, 0.0f32..=22.0)
                        .text("dive launch speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.dive_jump, 0.0f32..=12.0).text("dive hop (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.dive_tuck_speed, 2.0f32..=30.0)
                        .text("dive tuck rate (/s)"),
                );
                if ui.button("Reset dive & prone").clicked() {
                    let d = SlideSettings::default();
                    s.prone_drop = d.prone_drop;
                    s.prone_speed = d.prone_speed;
                    s.dive_speed = d.dive_speed;
                    s.dive_jump = d.dive_jump;
                    s.dive_tuck_speed = d.dive_tuck_speed;
                }
            });

            ui.separator();
            ui.collapsing("Weapon sway", |ui| {
                let w = &mut *sway;
                ui.label("the gun lags the way you turn, then catches up");
                ui.add(
                    egui::Slider::new(&mut w.hip_strength, 0.0f32..=2.0)
                        .text("hip strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.ads_strength, 0.0f32..=0.5)
                        .text("ADS strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.return_speed, 1.0f32..=20.0).text("catch-up speed"),
                );
                ui.add(
                    egui::Slider::new(&mut w.max_offset_deg, 0.0f32..=30.0).text("max offset (°)"),
                );
                ui.label(format!(
                    "live strength @ ads.t {:.2} = {:.4}",
                    ads.t,
                    w.hip_strength.lerp(w.ads_strength, ads.t.clamp(0.0, 1.0)),
                ));

                if ui.button("Copy weapon sway to console").clicked() {
                    info!(
                        "weapon sway: hip_strength {:.4}, ads_strength {:.4}, \
                         return_speed {:.4}, max_offset_deg {:.4}",
                        w.hip_strength, w.ads_strength, w.return_speed, w.max_offset_deg,
                    );
                }
                if ui.button("Reset weapon sway").clicked() {
                    *w = WeaponSwaySettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Camera shake", |ui| {
                let c = &mut *shake_cfg;
                ui.label("per-shot kick — up/down + side/side only");
                ui.add(
                    egui::Slider::new(&mut c.trauma_per_shot, 0.0f32..=1.0).text("trauma per shot"),
                );
                ui.add(egui::Slider::new(&mut c.decay, 0.5f32..=12.0).text("trauma decay (/s)"));
                ui.add(egui::Slider::new(&mut c.frequency, 5.0f32..=120.0).text("frequency"));
                ui.add(
                    egui::Slider::new(&mut c.pos_max, 0.0f32..=0.4)
                        .text("up/down + L/R amount (m)"),
                );
                ui.separator();
                ui.label("view punch — rotates gun + cameras together (eased while scoped)");
                ui.add(
                    egui::Slider::new(&mut c.view_punch_deg, 0.0f32..=12.0)
                        .text("view punch up (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.view_jitter_deg, 0.0f32..=6.0)
                        .text("view punch chaos (°)"),
                );
                ui.separator();
                ui.label("weapon shudder — gun kicks back toward the eye, muzzle climbs");
                ui.add(
                    egui::Slider::new(&mut c.weapon_kick, 0.0f32..=0.4).text("weapon kick back (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.weapon_kick_deg, 0.0f32..=15.0)
                        .text("weapon muzzle climb (°)"),
                );
                ui.separator();
                ui.label("front/back: eye punches back off the scope, then returns");
                ui.add(
                    egui::Slider::new(&mut c.recoil_kick, 0.0f32..=2.0).text("backward kick (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.recoil_return, 2.0f32..=60.0)
                        .text("return speed (/s)"),
                );
                ui.label(format!("recoil now: {:.3} m", shake.recoil));

                if ui.button("Copy camera shake to console").clicked() {
                    info!(
                        "camera shake: trauma_per_shot {:.4}, decay {:.4}, frequency {:.4}, \
                         pos_max {:.4}, view_punch_deg {:.4}, view_jitter_deg {:.4}, \
                         weapon_kick {:.4}, weapon_kick_deg {:.4}, recoil_kick {:.4}, \
                         recoil_return {:.4}",
                        c.trauma_per_shot,
                        c.decay,
                        c.frequency,
                        c.pos_max,
                        c.view_punch_deg,
                        c.view_jitter_deg,
                        c.weapon_kick,
                        c.weapon_kick_deg,
                        c.recoil_kick,
                        c.recoil_return,
                    );
                }
                if ui.button("Reset camera shake").clicked() {
                    *c = ShakeSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Animations", |ui| {
                let a = &mut *anim;
                ui.add(
                    egui::Slider::new(&mut a.rechamber_speed, 0.1f32..=4.0)
                        .text("rechamber speed (×)"),
                );
                if ui.button("Reset animations").clicked() {
                    *a = AnimationSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Fog & Sky", |ui| {
                let s = &mut *scene;
                ui.add(
                    egui::Slider::new(&mut s.fog_visibility_m, 20.0f32..=2000.0)
                        .logarithmic(true)
                        .text("fog visibility (m)"),
                );
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut s.fog_color);
                    ui.label("fog colour");
                });
                ui.add(
                    egui::Slider::new(&mut s.fog_sun_exponent, 1.0f32..=100.0)
                        .text("sun-scatter tightness"),
                );
                ui.separator();
                ui.add(egui::Slider::new(&mut s.sun_lux, 0.0f32..=120_000.0).text("sun (lux)"));
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut s.sun_color);
                    ui.label("sun colour");
                });
                ui.add(
                    egui::Slider::new(&mut s.ambient_lux, 0.0f32..=6000.0)
                        .text("sky ambient (lux)"),
                );
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut s.ambient_color);
                    ui.label("ambient colour");
                });
                ui.add(egui::Slider::new(&mut s.bloom_intensity, 0.0f32..=0.5).text("bloom"));
                if ui.button("Reset fog & sky").clicked() {
                    *s = SceneTuning::default();
                }
            });
        });
    Ok(())
}

// ---------------------------------------------------------------------------
// Update systems
// ---------------------------------------------------------------------------

/// Sprint control: the sprint key flips sprint on / off, and sprint also drops
/// on its own the moment the player stops feeding a movement key (so it never
/// "sticks" while standing still). Entering a crouch / slide / dive / prone
/// forces it off from `crouch_slide`; pressing sprint out of a crouch or prone
/// stands the player up with sprint already active.
fn toggle_sprint(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut sprinting: ResMut<Sprinting>,
) {
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }
    let move_held = binds.forward.pressed(&keys, &mouse)
        || binds.back.pressed(&keys, &mouse)
        || binds.left.pressed(&keys, &mouse)
        || binds.right.pressed(&keys, &mouse);

    if binds.sprint.just_pressed(&keys, &mouse) {
        sprinting.0 = !sprinting.0;
    } else if sprinting.0 && !move_held {
        sprinting.0 = false; // stopped moving — sprint drops
    }
}

/// Wipe crouch / slide state so re-entering the world always starts upright.
fn reset_slide(mut slide: ResMut<Slide>, mut head: Single<&mut Transform, With<PlayerHead>>) {
    *slide = Slide::default();
    head.translation.y = 0.0;
}

/// Re-entering the world always starts on the sniper, model shown, animation
/// parked at rest — so quitting mid-swap can't leave the next game weaponless.
fn reset_weapon(
    mut weapon: ResMut<Weapon>,
    mut view_model: Query<(&ViewModelAnimation, &mut Visibility), With<ViewModel>>,
    mut players: Query<&mut AnimationPlayer>,
) {
    *weapon = Weapon::default();
    if let Ok((vm, mut vis)) = view_model.single_mut() {
        *vis = Visibility::Inherited;
        if let Some(mut player) = players.iter_mut().next() {
            if let Some(active) = player.animation_mut(vm.index) {
                active.seek_to(0.0);
                active.pause();
            }
        }
    }
}

/// Crouch / slide / dive / prone state machine (Call-of-Duty style).
///
/// Crouch/slide key (`C` by default):
/// * standing + still → **crouch** (slow ducked walk, no sprint);
/// * standing + moving → **slide** along the travel direction — fast, then
///   friction bleeds it off and the player smoothly stands. A slide from a
///   sprint carries extra speed; you can't slide while already crouched.
/// * jump cancels a slide on the spot / stands you up out of a crouch.
///
/// Prone/dive key (`Left Ctrl` by default):
/// * still → toggle **prone** ⇄ whatever you were (standing or crouched);
/// * standing + moving → **dolphin dive**: a forward hop that lands prone, the
///   dive-sound firing on impact. Can't dive while crouched.
#[allow(clippy::too_many_arguments)]
fn crouch_slide(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    cfg: Res<SlideSettings>,
    sounds: Res<GameSounds>,
    weapon: Res<Weapon>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    mut sprinting: ResMut<Sprinting>,
    mut slide: ResMut<Slide>,
    player: Single<(&Transform, &mut PlayerPhysics), With<Player>>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
    mut commands: Commands,
) {
    let dt = time.delta_secs().max(1.0e-5);
    slide.ate_jump = false;
    // The lighter secondary bumps slide-launch and dive speed the same way it
    // bumps walking (see `move_player`).
    let weapon_mult = if weapon.slot == WeaponSlot::Secondary {
        SECONDARY_MOVE_MULT
    } else {
        1.0
    };

    let locked = window.cursor_options.grab_mode != CursorGrabMode::None;
    let crouch_pressed = locked && binds.crouch.just_pressed(&keys, &mouse);
    let prone_pressed = locked && binds.prone.just_pressed(&keys, &mouse);
    let jump_pressed = locked && binds.jump.just_pressed(&keys, &mouse);
    // `toggle_sprint` (runs first) has already flipped sprint on for this press.
    let sprint_pressed = locked && binds.sprint.just_pressed(&keys, &mouse);

    let (transform, mut physics) = player.into_inner();
    let grounded = physics.grounded;
    let planar = Vec3::new(
        physics.horizontal_velocity.x,
        0.0,
        physics.horizontal_velocity.z,
    );
    let speed_now = planar.length();
    let move_held = binds.forward.pressed(&keys, &mouse)
        || binds.back.pressed(&keys, &mouse)
        || binds.left.pressed(&keys, &mouse)
        || binds.right.pressed(&keys, &mouse);
    let moving = speed_now > 0.5 || move_held;
    // Direction we're travelling, falling back to facing.
    let travel_dir = {
        let d = planar.normalize_or_zero();
        if d == Vec3::ZERO {
            let f = *transform.forward();
            Vec3::new(f.x, 0.0, f.z).normalize_or_zero()
        } else {
            d
        }
    };

    match slide.stance {
        Stance::Standing => {
            if grounded && crouch_pressed {
                if moving {
                    let launch = (cfg.slide_speed
                        + if sprinting.0 { cfg.sprint_bonus } else { 0.0 })
                        * weapon_mult;
                    // A slide never slows you down — momentum carries.
                    slide.velocity = travel_dir * launch.max(speed_now);
                    slide.timer = 0.0;
                    slide.stance = Stance::Sliding;
                    sprinting.0 = false;
                    if let Some(e) = slide.sound.take() {
                        commands.entity(e).try_despawn();
                    }
                    slide.sound = Some(
                        commands
                            .spawn((
                                AudioPlayer::new(sounds.slide.clone()),
                                PlaybackSettings::DESPAWN,
                            ))
                            .id(),
                    );
                    snd.note(killcam::SND_SLIDE);
                } else {
                    slide.stance = Stance::Crouching;
                    sprinting.0 = false;
                }
            } else if grounded && prone_pressed {
                if moving {
                    // Dolphin dive: leap forward, land prone (sound on impact).
                    physics.horizontal_velocity =
                        travel_dir * (cfg.dive_speed * weapon_mult).max(speed_now);
                    physics.vertical_velocity = cfg.dive_jump;
                    physics.grounded = false;
                    slide.stance = Stance::Diving;
                    slide.prone_from = Stance::Standing;
                    sprinting.0 = false;
                } else {
                    slide.stance = Stance::Prone;
                    slide.prone_from = Stance::Standing;
                    sprinting.0 = false;
                }
            }
        }
        Stance::Crouching => {
            if sprint_pressed {
                // Stand up and take off — sprint is already on from `toggle_sprint`.
                slide.stance = Stance::Standing;
            } else if prone_pressed {
                // No diving out of a crouch — just drop prone.
                slide.stance = Stance::Prone;
                slide.prone_from = Stance::Crouching;
                sprinting.0 = false;
            } else if crouch_pressed || jump_pressed {
                slide.stance = Stance::Standing;
                slide.ate_jump = jump_pressed;
                sprinting.0 = false;
            } else {
                sprinting.0 = false;
            }
        }
        Stance::Sliding => {
            sprinting.0 = false;
            slide.timer += dt;
            let spd = (slide.velocity.length() - cfg.friction * dt).max(0.0);
            slide.velocity = slide.velocity.normalize_or_zero() * spd;

            let cancelled = jump_pressed;
            if cancelled
                || spd < cfg.min_speed
                || slide.timer >= cfg.max_time
                || !grounded
            {
                slide.stance = Stance::Standing;
                slide.velocity = Vec3::ZERO;
                if cancelled {
                    slide.ate_jump = true;
                    if let Some(e) = slide.sound.take() {
                        commands.entity(e).try_despawn(); // cut the slide sound short
                    }
                } else {
                    slide.sound = None; // ran its course — let the sound finish
                }
            }
        }
        Stance::Diving => {
            sprinting.0 = false;
            if grounded {
                // Belly hit the ground: settle prone and thump.
                slide.stance = Stance::Prone;
                slide.prone_from = Stance::Standing;
                commands.spawn((
                    AudioPlayer::new(sounds.dive.clone()),
                    PlaybackSettings::DESPAWN,
                ));
                snd.note(killcam::SND_DIVE);
            }
        }
        Stance::Prone => {
            if sprint_pressed {
                // Stand straight up and take off.
                slide.stance = Stance::Standing;
            } else if prone_pressed || jump_pressed || crouch_pressed {
                slide.stance = slide.prone_from;
                slide.ate_jump = jump_pressed;
                sprinting.0 = false;
            } else {
                sprinting.0 = false;
            }
        }
    }

    // Ease the camera toward the target height for the stance and write it onto
    // the head (which carries the cameras + gun), so it's never a jerk.
    let target_drop = match slide.stance {
        Stance::Standing => 0.0,
        Stance::Crouching | Stance::Sliding => -cfg.crouch_drop,
        Stance::Diving | Stance::Prone => -cfg.prone_drop,
    };
    let rate = if target_drop < slide.drop - 1.0e-4 {
        if slide.stance == Stance::Diving {
            cfg.dive_tuck_speed
        } else {
            cfg.duck_speed
        }
    } else {
        cfg.stand_speed
    };
    slide.drop += (target_drop - slide.drop) * (1.0 - (-rate * dt).exp());
    if slide.drop.abs() < 1.0e-4 {
        slide.drop = 0.0;
    }
    head.translation.y = slide.drop;
}

fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    settings: Res<MovementSettings>,
    slide_cfg: Res<SlideSettings>,
    sprinting: Res<Sprinting>,
    slide: Res<Slide>,
    weapon: Res<Weapon>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let (mut transform, mut physics) = player.into_inner();
    // The lighter secondary lets the player move a touch quicker in every stance.
    let weapon_mult = if weapon.slot == WeaponSlot::Secondary {
        SECONDARY_MOVE_MULT
    } else {
        1.0
    };

    // A slide ignores steering entirely — `crouch_slide` owns the velocity and
    // bleeds it off with friction (already scaled for the equipped weapon).
    if slide.stance == Stance::Sliding {
        physics.horizontal_velocity = slide.velocity;
    } else if physics.grounded {
        // Input only steers you while your feet are on something — in the air
        // the velocity from the moment you left the ground carries you.
        let mut direction = Vec3::ZERO;
        let forward = *transform.forward();
        let right = *transform.right();
        if binds.forward.pressed(&keys, &mouse) {
            direction += forward;
        }
        if binds.back.pressed(&keys, &mouse) {
            direction -= forward;
        }
        if binds.right.pressed(&keys, &mouse) {
            direction += right;
        }
        if binds.left.pressed(&keys, &mouse) {
            direction -= right;
        }
        direction.y = 0.0;

        let speed = match slide.stance {
            Stance::Crouching => slide_cfg.crouch_speed,
            Stance::Prone => slide_cfg.prone_speed,
            _ if sprinting.0 => settings.sprint_speed,
            _ => settings.walk_speed,
        };
        physics.horizontal_velocity = direction.normalize_or_zero() * speed * weapon_mult;
    }

    transform.translation += physics.horizontal_velocity * time.delta_secs();
}

/// Snaps the player back onto the roof of the building.
fn teleport_home(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    if binds.teleport_home.just_pressed(&keys, &mouse) {
        let (mut transform, mut physics) = player.into_inner();
        transform.translation = SPAWN_POS;
        physics.horizontal_velocity = Vec3::ZERO;
        physics.vertical_velocity = 0.0;
        physics.grounded = true;
    }
}

/// Launches the player upward when they're standing on something. While
/// crouched or sliding the jump key is spoken for (stand up / slide-cancel — see
/// `crouch_slide`), so this bails on anything but a plain standing jump.
fn jump(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    settings: Res<MovementSettings>,
    slide: Res<Slide>,
    mut physics: Single<&mut PlayerPhysics, With<Player>>,
) {
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }
    if slide.stance != Stance::Standing || slide.ate_jump {
        return;
    }
    if physics.grounded && binds.jump.just_pressed(&keys, &mouse) {
        physics.vertical_velocity = settings.jump_speed;
        physics.grounded = false;
    }
}

/// Pull the player down and stop them on whichever surface is under them — the
/// building roof while over its footprint, otherwise the ground. Walk off the
/// roof edge and there's nothing under the feet, so the player falls.
fn apply_gravity(
    time: Res<Time>,
    settings: Res<MovementSettings>,
    slide: Res<Slide>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let dt = time.delta_secs();
    let (mut transform, mut physics) = player.into_inner();

    physics.vertical_velocity -= settings.gravity * dt;

    let feet_now = transform.translation.y - EYE_HEIGHT;
    let feet_next = feet_now + physics.vertical_velocity * dt;

    let roof = BUILDING_CENTER.y + BUILDING_SIZE.y * 0.5;
    let over_building = transform.translation.x.abs() <= BUILDING_SIZE.x * 0.5
        && (transform.translation.z - BUILDING_CENTER.z).abs() <= BUILDING_SIZE.z * 0.5;
    // Only land on the roof from above/at its level — not when walking through
    // the building's base at ground height.
    let surface = if over_building && feet_now >= roof - GROUND_SNAP {
        roof
    } else {
        0.0
    };

    // A dolphin dive lands on the belly: it also counts as touching down once
    // the tucked camera (`transform.y + slide.drop`, drop ≤ 0) gets within
    // `DIVE_CLEARANCE` of the surface — sooner than the standing feet would.
    let dive_landed = slide.stance == Stance::Diving
        && transform.translation.y + slide.drop + physics.vertical_velocity * dt
            <= surface + DIVE_CLEARANCE;

    if feet_next <= surface || dive_landed {
        transform.translation.y = surface + EYE_HEIGHT;
        physics.vertical_velocity = 0.0;
        physics.grounded = true;
    } else {
        transform.translation.y = feet_next + EYE_HEIGHT;
        physics.grounded = false;
    }
}

fn look_around(
    mouse_motion: Res<AccumulatedMouseMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    ads: Res<Ads>,
    settings: Res<Settings>,
    mut look_delta: ResMut<LookDelta>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    // Ignore look input while the cursor is free.
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }

    let delta = mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    // Base sensitivity × the player's multiplier, slowed further as they zoom in.
    let sens =
        MOUSE_SENSITIVITY * settings.sensitivity * 1.0f32.lerp(ADS_SENSITIVITY_SCALE, ease(ads.t));

    // Yaw on the body...
    let yaw = -delta.x * sens.x;
    player.rotate_y(yaw);

    // ...pitch on the head, clamped so we can't flip over.
    let (_, current_pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
    let new_pitch = (current_pitch - delta.y * sens.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    head.rotation = Quat::from_rotation_x(new_pitch);

    // Hand the actual applied rotation to the sway (pitch measured after the
    // clamp so pinning against the limit doesn't keep feeding it).
    look_delta.applied = Vec2::new(yaw, new_pitch - current_pitch);
}

/// Seed the spin tracker from the player's current facing so entering the world
/// (facing -Z) doesn't register as a giant turn.
fn reset_trick(mut trick: ResMut<TrickState>, player: Single<&Transform, With<Player>>) {
    *trick = TrickState::default();
    trick.last_yaw = player.rotation.to_euler(EulerRot::YXZ).0;
}

/// Accumulate the player's yaw spin for style points (see [`TrickState`]).
fn track_trick(
    time: Res<Time>,
    player: Single<(&Transform, &PlayerPhysics), With<Player>>,
    mut trick: ResMut<TrickState>,
) {
    let (transform, physics) = *player;
    let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
    let mut d = yaw - trick.last_yaw;
    if d > PI {
        d -= 2.0 * PI;
    } else if d < -PI {
        d += 2.0 * PI;
    }
    trick.last_yaw = yaw;

    let deg = d.to_degrees();
    if deg.abs() < TRICK_TURN_EPS_DEG {
        trick.idle += time.delta_secs();
        if trick.idle >= TRICK_IDLE_RESET_SECS {
            trick.reset();
        }
        return;
    }
    trick.idle = 0.0;
    if !physics.grounded {
        trick.airborne = true;
    }

    let s = deg.signum();
    if trick.dir == 0.0 || s == trick.dir {
        trick.dir = s;
        trick.run_deg += deg.abs();
    } else {
        // Direction reversed — bank a clean-enough run, then start a new one.
        if trick.run_deg >= TRICK_MIN_RUN_DEG {
            trick.banked_deg += trick.run_deg;
        }
        trick.dir = s;
        trick.run_deg = deg.abs();
    }
}

/// Ramp `Ads::t` toward 1 while the right mouse button is held, back toward 0
/// otherwise.
#[allow(clippy::too_many_arguments)]
fn update_ads(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    tuning: Res<AdsTuning>,
    sounds: Res<GameSounds>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    mut ads: ResMut<Ads>,
    mut was_aiming: Local<bool>,
    mut commands: Commands,
) {
    if tuning.force_full {
        ads.t = 1.0;
        *was_aiming = true;
        return;
    }

    let aiming =
        window.cursor_options.grab_mode != CursorGrabMode::None && binds.aim.pressed(&keys, &mouse);

    // One-shot cue the instant the player starts / stops aiming.
    if aiming != *was_aiming {
        let (clip, bit) = if aiming {
            (sounds.aim_in.clone(), killcam::SND_AIM_IN)
        } else {
            (sounds.aim_out.clone(), killcam::SND_AIM_OUT)
        };
        commands.spawn((AudioPlayer::new(clip), PlaybackSettings::DESPAWN));
        snd.note(bit);
        *was_aiming = aiming;
    }

    let target = if aiming { 1.0 } else { 0.0 };
    let step = time.delta_secs() / ADS_DURATION;
    ads.t = if ads.t < target {
        (ads.t + step).min(target)
    } else {
        (ads.t - step).max(target)
    };
}

/// Blend the world-camera FOV and the view-model pose between hip and ADS.
fn apply_ads(
    ads: Res<Ads>,
    poses: Res<ViewModelPoses>,
    tuning: Res<AdsTuning>,
    settings: Res<Settings>,
    mut world_projection: Single<&mut Projection, With<WorldModelCamera>>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let e = ease(ads.t);

    if let Projection::Perspective(perspective) = world_projection.as_mut() {
        perspective.fov = settings
            .fov
            .to_radians()
            .lerp(tuning.fov_deg.to_radians(), e);
    }

    **view_model = lerp_pose(&poses.hip, &poses.ads, e);
}

/// Make the weapon trail the direction the player turns and then catch up.
///
/// Runs after [`apply_ads`] has written the base pose and multiplies a small
/// rotation about the view origin onto it. The target angle is the view's
/// angular velocity this frame scaled by `strength` (seconds of lag), negated so
/// the gun lags *behind* the turn; a frame-rate-independent ease pulls the live
/// offset toward it, so releasing the turn (target → 0) lets the gun spring back.
/// `strength` lerps `hip → ads` by `Ads::t`, so half-aimed is exactly half sway.
fn weapon_sway(
    time: Res<Time>,
    tuning: Res<WeaponSwaySettings>,
    ads: Res<Ads>,
    mut look: ResMut<LookDelta>,
    mut state: ResMut<WeaponSwayState>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let dt = time.delta_secs().max(1e-5);
    let applied = look.applied;
    look.applied = Vec2::ZERO; // consumed — a still frame reads as no turn

    let strength = tuning
        .hip_strength
        .lerp(tuning.ads_strength, ads.t.clamp(0.0, 1.0));

    let max = tuning.max_offset_deg.to_radians();
    // `-applied / dt` is the view's angular velocity, opposite the turn.
    let target = (-applied / dt * strength).clamp(Vec2::splat(-max), Vec2::splat(max));

    let k = 1.0 - (-tuning.return_speed * dt).exp();
    state.offset = state.offset.lerp(target, k);

    let sway = Quat::from_euler(EulerRot::YXZ, state.offset.x, state.offset.y, 0.0);
    **view_model = Transform::from_rotation(sway) * **view_model;
}

/// Push `SceneTuning` onto the live fog / sun / ambient / bloom whenever it
/// changes (also once at startup, which just re-applies the consts).
fn apply_scene_tuning(
    scene: Res<SceneTuning>,
    mut ambient: ResMut<AmbientLight>,
    mut sun: Single<&mut DirectionalLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
    mut bloom: Single<&mut Bloom, With<WorldModelCamera>>,
) {
    if !scene.is_changed() {
        return;
    }
    ambient.color = color_from_parts(scene.ambient_color);
    ambient.brightness = scene.ambient_lux;

    sun.illuminance = scene.sun_lux;
    sun.color = color_from_parts(scene.sun_color);

    fog.color = color_from_parts(scene.fog_color);
    fog.directional_light_color = color_from_parts(scene.sun_color);
    fog.directional_light_exponent = scene.fog_sun_exponent;
    fog.falloff = FogFalloff::from_visibility(scene.fog_visibility_m);

    bloom.intensity = scene.bloom_intensity;
}

/// In debug mode, the "Lock / Unlock Cursor" key (rebindable, `L` by default)
/// frees the cursor so egui sliders can be dragged, and locks it again. Also
/// re-locks automatically if debug mode is switched off while the cursor is loose.
fn debug_cursor_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    settings: Res<Settings>,
    menu: Res<menu::Menu>,
    window: Single<&mut Window, With<PrimaryWindow>>,
) {
    if menu.is_open() {
        return; // the Esc menu owns the cursor
    }
    let mut window = window.into_inner();
    let loose = window.cursor_options.grab_mode == CursorGrabMode::None;

    if !settings.debug_mode {
        if loose {
            set_cursor_grabbed(&mut window, true);
        }
        return;
    }
    if binds.cursor_toggle.just_pressed(&keys, &mouse) {
        set_cursor_grabbed(&mut window, loose);
    }
}

/// Drive the render-to-texture scope: switch its camera on only while aiming,
/// keep its magnification in sync, and fade the scope image in on the lens.
fn update_scope(
    ads: Res<Ads>,
    tuning: Res<AdsTuning>,
    scope_camera: Single<(&mut Camera, &mut Projection), With<ScopeCamera>>,
    mut reticle: Single<&mut Transform, With<ScopeReticle>>,
    mut lens: Query<(&MeshMaterial3d<StandardMaterial>, &mut Visibility), With<ScopeLens>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let active = ads.t > SCOPE_SHOW_AT;

    let (mut camera, mut projection) = scope_camera.into_inner();
    camera.is_active = active;
    let scope_fov = tuning.scope_fov_deg.to_radians();
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = scope_fov;
    }

    // Keep the reticle quad exactly filling the scope camera's view (square RT).
    let fill = 2.0 * RETICLE_DIST * (scope_fov * 0.5).tan();
    reticle.scale = Vec3::new(fill, fill, 1.0);

    // The lens is always drawn: a reflective glass disc at the hip, the sight
    // picture while scoped. `e` crossfades between the two looks.
    let e = ease(ads.t);
    let k = 1.0 - e;
    for (material, mut visibility) in &mut lens {
        *visibility = Visibility::Inherited;
        if let Some(material) = materials.get_mut(&material.0) {
            // Sight picture rides on emissive: black at the hip, full while scoped.
            material.emissive = LinearRgba::rgb(e, e, e);
            // Glass tint fades to a black backing so the sight picture stays clean.
            material.base_color = Color::srgba(
                LENS_TINT.0 * k,
                LENS_TINT.1 * k,
                LENS_TINT.2 * k,
                0.9f32.lerp(1.0, e),
            );
            // Dial the mirror sheen out as the player scopes in.
            material.perceptual_roughness = LENS_ROUGHNESS_HIP.lerp(LENS_ROUGHNESS_ADS, e);
            material.metallic = LENS_METALLIC_HIP * k;
            material.reflectance = LENS_REFLECTANCE_HIP.lerp(0.5, e);
        }
    }
}

/// Build a seamless tiling ground texture: layered value noise ramped between a
/// dark and a light tarmac tone, with fine grain plus the odd lighter aggregate
/// fleck, so the ground reads as weathered asphalt instead of a flat grid.
fn build_ground_texture() -> Image {
    const N: u32 = 512;

    // Cheap integer-lattice hash → [0, 1).
    fn hash(x: i32, y: i32) -> f32 {
        let mut h = (x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263)) as u32;
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        h ^= h >> 16;
        h as f32 / u32::MAX as f32
    }

    fn smooth(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }

    // Value noise on a lattice that wraps every `period` cells, so a tile whose
    // width spans a whole number of periods is seamless.
    fn value_noise(x: f32, y: f32, period: i32) -> f32 {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let fx = smooth(x - x0 as f32);
        let fy = smooth(y - y0 as f32);
        let w = |v: i32| v.rem_euclid(period);
        let a = hash(w(x0), w(y0));
        let b = hash(w(x0 + 1), w(y0));
        let c = hash(w(x0), w(y0 + 1));
        let d = hash(w(x0 + 1), w(y0 + 1));
        let top = a + (b - a) * fx;
        let bot = c + (d - c) * fx;
        top + (bot - top) * fy
    }

    // fBm whose octave frequencies all divide the tile, so the sum tiles too.
    fn fbm(u: f32, v: f32) -> f32 {
        let (mut sum, mut amp, mut freq) = (0.0, 0.5, 4.0);
        for _ in 0..5 {
            sum += value_noise(u * freq, v * freq, freq as i32) * amp;
            freq *= 2.0;
            amp *= 0.5;
        }
        sum
    }

    let lo = [24.0f32, 25.0, 28.0]; // wet/shadowed tarmac
    let hi = [70.0f32, 71.0, 75.0]; // sun-bleached tarmac
    let fleck = [118.0f32, 116.0, 120.0]; // exposed aggregate

    let mut data = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let u = x as f32 / N as f32;
            let v = y as f32 / N as f32;

            let n = fbm(u, v).clamp(0.0, 1.0);
            let speck = value_noise(u * 96.0, v * 96.0, 96);
            let fleck_amt = if speck > 0.86 { (speck - 0.86) / 0.14 } else { 0.0 };

            let mut rgb = [0u8; 3];
            for c in 0..3 {
                let base = lo[c] + (hi[c] - lo[c]) * n;
                rgb[c] = (base * (1.0 - fleck_amt) + fleck[c] * fleck_amt).round() as u8;
            }
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..default()
    });
    image
}

/// Recentre the sky sphere on the camera so its edge is never reached (it uses
/// last frame's camera `GlobalTransform`, which is imperceptible at this scale).
fn sky_follow_camera(
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut sky: Single<&mut Transform, With<SkySphere>>,
) {
    sky.translation = camera.translation();
}

/// Lock/unlock the mouse to the window. Called on startup and by the menu when
/// it opens/closes.
pub(crate) fn set_cursor_grabbed(window: &mut Window, grabbed: bool) {
    if grabbed {
        window.cursor_options.grab_mode = CursorGrabMode::Locked;
        window.cursor_options.visible = false;
    } else {
        window.cursor_options.grab_mode = CursorGrabMode::None;
        window.cursor_options.visible = true;
    }
}

/// Fire, reload and weapon-swap (bindings). Firing plays Shoot → Rechamber and
/// spends a round; reload plays the Reload segment and then refills the mag.
/// Swapping to the secondary plays Hide in full then drops the sniper model;
/// swapping back plays Show in full and restarts any reload / rechamber the swap
/// cut short. Nothing new is accepted while an animation is mid-play (except the
/// swap key), so shots are impossible until a reload finishes.
#[allow(clippy::too_many_arguments)]
fn weapon_system(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    (view_model, mut view_model_vis): (
        Single<&ViewModelAnimation>,
        Single<&mut Visibility, With<ViewModel>>,
    ),
    (cam, action_sounds, mut players): (
        Query<&GlobalTransform, With<WorldModelCamera>>,
        Query<Entity, With<WeaponActionSound>>,
        Query<&mut AnimationPlayer>,
    ),
    mut weapon: ResMut<Weapon>,
    mut pending_shot: ResMut<PendingShot>,
    mut shake: ResMut<Shake>,
    mut muzzle: ResMut<MuzzleFlashState>,
    mut smoke: ResMut<SmokeEmission>,
    mut shots: EventWriter<practice::LocalShot>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    (shake_cfg, sounds, anim): (Res<ShakeSettings>, Res<GameSounds>, Res<AnimationSettings>),
    mut commands: Commands,
) {
    let node = view_model.index;
    let Some(mut player) = players.iter_mut().next() else {
        return;
    };
    let locked = window.cursor_options.grab_mode != CursorGrabMode::None;

    // Weapon swap — accepted even mid-action, so it can cut a reload / rechamber
    // short.
    if locked && binds.swap_weapon.just_pressed(&keys, &mouse) {
        match weapon.slot {
            WeaponSlot::Primary => {
                // Stow the sniper. Cancel whatever it was doing: silence the
                // reload / rechamber audio, and remember a reload / rechamber so
                // it can be replayed from the top when the sniper is drawn again.
                if let Some(mut busy) = weapon.busy.take() {
                    for e in &action_sounds {
                        commands.entity(e).try_despawn();
                    }
                    if let Some(seg) = busy.remaining.first().copied() {
                        if seg.name == SEGMENTS[SEG_RECHAMBER].name
                            || seg.name == SEGMENTS[SEG_RELOAD].name
                        {
                            busy.seg_end = seg.end_secs();
                            weapon.interrupted = Some(busy);
                        }
                    }
                }
                weapon.slot = WeaponSlot::Secondary;
                play_segment(&mut player, node, SEGMENTS[SEG_HIDE]);
                weapon.busy = Some(WeaponBusy {
                    remaining: vec![SEGMENTS[SEG_HIDE]],
                    seg_end: SEGMENTS[SEG_HIDE].end_secs(),
                    on_finish: WeaponFinish::Holster,
                });
            }
            WeaponSlot::Secondary => {
                // Draw the sniper back: model on, play Show in full; the
                // interrupted action (if any) restarts when Show finishes.
                weapon.slot = WeaponSlot::Primary;
                weapon.busy = None;
                **view_model_vis = Visibility::Inherited;
                play_segment(&mut player, node, SEGMENTS[SEG_SHOW]);
                weapon.busy = Some(WeaponBusy {
                    remaining: vec![SEGMENTS[SEG_SHOW]],
                    seg_end: SEGMENTS[SEG_SHOW].end_secs(),
                    on_finish: WeaponFinish::Draw,
                });
            }
        }
        return;
    }

    // Advance an in-progress action.
    if weapon.busy.is_some() {
        let next_or_finish = {
            let busy = weapon.busy.as_mut().unwrap();
            let done = player
                .animation(node)
                .is_none_or(|a| a.is_finished() || a.seek_time() >= busy.seg_end);
            if !done {
                return;
            }
            busy.remaining.remove(0);
            match busy.remaining.first().copied() {
                Some(next) => {
                    busy.seg_end = next.end_secs();
                    Ok(next)
                }
                None => Err(busy.on_finish),
            }
        };

        match next_or_finish {
            Ok(next) => {
                play_segment(&mut player, node, next);
                if next.name == SEGMENTS[SEG_RECHAMBER].name {
                    commands.spawn((
                        AudioPlayer::new(sounds.rechamber.clone()),
                        PlaybackSettings::DESPAWN,
                        WeaponActionSound,
                    ));
                    snd.note(killcam::SND_RECHAMBER);
                    if let Some(active) = player.animation_mut(node) {
                        active.set_speed(anim.rechamber_speed);
                    }
                }
            }
            Err(on_finish) => {
                // Whole queue played out: park at the rest pose, apply effect.
                if let Some(active) = player.animation_mut(node) {
                    active.seek_to(0.0);
                    active.pause();
                }
                weapon.busy = None;
                match on_finish {
                    WeaponFinish::Nothing => {}
                    WeaponFinish::Reload => {
                        let moved = (MAG_SIZE - weapon.mag).min(weapon.reserve);
                        weapon.mag += moved;
                        weapon.reserve -= moved;
                    }
                    WeaponFinish::Holster => {
                        // Sniper fully hidden — hands are now empty.
                        **view_model_vis = Visibility::Hidden;
                    }
                    WeaponFinish::Draw => {
                        // Sniper back out: restart whatever the swap interrupted,
                        // animation and sound from the top.
                        if let Some(resumed) = weapon.interrupted.take() {
                            let seg = resumed.remaining[0];
                            play_segment(&mut player, node, seg);
                            if seg.name == SEGMENTS[SEG_RECHAMBER].name {
                                commands.spawn((
                                    AudioPlayer::new(sounds.rechamber.clone()),
                                    PlaybackSettings::DESPAWN,
                                    WeaponActionSound,
                                ));
                                snd.note(killcam::SND_RECHAMBER);
                                if let Some(active) = player.animation_mut(node) {
                                    active.set_speed(anim.rechamber_speed);
                                }
                            } else if seg.name == SEGMENTS[SEG_RELOAD].name {
                                commands.spawn((
                                    AudioPlayer::new(sounds.reload.clone()),
                                    PlaybackSettings::DESPAWN,
                                    WeaponActionSound,
                                ));
                                snd.note(killcam::SND_RELOAD);
                            }
                            weapon.busy = Some(resumed);
                        }
                    }
                }
            }
        }
        return;
    }

    // Idle: only take input while the cursor is captured (i.e. in-game) and the
    // sniper is the equipped slot (the secondary has no actions yet).
    if !locked || weapon.slot != WeaponSlot::Primary {
        return;
    }

    if binds.fire.just_pressed(&keys, &mouse) && weapon.mag > 0 {
        weapon.mag -= 1;
        pending_shot.0 = Some(()); // net::write_input turns this into a fire request
        shake.trauma = (shake.trauma + shake_cfg.trauma_per_shot).min(1.0);
        shake.recoil = shake_cfg.recoil_kick;
        muzzle.shots = muzzle.shots.wrapping_add(1);
        muzzle.roll = rand_roll(muzzle.shots);
        muzzle.intensity = 1.0;
        smoke.0 = Some(0.0);
        commands.spawn((
            AudioPlayer::new(sounds.shot.clone()),
            PlaybackSettings::DESPAWN,
        ));
        snd.note(killcam::SND_SHOT);
        // Hand the shot ray to `practice::resolve_local_shot`: it kicks up the
        // ground dust locally (instant, and the only path in solo Practice) and,
        // in Practice, resolves the hit + scoring against the offline bots. In a
        // real game the server also broadcasts this shot; `net::receive_shots`
        // drops the echo for our own peer so nothing double-spawns.
        if let Ok(cam) = cam.single() {
            shots.write(practice::LocalShot {
                origin: cam.translation(),
                dir: cam.forward().as_vec3(),
            });
        }
        play_segment(&mut player, node, SEGMENTS[SEG_SHOOT]);
        weapon.busy = Some(WeaponBusy {
            remaining: vec![SEGMENTS[SEG_SHOOT], SEGMENTS[SEG_RECHAMBER]],
            seg_end: SEGMENTS[SEG_SHOOT].end_secs(),
            on_finish: WeaponFinish::Nothing,
        });
    } else if binds.fire.just_pressed(&keys, &mouse) {
        // Trigger pulled on an empty mag — click, no bang.
        commands.spawn((
            AudioPlayer::new(sounds.out_of_ammo.clone()),
            PlaybackSettings::DESPAWN,
        ));
    } else if binds.reload.just_pressed(&keys, &mouse) && weapon.reserve > 0 {
        // TEMP: reload allowed even with a full mag, for reload-sound testing
        commands.spawn((
            AudioPlayer::new(sounds.reload.clone()),
            PlaybackSettings::DESPAWN,
            WeaponActionSound,
        ));
        snd.note(killcam::SND_RELOAD);
        play_segment(&mut player, node, SEGMENTS[SEG_RELOAD]);
        weapon.busy = Some(WeaponBusy {
            remaining: vec![SEGMENTS[SEG_RELOAD]],
            seg_end: SEGMENTS[SEG_RELOAD].end_secs(),
            on_finish: WeaponFinish::Reload,
        });
    }
}

/// Decay the muzzle flash and push its state onto the sprite: full alpha the
/// frame a shot fires, then a quick fade; a fresh random roll each shot.
fn update_muzzle_flash(
    time: Res<Time>,
    settings: Res<MuzzleFlashSettings>,
    mut state: ResMut<MuzzleFlashState>,
    flash: Single<
        (
            &mut Transform,
            &MeshMaterial3d<StandardMaterial>,
            &mut Visibility,
        ),
        With<MuzzleFlash>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    state.intensity = (state.intensity - time.delta_secs() / MUZZLE_FLASH_TIME).max(0.0);

    let (mut transform, material, mut visibility) = flash.into_inner();

    *visibility = if state.intensity > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };

    *transform = Transform {
        translation: settings.translation,
        rotation: Quat::from_rotation_z(state.roll),
        scale: Vec3::new(
            settings.size.x.max(1.0e-4),
            settings.size.y.max(1.0e-4),
            1.0,
        ),
    };

    if let Some(material) = materials.get_mut(&material.0) {
        material.base_color = Color::srgba(1.0, 1.0, 1.0, state.intensity);
    }
}

/// Release smoke from the barrel for `SmokeSettings::duration` seconds after a
/// shot. Particles spawned later in the burst start dimmer, ramping from
/// `max_opacity` (right after the shot) down to 0 (end of the burst).
fn emit_smoke(
    time: Res<Time>,
    settings: Res<SmokeSettings>,
    assets: Res<SmokeAssets>,
    player: Single<&Transform, With<Player>>,
    head: Single<&Transform, With<PlayerHead>>,
    existing: Query<(), With<Smoke>>,
    mut emission: ResMut<SmokeEmission>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut acc: Local<f32>,
    mut seq: Local<u32>,
) {
    // Advance / end the current burst.
    let Some(mut since_shot) = emission.0 else {
        *acc = 0.0;
        return;
    };
    since_shot += time.delta_secs();
    if since_shot >= settings.duration || settings.spawn_rate <= 0.0 {
        emission.0 = None;
        *acc = 0.0;
        return;
    }
    emission.0 = Some(since_shot);

    // Peak opacity for a particle spawned right now (dimmer later in the burst).
    let burst_progress = (since_shot / settings.duration.max(1.0e-4)).clamp(0.0, 1.0);
    let peak_alpha = settings.max_opacity * (1.0 - burst_progress);
    if peak_alpha < 0.02 {
        return;
    }
    let fade_in = settings.fade_in.max(0.0);
    let lifetime = (fade_in + settings.fade_time.max(0.05)).max(0.05);

    let interval = 1.0 / settings.spawn_rate;
    *acc = (*acc + time.delta_secs()).min(interval * SMOKE_MAX_PER_FRAME);

    // Camera pose from the live transforms (Player = eye position, its child
    // Head carries pitch) so a fast turn while firing doesn't lag the origin.
    let cam_rotation = player.rotation * head.rotation;
    let origin = player.translation + cam_rotation * settings.spawn_offset;
    let mut budget = SMOKE_MAX.saturating_sub(existing.iter().count());

    while *acc >= interval {
        *acc -= interval;
        if budget == 0 {
            continue;
        }
        budget -= 1;
        *seq = seq.wrapping_add(1);
        let s = *seq;

        let angle = rand_roll(s.wrapping_mul(3));
        let speed = rand01(s.wrapping_mul(5)) * settings.spread;
        let velocity = Vec3::new(angle.cos() * speed, settings.rise_rate, angle.sin() * speed);

        let material = materials.add(StandardMaterial {
            base_color_texture: Some(assets.texture.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        });

        commands.spawn((
            Smoke {
                velocity,
                age: 0.0,
                fade_in,
                lifetime,
                roll: rand_roll(s.wrapping_mul(7)),
                peak_alpha,
            },
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(origin).with_scale(Vec3::splat(settings.scale.max(1.0e-4))),
            NoFrustumCulling,
        ));
    }
}

/// Drift each smoke particle, keep it facing the camera, fade it, and despawn
/// it (freeing its material) when its lifetime is up.
fn update_smoke(
    time: Res<Time>,
    settings: Res<SmokeSettings>,
    player: Single<&Transform, (With<Player>, Without<Smoke>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<Smoke>)>,
    mut particles: Query<
        (
            Entity,
            &mut Transform,
            &mut Smoke,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<Smoke>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    // Live camera pose (Player = eye, its child carries pitch) — not the
    // previous frame's propagated `GlobalTransform`.
    let cam_pos = player.translation;
    let cam_up = (player.rotation * head.rotation) * Vec3::Y;
    let cam_right_fallback = (player.rotation * head.rotation) * Vec3::X;
    let scale = Vec3::splat(settings.scale.max(1.0e-4));

    for (entity, mut transform, mut particle, material) in &mut particles {
        particle.age += dt;
        if particle.age >= particle.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }

        transform.translation += particle.velocity * dt;

        // Spherical billboard: the quad's front (+Z) points straight at the
        // camera position, its up follows the camera up, so every puff always
        // shows the same head-on face no matter which way the player turns.
        let to_cam = cam_pos - transform.translation;
        if to_cam.length_squared() > 1.0e-6 {
            let normal = to_cam.normalize();
            let mut right = cam_up.cross(normal);
            if right.length_squared() < 1.0e-6 {
                right = cam_right_fallback;
            }
            let right = right.normalize();
            let up = normal.cross(right);
            let facing = Quat::from_mat3(&Mat3::from_cols(right, up, normal));
            transform.rotation = facing * Quat::from_rotation_z(particle.roll);
        }
        transform.scale = scale;

        // Envelope: ramp 0 -> 1 over `fade_in`, then 1 -> 0 over the rest.
        let envelope = if particle.age < particle.fade_in {
            particle.age / particle.fade_in.max(1.0e-4)
        } else {
            let fade_out = (particle.lifetime - particle.fade_in).max(1.0e-4);
            1.0 - (particle.age - particle.fade_in) / fade_out
        };
        let alpha = particle.peak_alpha * envelope.clamp(0.0, 1.0);
        if let Some(material) = materials.get_mut(&material.0) {
            material.base_color = Color::srgba(1.0, 1.0, 1.0, alpha);
        }
    }
}

/// A unit vector inside a cone of half-angle `half` around `axis`, chosen
/// deterministically from `seed` (uniform over the cap).
fn cone_dir(axis: Vec3, half: f32, seed: u32) -> Vec3 {
    let a = rand01(seed.wrapping_mul(3)) * (PI * 2.0);
    let z = 1.0 - rand01(seed.wrapping_mul(7)) * (1.0 - half.cos());
    let r = (1.0 - z * z).max(0.0).sqrt();
    let local = Vec3::new(r * a.cos(), z, r * a.sin());
    if axis.abs_diff_eq(Vec3::Y, 1.0e-4) {
        local
    } else {
        Quat::from_rotation_arc(Vec3::Y, axis.normalize_or_zero()) * local
    }
}

/// Fresh unlit blended material for one impact sprite.
fn impact_material(texture: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(texture),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// Kick up a short rock + dust burst at every ground impact the server reported
/// this frame. World-space, so it's seen from every angle and by every client.
fn spawn_ground_impact(
    mut events: EventReader<GroundImpact>,
    assets: Res<ImpactAssets>,
    rocks: Res<RockSettings>,
    dust: Res<DustSettings>,
    existing: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let mut budget = IMPACT_MAX.saturating_sub(existing.iter().count());

    for ev in events.read() {
        // Nudge just above the surface so the sprites don't z-fight the ground.
        let at = ev.0 + Vec3::Y * 0.02;
        *seq = seq.wrapping_add(1);

        for i in 0..rocks.count {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let s = seq
                .wrapping_mul(2_654_435_761)
                .wrapping_add(i.wrapping_mul(40_503))
                .wrapping_add(0x11);
            let dir = cone_dir(Vec3::Y, rocks.spread_deg.to_radians(), s);
            let speed = rocks.speed * (0.55 + 0.45 * rand01(s ^ 0x9e37));
            // `rocks.png` is a sheet of ~25 rocks; show one 1/ROCK_COLS × 1/ROCK_ROWS
            // cell of it per particle so each sprite is a single rock, not the pile.
            let col = (rand01(s ^ 0x3) * ROCK_COLS as f32) as u32 % ROCK_COLS;
            let row = (rand01(s ^ 0x5) * ROCK_ROWS as f32) as u32 % ROCK_ROWS;
            let mut material = impact_material(assets.rocks.clone());
            material.uv_transform = Affine2::from_scale_angle_translation(
                Vec2::new(1.0 / ROCK_COLS as f32, 1.0 / ROCK_ROWS as f32),
                0.0,
                Vec2::new(col as f32 / ROCK_COLS as f32, row as f32 / ROCK_ROWS as f32),
            );
            commands.spawn((
                StateScoped(AppState::InGame),
                ImpactParticle {
                    velocity: dir * speed,
                    gravity: rocks.gravity,
                    drag: 0.0,
                    age: 0.0,
                    lifetime: rocks.lifetime.max(0.1) * (0.7 + 0.6 * rand01(s ^ 0x1234)),
                    fade_in: 0.0,
                    roll: rand_roll(s ^ 0x77),
                    spin: (rand01(s ^ 0xab) * 2.0 - 1.0) * rocks.spin,
                    scale0: rocks.scale,
                    scale1: rocks.scale,
                    peak_alpha: 1.0,
                },
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(material)),
                Transform::from_translation(at).with_scale(Vec3::splat(rocks.scale.max(1.0e-4))),
                NoFrustumCulling,
            ));
        }

        for i in 0..dust.count {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let s = seq
                .wrapping_mul(40_503)
                .wrapping_add(i.wrapping_mul(2_654_435_761))
                .wrapping_add(0xd057);
            let dir = cone_dir(Vec3::Y, dust.spread_deg.to_radians(), s);
            let speed = dust.speed * (0.5 + 0.5 * rand01(s ^ 0x55));
            commands.spawn((
                StateScoped(AppState::InGame),
                ImpactParticle {
                    velocity: dir * speed + Vec3::Y * dust.rise,
                    gravity: 0.0,
                    drag: dust.drag,
                    age: 0.0,
                    lifetime: dust.lifetime.max(0.1) * (0.75 + 0.5 * rand01(s ^ 0x9f)),
                    fade_in: dust.fade_in.max(0.0),
                    roll: rand_roll(s ^ 0x21),
                    spin: 0.0,
                    scale0: dust.start_scale,
                    scale1: dust.end_scale,
                    peak_alpha: dust.opacity,
                },
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(impact_material(assets.dust.clone()))),
                Transform::from_translation(at)
                    .with_scale(Vec3::splat(dust.start_scale.max(1.0e-4))),
                NoFrustumCulling,
            ));
        }
    }
}

/// Integrate every live impact particle: gravity + drag, camera billboard with
/// its own roll/spin, scale ramp, opacity envelope, then despawn (freeing the
/// material) at end of life.
fn update_impact_particles(
    time: Res<Time>,
    player: Single<&Transform, (With<Player>, Without<ImpactParticle>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<ImpactParticle>)>,
    mut particles: Query<(
        Entity,
        &mut Transform,
        &mut ImpactParticle,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cam_pos = player.translation;
    let cam_rot = player.rotation * head.rotation;
    let cam_up = cam_rot * Vec3::Y;
    let cam_right_fallback = cam_rot * Vec3::X;

    for (entity, mut transform, mut p, material) in &mut particles {
        p.age += dt;
        if p.age >= p.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }

        let (gravity, drag) = (p.gravity, p.drag);
        p.velocity.y -= gravity * dt;
        p.velocity *= (1.0 - drag * dt).max(0.0);
        transform.translation += p.velocity * dt;

        let f = (p.age / p.lifetime.max(1.0e-4)).clamp(0.0, 1.0);
        let scale = p.scale0.lerp(p.scale1, f).max(1.0e-4);

        let to_cam = cam_pos - transform.translation;
        let roll = p.roll + p.spin * p.age;
        if to_cam.length_squared() > 1.0e-6 {
            let normal = to_cam.normalize();
            let mut right = cam_up.cross(normal);
            if right.length_squared() < 1.0e-6 {
                right = cam_right_fallback;
            }
            let right = right.normalize();
            let up = normal.cross(right);
            let facing = Quat::from_mat3(&Mat3::from_cols(right, up, normal));
            transform.rotation = facing * Quat::from_rotation_z(roll);
        }
        transform.scale = Vec3::splat(scale);

        let envelope = if p.age < p.fade_in {
            p.age / p.fade_in.max(1.0e-4)
        } else {
            let fade_out = (p.lifetime - p.fade_in).max(1.0e-4);
            1.0 - (p.age - p.fade_in) / fade_out
        };
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::srgba(1.0, 1.0, 1.0, p.peak_alpha * envelope.clamp(0.0, 1.0));
        }
    }
}

/// Rebuild the shake nodes' local transforms each frame from the current state,
/// always from zero, so they settle back to exactly identity and never drift aim.
///
/// * `CameraShake` (gun + cameras): up / down + side / side positional jitter
///   *and* the view punch — a directional pitch-up plus rotational chaos, both
///   scaled by `trauma` / `trauma²` and eased down while scoped. Rotating this
///   node turns the gun and the cameras as one, so the gun stays screen-locked
///   while the world swings.
/// * `CameraRecoil` (cameras only): local +Z (straight back along the view axis)
///   set to the current recoil kick, which snaps up on a shot and eases home.
fn camera_shake(
    time: Res<Time>,
    cfg: Res<ShakeSettings>,
    ads: Res<Ads>,
    mut shake: ResMut<Shake>,
    mut rig: Single<&mut Transform, (With<CameraShake>, Without<CameraRecoil>)>,
    mut recoil_node: Single<&mut Transform, (With<CameraRecoil>, Without<CameraShake>)>,
) {
    let dt = time.delta_secs();

    // --- forward / back recoil: snap back on a shot, ease home fast ---------
    if shake.recoil > 0.0 {
        shake.recoil *= (-cfg.recoil_return * dt).exp();
        if shake.recoil < 1.0e-5 {
            shake.recoil = 0.0;
        }
    }
    let want_recoil = Transform::from_xyz(0.0, 0.0, shake.recoil); // +Z = backward
    if **recoil_node != want_recoil {
        **recoil_node = want_recoil;
    }

    // --- up / down + side / side oscillation -------------------------------
    shake.trauma = (shake.trauma - cfg.decay * dt).max(0.0);

    if shake.trauma <= 0.0 {
        shake.phase = 0.0;
        if **rig != Transform::IDENTITY {
            **rig = Transform::IDENTITY;
        }
        return;
    }

    shake.phase += dt * cfg.frequency;
    let s = shake.phase;
    let amt = shake.trauma * shake.trauma;

    // Ease the rotational punch off as the player scopes in, so ADS stays
    // controllable while the hip still kicks hard.
    let ads_scale = 1.0 - 0.7 * ads.t.clamp(0.0, 1.0);

    // Positional jitter: up / down + side / side, no Z.
    let translation = Vec3::new(
        (s * 1.53 + 0.4).sin() * cfg.pos_max * amt,
        (s * 1.19 + 3.3).sin() * cfg.pos_max * amt,
        0.0,
    );

    // View punch: a directional pitch-up that recovers with `trauma`, plus
    // rotational chaos (`trauma²`) on top. +X rotation looks up.
    let punch = shake.trauma * cfg.view_punch_deg.to_radians() * ads_scale;
    let jitter = cfg.view_jitter_deg.to_radians() * amt * ads_scale;
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        (s * 0.91 + 1.7).sin() * jitter,       // yaw
        punch + (s * 0.63).sin() * jitter,     // pitch (up)
        (s * 1.27 + 2.1).sin() * jitter * 0.6, // roll
    );

    **rig = Transform {
        translation,
        rotation,
        scale: Vec3::ONE,
    };
}

/// A short, sharp shudder layered onto the view model on top of the sway pose:
/// the gun punches back toward the eye and the muzzle climbs, then settles as
/// `Shake::trauma` decays. Separate from the view punch in [`camera_shake`] —
/// this moves the gun *relative* to the camera, the way a CoD weapon recoils,
/// and never touches aim. Pre-multiplied in camera space (+Z toward the eye,
/// +Y up), so it is independent of the view model's own orientation.
fn weapon_recoil_shudder(
    cfg: Res<ShakeSettings>,
    shake: Res<Shake>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    if shake.trauma <= 0.0 {
        return;
    }
    let amt = shake.trauma * shake.trauma;
    let s = shake.phase;

    let back = amt * cfg.weapon_kick;
    let rise = amt * cfg.weapon_kick * 0.5;
    let wobble = (s * 0.8).sin() * cfg.weapon_kick * 0.25 * amt;

    let kick = Transform {
        translation: Vec3::new(wobble, rise, back),
        rotation: Quat::from_rotation_x(shake.trauma * cfg.weapon_kick_deg.to_radians()),
        scale: Vec3::ONE,
    };
    **view_model = kick * **view_model;
}

/// Keep the bottom-right readout in sync with the ammo counts.
fn update_ammo_ui(weapon: Res<Weapon>, mut text: Single<&mut Text, With<AmmoText>>) {
    let wanted = format!("{} / {}", weapon.mag, weapon.reserve);
    if text.0 != wanted {
        text.0 = wanted;
    }
}
