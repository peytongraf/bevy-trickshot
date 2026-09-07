//! bevy-trickshot — first-person prototype.
//!
//! Loads `assets/sniper.glb` and parents it to the player camera as a view
//! model. The glTF ships a single baked clip (`allanims`) that packs every
//! action back to back; `SEGMENTS` slices it into the individual animations by
//! frame range.
//!
//! Controls (all rebindable — see `keybinds.rs`; defaults shown):
//!   * `W` / `A` / `S` / `D` — move
//!   * `Left Shift`          — toggle sprint
//!   * `B`                   — jump
//!   * `T`                   — teleport back onto the building
//!   * mouse                 — look around
//!   * right mouse (hold)    — aim down sight
//!   * left mouse            — fire
//!   * `R`                   — reload
//!   * `L`                   — play the next animation segment once (dev)
//!   * `Esc`                 — open / close the settings menu (see `menu.rs`)
//!
//! Sensitivity, FOV, username and keybinds are configured in the `Esc` menu and
//! persisted (`settings.rs`). Turn on **Debug Mode** there to show the egui
//! tuning panels (`ads_tuning_ui`, top-right) for muzzle flash / smoke / gravity.

mod keybinds;
mod menu;
mod settings;
mod updater;

use std::f32::consts::{FRAC_PI_2, PI};

use keybinds::KeyBindings;
use settings::Settings;

use bevy::{
    animation::RepeatAnimation,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    input::mouse::AccumulatedMouseMotion,
    math::{Affine2, FloatExt},
    pbr::{CascadeShadowConfigBuilder, NotShadowCaster},
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
/// ADS amount below which the scope image is hidden (and its camera switched
/// off), so no stale frame shows at the hip.
const SCOPE_SHOW_AT: f32 = 0.02;
/// Mouse sensitivity is scaled by this at full ADS so the zoomed view isn't
/// twitchy.
const ADS_SENSITIVITY_SCALE: f32 = 0.4;

/// Frame rate the `allanims` clip was baked at in Blender. If the segment cuts
/// below look off, check the console on startup: the game logs the clip's real
/// duration and the fps a 156-frame timeline would imply.
const ANIM_FPS: f32 = 24.0;

/// Magazine capacity and the total number of magazines the player carries
/// (current mag + reserve = `MAG_SIZE * TOTAL_MAGS`).
const MAG_SIZE: u32 = 5;
const TOTAL_MAGS: u32 = 6;

/// Indices into `SEGMENTS`.
const SEG_SHOOT: usize = 0;
const SEG_RECHAMBER: usize = 1;
const SEG_RELOAD: usize = 2;

/// Camera shake: one shot adds `SHAKE_ADD` trauma (capped at 1), which decays at
/// `SHAKE_DECAY` per second. The visible offset scales with `trauma²`, so it is
/// violent immediately and gone in a fraction of a second.
const SHAKE_ADD: f32 = 1.0;
const SHAKE_DECAY: f32 = 3.6;
const SHAKE_FREQ: f32 = 46.0;
const SHAKE_YAW_MAX: f32 = 0.022;
const SHAKE_PITCH_MAX: f32 = 0.045;
const SHAKE_ROLL_MAX: f32 = 0.030;
const SHAKE_POS_MAX: f32 = 0.02;

/// Seconds for the muzzle flash to go from full to gone (it pops on instantly).
const MUZZLE_FLASH_TIME: f32 = 0.06;

/// Hard cap on live smoke particles, and the most that can spawn in one frame.
const SMOKE_MAX: usize = 500;
const SMOKE_MAX_PER_FRAME: f32 = 8.0;

/// Cheap deterministic hash → a float in `[0, 1)`.
fn rand01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    x ^= x >> 22;
    x as f32 / u32::MAX as f32
}

/// Cheap deterministic hash → a roll angle in `[-PI, PI)`.
fn rand_roll(seed: u32) -> f32 {
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
        .add_plugins((settings::SettingsPlugin, menu::MenuPlugin))
        .insert_resource(AmbientLight {
            color: Color::WHITE,
            brightness: 90.0,
            ..default()
        })
        .init_resource::<ViewModelPoses>()
        .init_resource::<SegmentPlayback>()
        .init_resource::<Ads>()
        .init_resource::<AdsTuning>()
        .init_resource::<Weapon>()
        .init_resource::<Shake>()
        .init_resource::<MuzzleFlashSettings>()
        .init_resource::<MuzzleFlashState>()
        .init_resource::<SmokeSettings>()
        .init_resource::<SmokeEmission>()
        .init_resource::<MovementSettings>()
        .init_resource::<Sprinting>()
        .add_systems(
            Startup,
            (
                setup_world,
                setup_player,
                setup_audio,
                setup_hud_camera,
                setup_crosshair,
                setup_ammo_ui,
                setup_fps_ui,
                grab_cursor,
            ),
        )
        // Dev tuning panels — only while debug mode is on (Settings → Controls).
        .add_systems(
            EguiPrimaryContextPass,
            ads_tuning_ui.run_if(menu::debug_enabled),
        )
        .add_systems(Update, update_ads)
        .add_systems(
            Update,
            (
                // Gameplay input / simulation — frozen while a menu is open.
                (toggle_sprint, move_player, teleport_home, jump, apply_gravity)
                    .chain()
                    .run_if(menu::game_active),
                look_around.run_if(menu::game_active),
                weapon_system.run_if(menu::game_active),
                cycle_animation_segments.run_if(menu::game_active),
                // Visuals / HUD — keep running so shake, smoke and the scope
                // settle even while paused.
                apply_ads,
                update_scope,
                sky_follow_camera,
                camera_shake,
                update_muzzle_flash,
                // After `look_around` so the smoke uses this frame's aim, not
                // the previous frame's — otherwise a fast turn leaves the
                // sprites angled toward where the player just was.
                (emit_smoke.run_if(menu::game_active), update_smoke).after(look_around),
                update_ammo_ui,
                update_fps_ui,
            )
                .after(update_ads),
        )
        .run();
}

// ---------------------------------------------------------------------------
// Components / resources
// ---------------------------------------------------------------------------

/// Root of the player rig. Carries position and yaw (horizontal look).
#[derive(Component)]
struct Player;

/// Player physics: horizontal velocity (input-driven on the ground, frozen in
/// the air so a jump carries momentum), falling speed, and whether the feet are
/// resting on a surface.
#[derive(Component, Default)]
struct PlayerPhysics {
    horizontal_velocity: Vec3,
    vertical_velocity: f32,
    grounded: bool,
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

/// Child of `Player`. Carries pitch (vertical look); cameras and the gun hang
/// off of this so movement stays level with the ground.
#[derive(Component)]
struct PlayerHead;

/// The camera that renders the world (layer 0 only).
#[derive(Component)]
struct WorldModelCamera;

/// Parent of every camera and the view model. Its local transform is overwritten
/// each frame by `camera_shake` with the current shake offset (or identity).
#[derive(Component)]
struct CameraShake;

/// The HDR sky sphere; recentred on the camera every frame.
#[derive(Component)]
struct SkySphere;

/// The loaded sniper scene root.
#[derive(Component)]
struct ViewModel;

/// Handles + node index for the sniper's single animation clip.
#[derive(Component)]
struct ViewModelAnimation {
    graph: Handle<AnimationGraph>,
    clip: Handle<AnimationClip>,
    index: AnimationNodeIndex,
}

/// Tracks which slice of `SEGMENTS` the next `L` press will play, and where the
/// currently playing slice should stop.
#[derive(Resource, Default)]
struct SegmentPlayback {
    /// Index into `SEGMENTS` for the next press.
    next: usize,
    /// Clip time (seconds) at which the playing segment should pause.
    stop_at: Option<f32>,
    /// Whether the one-time clip-length report has been logged.
    logged_clip_info: bool,
}

/// Ammo counts and the animation the weapon is mid-way through, if any. While
/// `busy` is `Some` neither firing nor reloading is accepted.
#[derive(Resource)]
struct Weapon {
    /// Rounds in the current magazine.
    mag: u32,
    /// Rounds not in the magazine.
    reserve: u32,
    busy: Option<WeaponBusy>,
}

struct WeaponBusy {
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
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            mag: MAG_SIZE,
            reserve: MAG_SIZE * (TOTAL_MAGS - 1),
            busy: None,
        }
    }
}

/// The bottom-right ammo readout (`mag / reserve`).
#[derive(Component)]
struct AmmoText;

/// The top-left FPS readout.
#[derive(Component)]
struct FpsText;

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
struct MuzzleFlashState {
    intensity: f32,
    roll: f32,
    shots: u32,
}

/// Preloaded sound effects.
#[derive(Resource)]
struct GameSounds {
    shot: Handle<AudioSource>,
}

/// A single drifting, fading smoke sprite. World-space: once spawned it lives in
/// the world, so the player can walk through it.
#[derive(Component)]
struct Smoke {
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
struct SmokeEmission(Option<f32>);

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
struct Ads {
    t: f32,
}

/// Camera-shake state. `trauma` (0..1) is bumped on each shot and decays; it
/// only scales an offset that is rebuilt from zero every frame, so it can never
/// drift the aim. `phase` just advances the oscillation and resets at rest.
#[derive(Resource, Default)]
struct Shake {
    trauma: f32,
    phase: f32,
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
    // Ground: a 200 m plane textured with a tiled 1 m grid. Baking the grid into
    // the ground material (instead of drawing it with gizmos) keeps it in the
    // normal depth sort, so transparent things like smoke draw over it correctly.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(100.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(build_grid_texture())),
            uv_transform: Affine2::from_scale(Vec2::splat(200.0)),
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

    // Player-sized reference dummies (1.8 m capsules) straight ahead at known
    // distances, so the amount of zoom is easy to read. Player spawns at z = 5
    // looking toward -Z.
    let dummy_mesh = meshes.add(Capsule3d::new(0.3, 1.2));
    let dummy_material = materials.add(Color::srgb(0.85, 0.42, 0.15));
    for distance in [10.0f32, 25.0, 50.0, 100.0] {
        commands.spawn((
            Mesh3d(dummy_mesh.clone()),
            MeshMaterial3d(dummy_material.clone()),
            Transform::from_xyz(0.0, 0.9, 5.0 - distance),
        ));
    }

    // Sun.
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
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
    let (graph, index) = AnimationGraph::from_clip(clip.clone());
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
                            // World-model camera: renders the default layer 0.
                            rig.spawn((
                                WorldModelCamera,
                                Camera3d::default(),
                                Projection::from(PerspectiveProjection {
                                    fov: 90.0_f32.to_radians(),
                                    ..default()
                                }),
                            ));

                            // Scope camera: renders the world (layer 0) plus the
                            // reticle quad (layer 2, no view model) through a
                            // narrow FOV into `scope_image`. `order: -1` so it runs
                            // before the main camera; inactive until ADS.
                            rig.spawn((
                                ScopeCamera,
                                Camera3d::default(),
                                Camera {
                                    target: scope_image.clone().into(),
                                    order: -1,
                                    is_active: false,
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

                            // View-model camera: renders only layer 1 (the gun),
                            // on top. A tiny near plane lets the weapon come right
                            // up to the lens without being clipped.
                            rig.spawn((
                                Camera3d::default(),
                                Camera {
                                    order: 1,
                                    ..default()
                                },
                                Projection::from(PerspectiveProjection {
                                    fov: 70.0_f32.to_radians(),
                                    near: 0.0001,
                                    ..default()
                                }),
                                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                            ));

                            // The sniper view model itself.
                            rig.spawn((
                                ViewModel,
                                ViewModelAnimation { graph, clip, index },
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

        // The scope's rear lens ("lens_lens_0" in Blender): show the scope
        // render target on it, unlit, faded in by `update_scope`.
        if is_mesh && name_has("lens") {
            commands.entity(entity).insert((
                ScopeLens,
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                    base_color_texture: Some(scope_rt.0.clone()),
                    // The render target samples V-flipped on the lens; undo it.
                    uv_transform: Affine2::from_scale_angle_translation(
                        Vec2::new(1.0, -1.0),
                        0.0,
                        Vec2::new(0.0, 1.0),
                    ),
                    unlit: true,
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

fn setup_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(GameSounds {
        shot: asset_server.load("audio/sniper_shot.wav"),
    });
}

/// A 2D camera drawn last (order 2, no clear) that hosts all the HUD, so the
/// view-model gun (view-model camera, order 1) can't render over it.
fn setup_hud_camera(mut commands: Commands) {
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
    mut movement: ResMut<MovementSettings>,
    ads: Res<Ads>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("ADS tuning")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .resizable(false)
        .vscroll(true)
        .show(ctx, |ui| {
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
            ui.add(egui::Slider::new(&mut a.pitch, -0.6f32..=0.6).text("pitch").step_by(0.001));
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
                ui.add(egui::Slider::new(&mut sm.spawn_offset.x, -0.8f32..=0.8).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut sm.spawn_offset.y, -0.8f32..=0.8).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut sm.spawn_offset.z, -3.0f32..=0.0).text("z  (forward -)"),
                );
                ui.add(egui::Slider::new(&mut sm.scale, 0.02f32..=3.0).text("scale (m)"));
                ui.add(egui::Slider::new(&mut sm.rise_rate, 0.0f32..=4.0).text("rise rate (m/s)"));
                ui.add(egui::Slider::new(&mut sm.spread, 0.0f32..=2.0).text("spread (m/s)"));
                ui.add(egui::Slider::new(&mut sm.fade_in, 0.0f32..=5.0).text("fade in (s)"));
                ui.add(egui::Slider::new(&mut sm.fade_time, 0.1f32..=10.0).text("fade out (s)"));
                ui.add(egui::Slider::new(&mut sm.spawn_rate, 0.0f32..=150.0).text("spawn rate (/s)"));
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
            ui.collapsing("Movement", |ui| {
                let m = &mut *movement;
                ui.add(egui::Slider::new(&mut m.walk_speed, 0.0f32..=20.0).text("walk speed (m/s)"));
                ui.add(
                    egui::Slider::new(&mut m.sprint_speed, 0.0f32..=30.0).text("sprint speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut m.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(
                    egui::Slider::new(&mut m.jump_speed, 0.0f32..=20.0).text("jump strength (m/s)"),
                );
                if ui.button("Reset movement").clicked() {
                    *m = MovementSettings::default();
                }
            });
        });
    Ok(())
}

// ---------------------------------------------------------------------------
// Update systems
// ---------------------------------------------------------------------------

/// Left Shift toggles between walk and sprint.
fn toggle_sprint(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut sprinting: ResMut<Sprinting>,
) {
    if window.cursor_options.grab_mode != CursorGrabMode::None
        && binds.sprint.just_pressed(&keys, &mouse)
    {
        sprinting.0 = !sprinting.0;
    }
}

fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    settings: Res<MovementSettings>,
    sprinting: Res<Sprinting>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let (mut transform, mut physics) = player.into_inner();

    // Input only steers you while your feet are on something — in the air the
    // velocity from the moment you left the ground carries you.
    if physics.grounded {
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

        let speed = if sprinting.0 {
            settings.sprint_speed
        } else {
            settings.walk_speed
        };
        physics.horizontal_velocity = direction.normalize_or_zero() * speed;
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

/// Launches the player upward when they're standing on something.
fn jump(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    settings: Res<MovementSettings>,
    mut physics: Single<&mut PlayerPhysics, With<Player>>,
) {
    if window.cursor_options.grab_mode == CursorGrabMode::None {
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

    if feet_next <= surface {
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
    player.rotate_y(-delta.x * sens.x);

    // ...pitch on the head, clamped so we can't flip over.
    let (_, current_pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
    let new_pitch = (current_pitch - delta.y * sens.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    head.rotation = Quat::from_rotation_x(new_pitch);
}

/// Ramp `Ads::t` toward 1 while the right mouse button is held, back toward 0
/// otherwise.
fn update_ads(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    tuning: Res<AdsTuning>,
    mut ads: ResMut<Ads>,
) {
    if tuning.force_full {
        ads.t = 1.0;
        return;
    }

    let aiming = window.cursor_options.grab_mode != CursorGrabMode::None
        && binds.aim.pressed(&keys, &mouse);
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

    let alpha = ease(ads.t);
    for (material, mut visibility) in &mut lens {
        *visibility = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(material) = materials.get_mut(&material.0) {
            material.base_color = Color::srgba(1.0, 1.0, 1.0, alpha);
        }
    }
}

/// Build a 1-cell grid tile: mostly `ground` colour with `line`-coloured pixels
/// along two edges, wrapped so it tiles into a full grid.
fn build_grid_texture() -> Image {
    const N: u32 = 256;
    const LINE: u32 = 3;
    let ground = [31u8, 33, 38, 255];
    let line = [90u8, 97, 115, 255];

    let mut data = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let on_line = x < LINE || y < LINE;
            data.extend_from_slice(if on_line { &line } else { &ground });
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

/// Fire and reload (bindings). Firing plays Shoot → Rechamber and spends a round;
/// reload plays the Reload segment and then refills the mag from the reserve.
/// Nothing new is accepted while an animation is mid-play, so shots are
/// impossible until a reload finishes.
fn weapon_system(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    view_model: Single<&ViewModelAnimation>,
    mut players: Query<&mut AnimationPlayer>,
    mut weapon: ResMut<Weapon>,
    mut shake: ResMut<Shake>,
    mut muzzle: ResMut<MuzzleFlashState>,
    mut smoke: ResMut<SmokeEmission>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    let node = view_model.index;
    let Some(mut player) = players.iter_mut().next() else {
        return;
    };

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
            Ok(next) => play_segment(&mut player, node, next),
            Err(on_finish) => {
                // Whole queue played out: park at the rest pose, apply effect.
                if let Some(active) = player.animation_mut(node) {
                    active.seek_to(0.0);
                    active.pause();
                }
                weapon.busy = None;
                if on_finish == WeaponFinish::Reload {
                    let moved = (MAG_SIZE - weapon.mag).min(weapon.reserve);
                    weapon.mag += moved;
                    weapon.reserve -= moved;
                }
            }
        }
        return;
    }

    // Idle: only take input while the cursor is captured (i.e. in-game).
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }

    if binds.fire.just_pressed(&keys, &mouse) && weapon.mag > 0 {
        weapon.mag -= 1;
        shake.trauma = (shake.trauma + SHAKE_ADD).min(1.0);
        muzzle.shots = muzzle.shots.wrapping_add(1);
        muzzle.roll = rand_roll(muzzle.shots);
        muzzle.intensity = 1.0;
        smoke.0 = Some(0.0);
        commands.spawn((
            AudioPlayer::new(sounds.shot.clone()),
            PlaybackSettings::DESPAWN,
        ));
        play_segment(&mut player, node, SEGMENTS[SEG_SHOOT]);
        weapon.busy = Some(WeaponBusy {
            remaining: vec![SEGMENTS[SEG_SHOOT], SEGMENTS[SEG_RECHAMBER]],
            seg_end: SEGMENTS[SEG_SHOOT].end_secs(),
            on_finish: WeaponFinish::Nothing,
        });
    } else if binds.reload.just_pressed(&keys, &mouse)
        && weapon.mag < MAG_SIZE
        && weapon.reserve > 0
    {
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
            Transform::from_translation(origin)
                .with_scale(Vec3::splat(settings.scale.max(1.0e-4))),
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

/// Rebuild the `CameraShake` node's local transform from the current trauma —
/// always from zero, so it settles back to exactly identity and never drifts aim.
fn camera_shake(
    time: Res<Time>,
    mut shake: ResMut<Shake>,
    mut rig: Single<&mut Transform, With<CameraShake>>,
) {
    let dt = time.delta_secs();
    shake.trauma = (shake.trauma - SHAKE_DECAY * dt).max(0.0);

    if shake.trauma <= 0.0 {
        shake.phase = 0.0;
        if **rig != Transform::IDENTITY {
            **rig = Transform::IDENTITY;
        }
        return;
    }

    shake.phase += dt * SHAKE_FREQ;
    let s = shake.phase;
    let amt = shake.trauma * shake.trauma;

    **rig = Transform {
        translation: Vec3::new(
            (s * 1.53 + 0.4).sin() * SHAKE_POS_MAX * amt,
            (s * 1.19 + 3.3).sin() * SHAKE_POS_MAX * amt,
            0.0,
        ),
        rotation: Quat::from_euler(
            EulerRot::YXZ,
            s.sin() * SHAKE_YAW_MAX * amt,
            (s * 1.37 + 1.1).sin() * SHAKE_PITCH_MAX * amt,
            (s * 0.83 + 2.7).sin() * SHAKE_ROLL_MAX * amt,
        ),
        scale: Vec3::ONE,
    };
}

/// Keep the bottom-right readout in sync with the ammo counts.
fn update_ammo_ui(weapon: Res<Weapon>, mut text: Single<&mut Text, With<AmmoText>>) {
    let wanted = format!("{} / {}", weapon.mag, weapon.reserve);
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// `L` plays the next `SEGMENTS` entry once, from its start frame to its end
/// frame, then parks the animation there. Each press advances to the following
/// segment (wrapping around), so the whole clip can be stepped through and each
/// cut dialed in.
fn cycle_animation_segments(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    clips: Res<Assets<AnimationClip>>,
    view_model: Single<&ViewModelAnimation>,
    mut players: Query<&mut AnimationPlayer>,
    mut state: ResMut<SegmentPlayback>,
) {
    let node = view_model.index;
    let Some(mut player) = players.iter_mut().next() else {
        return;
    };

    // One-time: report the real clip length so ANIM_FPS / SEGMENTS can be
    // checked against reality.
    if !state.logged_clip_info {
        if let Some(clip) = clips.get(&view_model.clip) {
            let duration = clip.duration();
            info!(
                "allanims: duration {:.4}s at assumed {} fps",
                duration, ANIM_FPS
            );
            info!(
                "allanims: if the timeline runs 0..156, real fps is ~{:.3}",
                156.0 / duration
            );
            for (i, seg) in SEGMENTS.iter().enumerate() {
                let past_end = seg.end_secs() > duration + 1e-3;
                info!(
                    "  [{}] {:<12} frames {:>3}..{:<3}  {:.4}s..{:.4}s{}",
                    i,
                    seg.name,
                    seg.start_frame as i32,
                    seg.end_frame as i32,
                    seg.start_secs(),
                    seg.end_secs(),
                    if past_end { "  <- beyond clip end" } else { "" },
                );
            }
            state.logged_clip_info = true;
        }
    }

    // Pause the playing segment once it reaches its end frame.
    if let Some(stop_at) = state.stop_at {
        if let Some(active) = player.animation_mut(node) {
            if active.is_finished() || active.seek_time() >= stop_at {
                active.seek_to(stop_at);
                active.pause();
                state.stop_at = None;
            }
        }
    }

    if !binds.cycle_anim.just_pressed(&keys, &mouse) {
        return;
    }

    let seg = SEGMENTS[state.next];
    let start = seg.start_secs();
    let end = seg.end_secs();

    let active = player.play(node);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(1.0);
    active.replay();
    active.seek_to(start);
    active.resume();

    state.stop_at = Some(end);
    info!(
        "L -> [{}] {} (frames {}..{}, {:.3}s..{:.3}s)",
        state.next,
        seg.name,
        seg.start_frame as i32,
        seg.end_frame as i32,
        start,
        end,
    );
    state.next = (state.next + 1) % SEGMENTS.len();
}
