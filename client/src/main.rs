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
//!   * `T`                   — teleport to the saved point (spawn by default)
//!   * `G`                   — save the current position as the teleport point
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
use settings::{Settings, ShadowQuality};

use bevy::{
    animation::RepeatAnimation,
    audio::Volume,
    core_pipeline::bloom::Bloom,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    input::mouse::AccumulatedMouseMotion,
    math::{Affine2, FloatExt},
    pbr::{
        CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap, DistanceFog,
        FogFalloff, NotShadowCaster,
    },
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
/// Render layer for the player's body capsule — a shadow-only stand-in so the
/// sun casts a humanoid shadow instead of just the floating sniper + arms.
/// No camera renders this layer; only the sun's `RenderLayers` includes it.
const PLAYER_BODY_LAYER: usize = 3;
/// Empty render layer for the HUD camera, which renders after everything else
/// (including the view-model camera) so the UI is never covered by the gun.
const UI_LAYER: usize = 4;

/// Body capsule dimensions (metres) — a rough humanoid silhouette for the
/// shadow, not a real collider.
const BODY_CAPSULE_RADIUS: f32 = 0.25;
const BODY_CAPSULE_HEIGHT: f32 = 1.6;

/// Bold condensed display face used across the in-game HUD (score, ammo, fps,
/// score popups) and the kill-cam banner — the closest free/open stand-in for
/// the tall all-caps look modern Call of Duty titles use for this kind of text.
pub(crate) const HUD_FONT: &str = "fonts/BebasNeue-Regular.ttf";

/// Radius of the sky sphere. Kept inside the camera far plane; the sphere
/// follows the camera so the player never reaches its edge.
const SKY_RADIUS: f32 = 900.0;

/// Placement of `basic_map.glb` (position / yaw / uniform scale), live-tweakable
/// from the debug panel's "Map" section and pushed onto the loaded scene by
/// `apply_map_transform`. Rotation is yaw-only (about Y) — `map_surface_height`
/// derives a closed-form walkable surface from the model's known geometry, and
/// that only works for a level, unrotated-in-pitch/roll piece.
#[derive(Resource)]
struct MapSettings {
    position: Vec3,
    rotation_deg: f32,
    scale: f32,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            position: Vec3::new(16.0, 0.0, 8.0),
            rotation_deg: 0.0,
            scale: 2.0,
        }
    }
}

/// Marker on the spawned `basic_map.glb` scene root, so `apply_map_transform`
/// can find it.
#[derive(Component)]
struct MapModel;

/// One walkable piece of `basic_map.glb`'s hand-fit collision, in the model's
/// own local space (before `MapSettings` position / yaw / scale is applied).
/// There's no runtime mesh collision in this game (see `apply_gravity`'s
/// building-roof check) — `map_surface_height` closed-form computes "what's
/// the floor here" the same way, just with more pieces. `RampX` / `RampZ`
/// linearly interpolate height along the given axis between two edges.
enum MapSurface {
    Flat {
        x: (f32, f32),
        z: (f32, f32),
        y: f32,
    },
    RampX {
        z: (f32, f32),
        x_lo: f32,
        x_hi: f32,
        y_lo: f32,
        y_hi: f32,
    },
    RampZ {
        x: (f32, f32),
        z_lo: f32,
        z_hi: f32,
        y_lo: f32,
        y_hi: f32,
    },
}

/// Hand-measured from `basic_map.glb`'s node transforms: the base cube, a ramp
/// up onto it, a second taller cube reached by a second ramp, a third cube,
/// and the bridge connecting the second and third. Keep in sync with the
/// model if it's re-exported with different dimensions.
const MAP_SURFACES: &[MapSurface] = &[
    // First cube — top platform (widened to x: -2..6).
    MapSurface::Flat {
        x: (-2.0, 6.0),
        z: (-2.0, 2.0),
        y: 4.0,
    },
    // Ramp from the ground up onto the first cube (moved to its new edge).
    MapSurface::RampZ {
        x: (4.0, 6.0),
        z_lo: -9.2,
        z_hi: -2.0,
        y_lo: 0.0,
        y_hi: 4.0,
    },
    // Second, taller cube.
    MapSurface::Flat {
        x: (-6.0, -2.0),
        z: (-2.0, 2.0),
        y: 6.0,
    },
    // Ramp from the first cube's top up onto the second.
    MapSurface::RampX {
        z: (0.0, 2.0),
        x_lo: -2.0,
        x_hi: 3.22,
        y_lo: 6.0,
        y_hi: 4.0,
    },
    // Third cube, reached by the bridge.
    MapSurface::Flat {
        x: (-20.0, -16.0),
        z: (-2.0, 2.0),
        y: 6.0,
    },
    // Bridge connecting the second and third cubes.
    MapSurface::Flat {
        x: (-16.0, -6.0),
        z: (-0.98, 0.98),
        y: 6.0,
    },
];

/// World-space height of `basic_map.glb`'s walkable surface at `(world_x,
/// world_z)`, or `None` if that point isn't over any of it (so the caller
/// falls through to the ground). Inverse of the position/yaw/scale transform
/// `apply_map_transform` applies to the visual model, so collision always
/// matches what's on screen.
fn map_surface_height(world_x: f32, world_z: f32, map: &MapSettings) -> Option<f32> {
    let scale = map.scale.max(1.0e-4);
    let theta = map.rotation_deg.to_radians();
    let (sin, cos) = (theta.sin(), theta.cos());
    let dx = world_x - map.position.x;
    let dz = world_z - map.position.z;
    let lx = (dx * cos - dz * sin) / scale;
    let lz = (dx * sin + dz * cos) / scale;

    let mut best: Option<f32> = None;
    for s in MAP_SURFACES {
        let h = match *s {
            MapSurface::Flat { x, z, y } => {
                (lx >= x.0 && lx <= x.1 && lz >= z.0 && lz <= z.1).then_some(y)
            }
            MapSurface::RampZ {
                x,
                z_lo,
                z_hi,
                y_lo,
                y_hi,
            } => (lx >= x.0 && lx <= x.1 && lz >= z_lo && lz <= z_hi).then(|| {
                let t = ((lz - z_lo) / (z_hi - z_lo)).clamp(0.0, 1.0);
                y_lo.lerp(y_hi, t)
            }),
            MapSurface::RampX {
                z,
                x_lo,
                x_hi,
                y_lo,
                y_hi,
            } => (lz >= z.0 && lz <= z.1 && lx >= x_lo.min(x_hi) && lx <= x_lo.max(x_hi)).then(
                || {
                    let t = ((lx - x_lo) / (x_hi - x_lo)).clamp(0.0, 1.0);
                    y_lo.lerp(y_hi, t)
                },
            ),
        };
        if let Some(h) = h {
            best = Some(best.map_or(h, |b: f32| b.max(h)));
        }
    }
    best.map(|h| h * scale + map.position.y)
}

#[cfg(test)]
mod map_surface_tests {
    use super::*;

    fn settings(position: Vec3, rotation_deg: f32, scale: f32) -> MapSettings {
        MapSettings {
            position,
            rotation_deg,
            scale,
        }
    }

    #[test]
    fn flat_cube_top() {
        // (5.0, -1.5) is on the first cube's top but clear of both ramps'
        // footprints, which overlap the cube's edges where they attach.
        let map = settings(Vec3::ZERO, 0.0, 1.0);
        assert_eq!(map_surface_height(5.0, -1.5, &map), Some(4.0));
    }

    #[test]
    fn ramp_interpolates_between_its_edges() {
        let map = settings(Vec3::ZERO, 0.0, 1.0);
        assert!((map_surface_height(5.0, -9.2, &map).unwrap() - 0.0).abs() < 1.0e-3);
        assert!((map_surface_height(5.0, -2.0, &map).unwrap() - 4.0).abs() < 1.0e-3);
        let mid = map_surface_height(5.0, -5.6, &map).unwrap();
        assert!(
            mid > 1.5 && mid < 2.5,
            "expected a mid-ramp height, got {mid}"
        );
    }

    #[test]
    fn outside_every_footprint_is_none() {
        let map = settings(Vec3::ZERO, 0.0, 1.0);
        assert_eq!(map_surface_height(50.0, 50.0, &map), None);
    }

    #[test]
    fn position_offsets_the_surface() {
        let map = settings(Vec3::new(100.0, 5.0, 200.0), 0.0, 1.0);
        assert_eq!(map_surface_height(105.0, 198.5, &map), Some(9.0)); // 4.0 + 5.0
        assert_eq!(map_surface_height(0.0, 0.0, &map), None);
    }

    #[test]
    fn scale_multiplies_footprint_and_height() {
        let map = settings(Vec3::ZERO, 0.0, 2.0);
        assert_eq!(map_surface_height(10.0, -3.0, &map), Some(8.0)); // local (5,-1.5) * scale
        assert_eq!(map_surface_height(12.0, -4.0, &map), Some(8.0)); // local edge x=6,z=-2
        assert_eq!(map_surface_height(12.1, -4.0, &map), None);
    }

    #[test]
    fn second_ramp_connects_the_first_and_second_cube_tops() {
        let map = settings(Vec3::ZERO, 0.0, 1.0);
        // Low edge (x=3.22, into the first cube's footprint) is flush with its top.
        assert!((map_surface_height(3.22, 1.0, &map).unwrap() - 4.0).abs() < 1.0e-3);
        // High edge (x=-2, the shared wall) is flush with the second cube's top.
        assert!((map_surface_height(-2.0, 1.0, &map).unwrap() - 6.0).abs() < 1.0e-3);
    }

    #[test]
    fn second_and_third_cube_tops_and_bridge_are_all_walkable() {
        let map = settings(Vec3::ZERO, 0.0, 1.0);
        assert_eq!(map_surface_height(-4.0, 0.0, &map), Some(6.0)); // second cube
        assert_eq!(map_surface_height(-18.0, 0.0, &map), Some(6.0)); // third cube
        assert_eq!(map_surface_height(-11.0, 0.0, &map), Some(6.0)); // bridge between them
    }

    #[test]
    fn yaw_rotates_the_footprint() {
        let map = settings(Vec3::ZERO, 90.0, 1.0);
        assert!((map_surface_height(-9.2, -5.0, &map).unwrap() - 0.0).abs() < 1.0e-3);
        assert!((map_surface_height(-2.0, -5.0, &map).unwrap() - 4.0).abs() < 1.0e-3);
    }
}

/// Push `MapSettings` onto the loaded scene whenever it changes (also once at
/// startup, which just re-applies the defaults).
fn apply_map_transform(map: Res<MapSettings>, model: Single<&mut Transform, With<MapModel>>) {
    if !map.is_changed() {
        return;
    }
    let mut transform = model.into_inner();
    transform.translation = map.position;
    transform.rotation = Quat::from_rotation_y(map.rotation_deg.to_radians());
    transform.scale = Vec3::splat(map.scale);
}

/// Where the player spawns, and the initial [`TeleportPoint`] the teleport key
/// returns to. Ground level, facing the basic map's ramp. Follows
/// `MapSettings`'s default placement — if you move the map far from its default,
/// update this too.
const SPAWN_POS: Vec3 = Vec3::new(26.0, EYE_HEIGHT, -15.0);

/// The position and facing the teleport key snaps the player back to. Starts
/// at [`SPAWN_POS`] facing -Z (the spawn orientation); the "save teleport
/// point" key resets it to wherever the player is standing and looking.
/// Runtime-only — back to spawn on each launch.
#[derive(Resource)]
struct TeleportPoint {
    position: Vec3,
    /// Body yaw (rotation about Y), radians — matches `Player`'s `Transform`.
    yaw: f32,
    /// Head pitch, radians — matches `PlayerHead`'s `Transform`.
    pitch: f32,
}

impl Default for TeleportPoint {
    fn default() -> Self {
        Self {
            position: SPAWN_POS,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}
/// Player camera height above the feet — used to test the feet against surfaces.
const EYE_HEIGHT: f32 = 1.7;
/// Defaults for the "Movement" panel section (all live-adjustable).
const WALK_SPEED: f32 = 6.0;
const SPRINT_SPEED: f32 = 10.5;
const GRAVITY: f32 = 22.0;
const JUMP_SPEED: f32 = 8.0;
/// Feet within this distance above a surface still count as standing on it.
const GROUND_SNAP: f32 = 0.5;

/// How many `audio/footsteps/footstep_N.wav` clips there are (1-indexed).
const FOOTSTEP_CLIPS: usize = 10;

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
const ADS_DURATION: f32 = 0.4;
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
/// while fully scoped (fully matte — with `reflectance` at 0 there's no sun
/// glint on the glass once aimed in).
const LENS_ROUGHNESS_HIP: f32 = 0.04;
const LENS_ROUGHNESS_ADS: f32 = 1.0;
/// Metalness / reflectance of the glass at the hip; both fall to zero as the
/// player scopes in, leaving a non-reflective black backing behind the sight
/// picture so the sun can't glint off the lens while aimed in.
const LENS_METALLIC_HIP: f32 = 0.65;
const LENS_REFLECTANCE_HIP: f32 = 1.0;

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
const SHAKE_RECOIL_KICK: f32 = 0.05;
/// How fast that backward kick eases back to zero (larger = snappier return).
const SHAKE_RECOIL_RETURN: f32 = 5.0;
/// Fraction of the camera shake that survives at full ADS; it ramps linearly
/// back to the full effect (`1.0`) at the hip so scoped aim stays controllable.
const SHAKE_ADS_SCALE: f32 = 0.15;

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

/// No-scope inaccuracy ("No-scope spread" panel section). Each shot is thrown
/// off the aim point by a random up/down + left/right angle; the cap on that
/// angle is `hip_max_deg` at the hip and eases to `0` at full ADS, so a fully
/// scoped shot always lands dead on. The random draw can still come out small,
/// which is what lets an occasional hip shot connect.
#[derive(Resource)]
struct NoScopeSpread {
    /// Largest up/down or left/right offset at the hip, in degrees.
    hip_max_deg: f32,
    /// Accuracy curve: `1` = linear falloff, `>1` keeps the spread wide through
    /// a partial ADS and then tightens fast as it approaches full ADS.
    curve: f32,
}

impl Default for NoScopeSpread {
    fn default() -> Self {
        Self {
            hip_max_deg: 4.0,
            curve: 2.5,
        }
    }
}

/// The per-axis half-angle (radians) a shot may be thrown off the aim point at
/// the given ADS amount — `cfg.hip_max_deg` at `ads_t == 0`, easing to `0` at
/// `ads_t == 1` along `1 - ads_t^curve`.
fn noscope_spread_angle(cfg: &NoScopeSpread, ads_t: f32) -> f32 {
    let t = ads_t.clamp(0.0, 1.0);
    let scale = (1.0 - t.powf(cfg.curve.max(0.01))).clamp(0.0, 1.0);
    cfg.hip_max_deg.to_radians() * scale
}

/// Cheap "breathing" motion shared by [`idle_weapon_sway`] and [`update_scope`]'s
/// aim sway: two sine waves at different frequencies per axis trace a slow
/// Lissajous loop instead of a straight back-and-forth line, which reads as
/// more organic than a single shared frequency would.
fn breathing_offset(clock: f32, freq_hz: Vec2, amp_rad: Vec2) -> Vec2 {
    Vec2::new(
        (clock * freq_hz.x * std::f32::consts::TAU).sin() * amp_rad.x,
        (clock * freq_hz.y * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2).sin()
            * amp_rad.y,
    )
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
        .init_resource::<IdleSwayState>()
        .init_resource::<IdleSwaySettings>()
        .init_resource::<CrosshairSwayState>()
        .init_resource::<AimSwayState>()
        .init_resource::<AimSwaySettings>()
        .init_resource::<CrosshairSettings>()
        .init_resource::<TeleportPoint>()
        .init_resource::<NoScopeSpread>()
        .init_resource::<Weapon>()
        .init_resource::<ThrowingKnife>()
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
        .init_resource::<BloodSettings>()
        .init_resource::<TracerSettings>()
        .init_resource::<TrickState>()
        .add_event::<GroundImpact>()
        .add_event::<BloodImpact>()
        .add_event::<FireTracer>()
        .add_event::<TrickScoredEvent>()
        .add_event::<MatchEndedEvent>()
        .init_resource::<MovementSettings>()
        .init_resource::<Sprinting>()
        .init_resource::<Slide>()
        .init_resource::<SlideSettings>()
        .init_resource::<FootstepSettings>()
        .init_resource::<FootstepState>()
        .init_resource::<SoundVolumes>()
        .init_resource::<SceneTuning>()
        .init_resource::<MapSettings>()
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
                setup_bot_assets,
            ),
        )
        .add_systems(
            OnEnter(AppState::InGame),
            (
                grab_cursor,
                start_ambient,
                reset_slide,
                reset_trick,
                reset_weapon,
            ),
        )
        .add_systems(OnEnter(AppState::MainMenu), release_cursor)
        .add_systems(Update, (hud_visibility, crosshair_root_visibility))
        .add_systems(Update, apply_master_volume)
        // In `Last`, so it sees `AudioSink`s that bevy_audio adds in this
        // frame's `PostUpdate` and can scale them before they've really played.
        .add_systems(Last, apply_sound_volumes)
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
                    // Reads this frame's velocity + grounded state, so last in
                    // the chain.
                    footsteps,
                )
                    .chain()
                    .run_if(menu::game_active.and(killcam::no_killcam)),
                (look_around, save_teleport_point)
                    .run_if(menu::game_active.and(killcam::no_killcam)),
                weapon_system.run_if(menu::game_active.and(killcam::no_killcam)),
                // Visuals / HUD — keep running so shake, smoke and the scope
                // settle even while paused.
                apply_ads,
                update_scope.after(crosshair_sway),
                (fade_crosshair, update_crosshair_visibility),
                track_trick.after(look_around).run_if(killcam::no_killcam),
                (spawn_score_popup, update_score_popups, update_teleport_toast),
                sky_follow_camera,
                camera_shake.run_if(killcam::no_killcam),
                update_muzzle_flash,
                // After `look_around` so the smoke uses this frame's aim, not
                // the previous frame's — otherwise a fast turn leaves the
                // sprites angled toward where the player just was.
                (emit_smoke.run_if(menu::game_active), update_smoke).after(look_around),
                (
                    spawn_ground_impact,
                    spawn_blood_impact,
                    update_impact_particles,
                )
                    .after(look_around),
                (spawn_tracers, update_tracers),
                update_ammo_ui,
                update_fps_ui,
                apply_scene_tuning,
                apply_shadow_quality,
                apply_map_transform,
                debug_cursor_toggle,
            )
                .after(update_ads)
                .run_if(in_state(AppState::InGame)),
        )
        // Weapon sway rides on top of the ADS pose, using this frame's turn;
        // idle sway then layers a "breathing" drift on top while the player's
        // still, and the recoil shudder rides on top of both. All three stop
        // during a kill cam — `killcam::drive_killcam` reproduces them from the
        // recorded sway / shake state instead, so replay isn't stacked on top
        // of these reading live (frozen) state.
        .add_systems(
            Update,
            (weapon_sway, idle_weapon_sway, weapon_recoil_shudder)
                .chain()
                .after(look_around)
                .after(apply_ads)
                .run_if(in_state(AppState::InGame).and(killcam::no_killcam)),
        )
        // The scope reticle trails the aim (`crosshair_sway`); `update_scope`
        // then renders it with that offset. `consume_look_delta` clears this
        // frame's turn once both sway readers have had it — `look_around`
        // early-returns on a still frame without clearing it itself. These run
        // through a kill cam too, so the reticle eases back to centre on replay.
        .add_systems(
            Update,
            (
                crosshair_sway.after(look_around),
                consume_look_delta
                    .after(weapon_sway)
                    .after(crosshair_sway),
            )
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

/// Panel-adjustable footstep audio ("Footsteps" panel section). One step plays
/// every `*_stride` metres travelled on foot, so cadence rises with speed and
/// each stance (crouch / walk / sprint / prone) gets its own pace and loudness —
/// the Call-of-Duty model.
#[derive(Resource)]
struct FootstepSettings {
    /// Master on / off for footstep audio.
    enabled: bool,
    /// Overall volume, multiplying every per-stance level below.
    volume: f32,
    /// Metres travelled between steps, per stance.
    walk_stride: f32,
    sprint_stride: f32,
    crouch_stride: f32,
    prone_stride: f32,
    /// Per-stance loudness (linear, before `volume`).
    walk_volume: f32,
    sprint_volume: f32,
    crouch_volume: f32,
    prone_volume: f32,
    /// Random playback-rate (pitch) spread, ± this around 1.0, so repeats of the
    /// same clip don't sound identical.
    pitch_jitter: f32,
    /// Planar speed (m/s) below which the player counts as stopped.
    min_speed: f32,
}

impl Default for FootstepSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 3.0,
            walk_stride: 2.0,
            sprint_stride: 2.5,
            crouch_stride: 1.5,
            prone_stride: 1.2,
            walk_volume: 0.5,
            sprint_volume: 0.85,
            crouch_volume: 0.28,
            prone_volume: 0.2,
            pitch_jitter: 0.12,
            min_speed: 0.5,
        }
    }
}

/// Running footstep cadence state for the local player.
#[derive(Resource, Default)]
struct FootstepState {
    /// Distance (m) covered since the last step.
    accum: f32,
    /// Whether the player was moving on foot last frame — a fresh start fires a
    /// step immediately rather than after a full stride of silence.
    was_moving: bool,
    /// Index of the last clip played, so the next pick avoids an instant repeat.
    last: usize,
    /// Bumped each step, seeds the clip pick + pitch jitter.
    seq: u32,
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

/// Set by `weapon_system` on the frame the trigger is pulled; consumed by
/// `net::write_input`, which turns it into the tick's fire request. Carries the
/// world-space shot direction — already thrown off by no-scope inaccuracy (see
/// [`NoScopeSpread`]) — so the local hit resolution and the server agree.
#[derive(Resource, Default)]
pub(crate) struct PendingShot(pub Option<Vec3>);

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

/// Set by `start_view_model_animation` on the descendant entity that carries
/// the sniper's `AnimationPlayer`. Bots have their own `AnimationPlayer`s now
/// (see [`BotAnimationPlayer`]), so anywhere that used to assume "the"
/// `AnimationPlayer` in the world was the sniper's needs to filter on this.
#[derive(Component)]
pub(crate) struct SniperAnimationPlayer;

/// `models/bot.glb` is imported noticeably larger than [`shared::bots::BOT_HEIGHT`]
/// (the invisible hitbox capsule) — scaled down so what's on screen lines up
/// with where shots actually register.
pub(crate) const BOT_MODEL_SCALE: f32 = 2.0 / 3.0;

/// Playback speed for the death clip (`Armature|MTF_Die`) — plays out twice
/// as fast as authored.
pub(crate) const BOT_DIE_SPEED: f32 = 2.0;

/// Graph + node indices for `models/bot.glb`'s two animations, built once at
/// startup: `MTF_IdleAction1` (looped while alive) and `Armature|MTF_Die`
/// (played once on death).
#[derive(Resource, Clone)]
pub(crate) struct BotAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    die: AnimationNodeIndex,
}

/// Tags a bot's `SceneRoot` entity (practice bot, networked avatar, or kill-cam
/// ghost) so `start_bot_animation` knows to wire it up once the scene finishes
/// spawning — and so the observer can tell a bot's scene apart from any other
/// (the sniper, the map, ...).
#[derive(Component)]
pub(crate) struct BotVisual;

/// Set by `start_bot_animation` once it finds the `AnimationPlayer` inside a
/// bot's spawned scene, so later systems (death handling) can reach it
/// directly instead of re-walking the hierarchy every frame.
#[derive(Component)]
pub(crate) struct BotAnimationPlayer(pub(crate) Entity);

/// Build the bot animation graph. Added to the same `Startup` tuple as
/// `setup_player` / `setup_crosshair` — adding a *separate*
/// `add_systems(Startup, ...)` call (e.g. from a plugin) perturbs that tuple's
/// execution order enough to black out the in-game 3D view, so this must stay
/// part of the one tuple.
fn setup_bot_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(2).from_asset("models/bot.glb"));
    let die_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(3).from_asset("models/bot.glb"));
    let (graph, indices) = AnimationGraph::from_clips([idle_clip, die_clip]);
    let graph = graphs.add(graph);
    commands.insert_resource(BotAnimations {
        graph,
        idle: indices[0],
        die: indices[1],
    });
}

/// Fires once a bot's `SceneRoot` (tagged [`BotVisual`]) finishes spawning:
/// starts the idle animation looping and remembers which descendant holds the
/// `AnimationPlayer`, via [`BotAnimationPlayer`], for `play_bot_death` to use
/// later.
fn start_bot_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    bots: Query<(), With<BotVisual>>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<BotAnimations>,
) {
    let root = trigger.target();
    if !bots.contains(root) {
        return;
    }
    for entity in children.iter_descendants(root) {
        // Same fix as the sniper view model: a skinned mesh is frustum-culled
        // against its *rest-pose* AABB, not the animated one, so the idle
        // sway can walk the head/body right out of a rest-pose-computed
        // frustum — most visible zoomed in (scope), where the FOV is narrow.
        commands.entity(entity).insert(NoFrustumCulling);

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anims.idle);
            active.set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert(AnimationGraphHandle(anims.graph.clone()));
            commands.entity(root).insert(BotAnimationPlayer(entity));
        }
    }
}

/// Switch a bot to its death animation, played once. Callers should only
/// invoke this on the frame death is first observed (e.g. guarded by their
/// own "already dying" flag) — a no-op if the scene hasn't finished spawning
/// yet, in which case the bot just never animates a death, same as a shot
/// landing before `start_bot_animation` has run.
pub(crate) fn play_bot_death(
    root: Entity,
    roots: &Query<&BotAnimationPlayer>,
    players: &mut Query<&mut AnimationPlayer>,
    anims: &BotAnimations,
) {
    let Ok(target) = roots.get(root) else { return };
    let Ok(mut player) = players.get_mut(target.0) else {
        return;
    };
    let active = player.play(anims.die);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(BOT_DIE_SPEED);
    active.replay();
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

/// The dead-centre white dot; `fade_crosshair` fades it out as the player aims
/// in, `update_crosshair_visibility` shows it only while the sniper is the
/// active weapon and the throwing knife isn't held.
#[derive(Component)]
struct CenterDot;

/// The throwing-knife reticle (four ticks with a gap in the middle); shown in
/// place of [`CenterDot`] while [`ThrowingKnife::active`] is set.
#[derive(Component)]
struct ThrowingKnifeCrosshair;

/// Root of the crosshair overlay (both [`CenterDot`] and
/// [`ThrowingKnifeCrosshair`] live under it). Unlike `menu::HudElement`
/// (which `hud_visibility` also hides the instant a kill cam starts), this
/// root stays available through a replay — `killcam::drive_killcam` drives
/// `Weapon::slot` and [`ThrowingKnife::active`] off the recorded samples, so
/// the replay shows the same crosshair the shooter had at each moment. It's
/// still hidden outside a live game or behind a menu, via
/// `crosshair_root_visibility`.
#[derive(Component)]
struct CrosshairRoot;

/// Hold-to-snap-out state for the throwing-knife key. Independent of
/// [`WeaponSlot`] — it's an instant overlay on whatever's currently equipped
/// (snap the sniper away with no Hide animation, so a shot's rechamber can be
/// cut off for a "silent" quickscope), not a real weapon switch.
#[derive(Resource, Default)]
pub(crate) struct ThrowingKnife {
    pub(crate) active: bool,
}

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
    pub(crate) kill_enemy: Handle<AudioSource>,
    pub(crate) jump_land: Handle<AudioSource>,
    pub(crate) teleport: Handle<AudioSource>,
    /// `audio/footsteps/footstep_1..N.wav` — `footsteps` picks one at random
    /// per step.
    pub(crate) footsteps: Vec<Handle<AudioSource>>,
}

/// Linear volume of the looping nature ambience.
const AMBIENT_VOLUME: f32 = 0.5;

/// Panel-adjustable per-sound volume multipliers ("Sound volumes" panel
/// section). `1.0` leaves a sound at its built-in level; every one-shot is
/// scaled by its entry when it spawns (`apply_sound_volumes`), and the ambient
/// bed by `ambient` (folded into `apply_master_volume`). Footsteps have their
/// own controls in the "Footsteps" section and aren't here.
#[derive(Resource)]
struct SoundVolumes {
    shot: f32,
    rechamber: f32,
    reload: f32,
    ambient: f32,
    aim_in: f32,
    aim_out: f32,
    out_of_ammo: f32,
    slide: f32,
    dive: f32,
    kill_enemy: f32,
    jump_land: f32,
    teleport: f32,
}

impl Default for SoundVolumes {
    fn default() -> Self {
        Self {
            shot: 1.0,
            rechamber: 1.0,
            reload: 1.5,
            ambient: 2.5,
            aim_in: 1.0,
            aim_out: 1.0,
            out_of_ammo: 1.0,
            slide: 1.0,
            dive: 1.0,
            kill_enemy: 5.5,
            jump_land: 0.5,
            teleport: 1.0,
        }
    }
}

impl SoundVolumes {
    /// The multiplier for `handle`, or `None` if it isn't a one-shot this
    /// resource covers (footstep clips, the ambient loop).
    fn oneshot_for(&self, handle: &Handle<AudioSource>, sounds: &GameSounds) -> Option<f32> {
        let id = handle.id();
        [
            (sounds.shot.id(), self.shot),
            (sounds.rechamber.id(), self.rechamber),
            (sounds.reload.id(), self.reload),
            (sounds.aim_in.id(), self.aim_in),
            (sounds.aim_out.id(), self.aim_out),
            (sounds.out_of_ammo.id(), self.out_of_ammo),
            (sounds.slide.id(), self.slide),
            (sounds.dive.id(), self.dive),
            (sounds.kill_enemy.id(), self.kill_enemy),
            (sounds.jump_land.id(), self.jump_land),
            (sounds.teleport.id(), self.teleport),
        ]
        .into_iter()
        .find_map(|(hid, vol)| (hid == id).then_some(vol))
    }
}

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

/// A shot's visual tracer path, `start -> end` in world space. Fired for the
/// local shooter's own shot the instant it's taken (`resolve_local_shot`, both
/// game modes) and for every other player's shot off the server's
/// authoritative `ShotResolved` broadcast (`net::receive_shots`). Consumed by
/// `spawn_tracers`.
#[derive(Event)]
pub(crate) struct FireTracer {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
}

/// A streak along a shot's path: a brief bright "just fired" flash, then a
/// lingering smoke trail along the same line that widens and fades out. The
/// sniper is instant-hitscan, so there's no real flight time to animate —
/// both phases are a stylised read of "a shot just went through here."
#[derive(Component)]
pub(crate) struct Tracer {
    age: f32,
}

/// Panel-adjustable tracer look (`ads_tuning_ui`'s "Tracer" section).
#[derive(Resource)]
struct TracerSettings {
    /// Seconds the bright flash phase lasts.
    flash_secs: f32,
    /// Seconds the smoke trail then fades over.
    smoke_secs: f32,
    /// Flash color — bright and additive/emissive so `Bloom` (already on the
    /// world camera) reads it as a hot, glowing streak.
    flash_color: [f32; 3],
    /// How many stops brighter than `flash_color` the emissive glow is.
    flash_emissive_boost: f32,
    /// Line radius (m) during the flash phase.
    flash_radius: f32,
    /// Smoke-trail color — desaturated, alpha-blended, not emissive.
    smoke_color: [f32; 3],
    /// Opacity the smoke trail starts at, right as the flash ends.
    smoke_start_alpha: f32,
    /// Line radius (m) the smoke trail has widened to by the end of its fade.
    smoke_radius: f32,
}

impl Default for TracerSettings {
    fn default() -> Self {
        Self {
            flash_secs: 0.06,
            smoke_secs: 3.0,
            flash_color: srgb_parts(Color::srgb(1.0, 0.8, 0.35)),
            flash_emissive_boost: 6.0,
            flash_radius: 0.018,
            smoke_color: srgb_parts(Color::srgb(0.72, 0.7, 0.66)),
            smoke_start_alpha: 0.45,
            smoke_radius: 0.05,
        }
    }
}

/// Shared unit cylinder (radius 0.5, height 1) that `spawn_tracers` scales to
/// each shot's length/width, so tracers don't allocate a fresh mesh per shot.
#[derive(Resource)]
struct TracerAssets {
    mesh: Handle<Mesh>,
}

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
    /// RGB the sprite's blended material is tinted with (multiplies the
    /// texture) — white for rocks / dust, `BloodSettings::color` for blood.
    tint: [f32; 3],
}

/// Shared quad + the impact textures (`spawn_ground_impact` / `spawn_blood_impact`
/// clone a fresh material per particle so each fades on its own). `blood` is a
/// transparent PNG whose droplets already carry their own colour.
#[derive(Resource)]
struct ImpactAssets {
    quad: Handle<Mesh>,
    dust: Handle<Image>,
    rocks: Handle<Image>,
    blood: Handle<Image>,
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

/// A shot connected with a bot at `point`, travelling along `dir` (unit).
/// Consumed by `spawn_blood_impact`, which squirts a blood burst out along the
/// shot from that point. Written by `practice::resolve_local_shot` for the
/// local shooter's own hits and by `net::follow_bot_avatars` when a
/// server-owned bot drops.
#[derive(Event)]
pub(crate) struct BloodImpact {
    pub(crate) point: Vec3,
    pub(crate) dir: Vec3,
}

/// Panel-adjustable blood squirt for a bot hit (`ads_tuning_ui`'s "Blood
/// splatter" section).
#[derive(Resource)]
struct BloodSettings {
    /// Droplets launched per hit.
    count: u32,
    /// Squirt speed (m/s), randomised.
    speed: f32,
    /// Spray cone half-angle around the shot direction (degrees).
    spread_deg: f32,
    /// Downward acceleration (m/s²) — droplets arc down.
    gravity: f32,
    /// Per-second velocity damping.
    drag: f32,
    /// Droplet sprite size at spawn (m), randomised.
    scale: f32,
    /// How much a droplet grows over its life (× `scale`).
    growth: f32,
    /// Seconds a droplet lives, randomised.
    lifetime: f32,
    /// Peak opacity, multiplying the texture's own alpha.
    opacity: f32,
    /// Multiplies the texture's colour — white keeps the PNG's own dark red.
    color: [f32; 3],
}

impl Default for BloodSettings {
    fn default() -> Self {
        Self {
            count: 40,
            speed: 6.0,
            spread_deg: 50.0,
            gravity: 17.0,
            drag: 1.4,
            scale: 0.2,
            growth: 2.0,
            lifetime: 1.0,
            opacity: 0.8,
            color: srgb_parts(Color::WHITE),
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
pub(crate) struct Shake {
    pub(crate) trauma: f32,
    pub(crate) phase: f32,
    pub(crate) recoil: f32,
}

/// Panel-adjustable camera-shake tuning. The oscillation + view-punch half
/// mirrors the `SHAKE_*` consts; the recoil half is the forward/back kick that
/// keeps the eye behind the scope lens when firing.
#[derive(Resource)]
pub(crate) struct ShakeSettings {
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
    /// Fraction of the camera shake left at full ADS (`0` = none, `1` = full).
    /// Ramps linearly to the full effect as the player aims back out.
    ads_scale: f32,
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
            ads_scale: SHAKE_ADS_SCALE,
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
pub(crate) struct AdsTuning {
    /// Pin ADS to fully aimed regardless of the right mouse button, so the pose
    /// can be tuned with the cursor free.
    force_full: bool,
    /// World-camera FOV (degrees) at full ADS. Lower = more zoom.
    fov_deg: f32,
    /// Scope camera FOV (degrees). Lower = more magnification inside the scope.
    scope_fov_deg: f32,
    /// Milliseconds to go from hip to full aim-down-sight (and back), the way
    /// Call of Duty reports ADS time. Lower = snappier.
    ads_duration_ms: f32,
    /// Shape of the hip↔ADS blend: `0` = linear, `1` = full ease-in-out
    /// (smootherstep). Applied to the view-model pose and the FOV zoom.
    ads_ease: f32,
    /// `Ads::t` at which the magnified sight picture starts fading onto the
    /// glass. Below this the lens just reads as reflective glass and the scope
    /// camera tracks the world FOV, so aiming in is a clean zoom-and-raise
    /// instead of a second, pre-zoomed image of the target sliding into place.
    /// `0.0` = old behaviour (picture fades in across the whole blend).
    scope_picture_at: f32,
}

impl Default for AdsTuning {
    fn default() -> Self {
        Self {
            force_full: false,
            fov_deg: ADS_FOV_DEG,
            scope_fov_deg: SCOPE_FOV_DEG,
            ads_duration_ms: ADS_DURATION * 1000.0,
            ads_ease: 0.8,
            scope_picture_at: 0.25,
        }
    }
}

/// The yaw / pitch the view actually rotated by this frame (radians), written by
/// [`look_around`]. Read by [`weapon_sway`] and [`crosshair_sway`], then zeroed
/// by [`consume_look_delta`] so a frame with no mouse input (or a paused game)
/// reads as "no turn" — `look_around` early-returns on a still frame without
/// touching it.
#[derive(Resource, Default)]
struct LookDelta {
    /// `x` = yaw (positive = turned left), `y` = pitch (positive = looked up).
    applied: Vec2,
}

/// Running weapon-sway offset (radians), a low-passed lag behind the view.
#[derive(Resource, Default)]
pub(crate) struct WeaponSwayState {
    /// `x` = yaw offset, `y` = pitch offset, applied on top of the ADS pose.
    pub(crate) offset: Vec2,
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

/// Running clock + blend for [`idle_weapon_sway`].
#[derive(Resource, Default)]
struct IdleSwayState {
    /// Seconds, advanced only while `blend` is above ~0 so the motion doesn't
    /// jump mid-cycle when it resumes after a walk.
    clock: f32,
    /// Eases `0` (moving) → `1` (settled at rest).
    blend: f32,
}

/// Panel-adjustable idle sway: a slow procedural "breathing" drift added to the
/// view model on top of [`WeaponSwaySettings`] while the player stands still —
/// that one only reacts to turning, this one is present even dead still. Fades
/// in on stopping / out on moving, and fades toward zero approaching full ADS,
/// where [`AimSwaySettings`] takes over the job of keeping the aim alive.
#[derive(Resource)]
struct IdleSwaySettings {
    /// Peak yaw/pitch drift (degrees), fully settled at the hip.
    amplitude_deg: Vec2,
    /// Cycles per second of the yaw / pitch drift — different so the weapon
    /// traces a slow loop instead of a straight back-and-forth line.
    frequency_hz: Vec2,
    /// How fast the effect blends in/out around a stop/start (larger = snappier).
    blend_speed: f32,
}

impl Default for IdleSwaySettings {
    fn default() -> Self {
        Self {
            amplitude_deg: Vec2::new(0.35, 0.22),
            frequency_hz: Vec2::new(0.18, 0.26),
            blend_speed: 2.5,
        }
    }
}

/// Running scope-reticle lag (radians of scope view), low-passed toward the
/// turn target by [`crosshair_sway`].
#[derive(Resource, Default)]
struct CrosshairSwayState {
    /// `x` = yaw drift, `y` = pitch drift, added to the reticle while scoped.
    offset: Vec2,
}

/// Panel-adjustable scope-reticle behaviour: size on the glass, the CoD-style
/// aim-in drift (starts toward one corner and slides to centre as you scope in),
/// the turn lag (the crosshair trails the aim a beat behind the gun's own
/// [`WeaponSwaySettings`] sway), and whether the HUD centre dot ever fades.
#[derive(Resource)]
struct CrosshairSettings {
    /// Reticle size multiplier (`1.0` = fills the scope view exactly; `>1` pushes
    /// the crosshair's outer ends past the glass edge).
    scale: f32,
    /// Where the reticle sits at `Ads::t == 0`, as a fraction of the scope's
    /// half-view — it eases to centre by full ADS. Positive `x` = left, positive
    /// `y` = up, so the default starts the crosshair toward the upper-left like a
    /// CoD scope. The scope camera counter-aims by the same amount so the target
    /// stays under the reticle.
    aim_in_frac: Vec2,
    /// Keep the HUD centre dot at full opacity instead of fading it out as the
    /// sight picture comes in.
    center_dot_always: bool,
    /// Turn-lag: seconds of lag — reticle drift ≈ turn rate (rad/s) × this.
    strength: f32,
    /// How fast the reticle eases back to centre (smaller = trails longer).
    return_speed: f32,
    /// Hard cap on the turn-lag drift, in degrees of the scope camera's view.
    max_offset_deg: f32,
}

impl Default for CrosshairSettings {
    fn default() -> Self {
        Self {
            scale: 1.2,
            aim_in_frac: Vec2::new(3.0, 3.0),
            center_dot_always: false,
            strength: 0.01,
            return_speed: 12.0,
            max_offset_deg: 0.3,
        }
    }
}

/// Running clock for [`update_scope`]'s aim-sway drift.
#[derive(Resource, Default)]
struct AimSwayState {
    clock: f32,
}

/// Panel-adjustable "aiming idle sway": a slow breathing drift on the scope
/// reticle while aiming down sights, scaled up toward full ADS instead of down
/// (the opposite of [`IdleSwaySettings`], which fades out approaching ADS).
/// Purely visual — the world camera itself never moves, so it never touches
/// where a shot actually lands (see [`weapon_system`]'s aim ray).
#[derive(Resource)]
struct AimSwaySettings {
    /// Peak yaw/pitch drift (degrees of scope view) at full ADS.
    amplitude_deg: Vec2,
    /// Cycles per second of the yaw / pitch drift.
    frequency_hz: Vec2,
}

impl Default for AimSwaySettings {
    fn default() -> Self {
        Self {
            amplitude_deg: Vec2::new(0.12, 0.09),
            frequency_hz: Vec2::new(0.2, 0.31),
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
                translation: Vec3::new(0.13, -0.24, -0.4),
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

/// Smoothstep easing, used for the scope sight-picture fade.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The hip↔ADS blend curve. `amount` blends from a straight line (`0`) to
/// smootherstep (`1`), so the aim-in slows into both ends by a tunable degree.
fn ads_ease(t: f32, amount: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let smootherstep = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    t + (smootherstep - t) * amount.clamp(0.0, 1.0)
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

    // The Blender-built basic map (cubes, ramps, bridge) — `apply_map_transform`
    // positions it from `MapSettings`; walkability is a hand-fit height field,
    // not real mesh collision (see `map_surface_height`).
    commands.spawn((
        MapModel,
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/basic_map.glb"))),
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
        // Light the world, the view model, and the (invisible) body capsule.
        RenderLayers::from_layers(&[0, VIEW_MODEL_RENDER_LAYER, PLAYER_BODY_LAYER]),
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

    // Bullet-impact debris (`spawn_ground_impact` / `spawn_blood_impact` clone a
    // material per particle).
    commands.insert_resource(ImpactAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        dust: asset_server.load("textures/dust.png"),
        rocks: asset_server.load("textures/rocks.png"),
        blood: asset_server.load("textures/blood-splatter-texture.png"),
    });

    // Unit cylinder (`spawn_tracers` scales + colors it per shot).
    commands.insert_resource(TracerAssets {
        mesh: meshes.add(Cylinder::new(0.5, 1.0)),
    });

    commands
        .spawn((
            Player,
            PlayerPhysics::default(),
            Transform::from_translation(SPAWN_POS),
            Visibility::default(),
        ))
        .with_children(|player| {
            // Invisible body capsule: no camera renders `PLAYER_BODY_LAYER`, but
            // the sun does, so it casts a humanoid shadow instead of a floating
            // gun + arms.
            player.spawn((
                Mesh3d(meshes.add(Capsule3d::new(
                    BODY_CAPSULE_RADIUS,
                    BODY_CAPSULE_HEIGHT - 2.0 * BODY_CAPSULE_RADIUS,
                ))),
                MeshMaterial3d(materials.add(Color::srgb(0.5, 0.5, 0.5))),
                Transform::from_xyz(0.0, BODY_CAPSULE_HEIGHT / 2.0 - EYE_HEIGHT, 0.0),
                RenderLayers::layer(PLAYER_BODY_LAYER),
            ));

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
            commands.entity(entity).insert((
                AnimationGraphHandle(anim.graph.clone()),
                // Bots now carry their own `AnimationPlayer`s too, so anything
                // that used to grab "the" `AnimationPlayer` unfiltered needs
                // this to pick the sniper's back out.
                SniperAnimationPlayer,
            ));
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
        kill_enemy: asset_server.load("audio/kill-enemy-sound.mp3"),
        jump_land: asset_server.load("audio/jump-landing-sound.mp3"),
        teleport: asset_server.load("audio/teleport.wav"),
        footsteps: (1..=FOOTSTEP_CLIPS)
            .map(|i| asset_server.load(format!("audio/footsteps/footstep_{i}.wav")))
            .collect(),
    });
}

/// Start the looping outdoor ambience when the player enters the world.
fn start_ambient(
    mut commands: Commands,
    sounds: Res<GameSounds>,
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
) {
    commands.spawn((
        AmbientAudio,
        StateScoped(AppState::InGame),
        AudioPlayer::new(sounds.ambient.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(
            AMBIENT_VOLUME * vols.ambient * settings.master_volume,
        )),
    ));
}

/// Push `Settings::master_volume` onto Bevy's `GlobalVolume`, which scales
/// every one-shot sound spawned from here on (shots, footsteps, UI, ...) with
/// no per-call-site changes needed. `GlobalVolume` doesn't retroactively touch
/// audio that's already playing, though, so the looping ambience needs its own
/// direct nudge here too — also picking up the "Sound volumes" panel's ambient
/// multiplier.
fn apply_master_volume(
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    mut global_volume: ResMut<GlobalVolume>,
    mut ambient: Query<&mut AudioSink, With<AmbientAudio>>,
) {
    if !settings.is_changed() && !vols.is_changed() {
        return;
    }
    global_volume.volume = Volume::Linear(settings.master_volume);
    for mut sink in &mut ambient {
        sink.set_volume(Volume::Linear(
            AMBIENT_VOLUME * vols.ambient * settings.master_volume,
        ));
    }
}

/// Scale each freshly-started one-shot to its "Sound volumes" multiplier.
/// bevy_audio bakes `PlaybackSettings::volume * GlobalVolume` into the sink when
/// it starts it, so we re-derive the same product with the per-sound factor
/// mixed in. Sounds not covered here (footsteps, the ambient loop) are left be.
fn apply_sound_volumes(
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    global_volume: Res<GlobalVolume>,
    mut fresh: Query<(&AudioPlayer, &mut AudioSink), Added<AudioSink>>,
) {
    let Some(sounds) = sounds else { return };
    for (player, mut sink) in &mut fresh {
        if let Some(mult) = vols.oneshot_for(&player.0, &sounds) {
            sink.set_volume(Volume::Linear(mult.max(0.0)) * global_volume.volume);
        }
    }
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

/// Same gating as `hud_visibility` (live game, no menu overlay) but *without*
/// the kill-cam check — a replay should still show the crosshair the shooter
/// had at each moment.
fn crosshair_root_visibility(
    state: Res<State<AppState>>,
    menu: Res<menu::Menu>,
    mut root: Query<&mut Visibility, With<CrosshairRoot>>,
) {
    if !(state.is_changed() || menu.is_changed()) {
        return;
    }
    let show = *state.get() == AppState::InGame && !menu.is_open();
    let want = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut root {
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

/// A small white dot dead-centre for lining the scope up, plus the (initially
/// hidden) throwing-knife reticle shown in its place while the knife is held.
/// Both live under one `CrosshairRoot` so they share the same centring node
/// and the same visibility gating (`crosshair_root_visibility`).
fn setup_crosshair(mut commands: Commands) {
    let bar = |width: f32, height: f32| {
        (
            Node {
                width: Val::Px(width),
                height: Val::Px(height),
                ..default()
            },
            BackgroundColor(Color::WHITE),
            BorderRadius::MAX,
        )
    };

    commands
        .spawn((
            CrosshairRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
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

            root.spawn((
                ThrowingKnifeCrosshair,
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
            ))
            .with_children(|knife| {
                knife.spawn(bar(2.0, 40.0));
                knife
                    .spawn(Node {
                        column_gap: Val::Px(16.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(bar(20.0, 2.0));
                        row.spawn(bar(20.0, 2.0));
                    });
                knife.spawn(bar(2.0, 40.0));
            });
        });
}

/// Fade the centre dot out as the player aims down the scope — fully gone once
/// the sight picture has come in, fully back at the hip — so it never sits over
/// the sight picture but still gives an aim reference through the raise.
fn fade_crosshair(
    ads: Res<Ads>,
    tuning: Res<AdsTuning>,
    crosshair: Res<CrosshairSettings>,
    dot: Single<(&mut BackgroundColor, &mut BorderColor), With<CenterDot>>,
) {
    let a = if crosshair.center_dot_always {
        1.0
    } else {
        1.0 - scope_picture_amount(ads.t, &tuning)
    };
    let (mut bg, mut border) = dot.into_inner();
    bg.0 = Color::srgba(1.0, 1.0, 1.0, a);
    border.0 = Color::srgba(0.0, 0.0, 0.0, 0.6 * a);
}

/// Pick the reticle: the centre dot only while the sniper is the active
/// weapon and the throwing knife isn't held; the throwing-knife crosshair
/// only while it is. During a kill cam, `killcam::drive_killcam` drives
/// `weapon.slot` / `ThrowingKnife::active` off the recorded samples, so this
/// reproduces the same swap the shooter saw instead of a live-only readout.
fn update_crosshair_visibility(
    weapon: Res<Weapon>,
    knife: Res<ThrowingKnife>,
    mut dot: Query<&mut Visibility, (With<CenterDot>, Without<ThrowingKnifeCrosshair>)>,
    mut reticle: Query<&mut Visibility, (With<ThrowingKnifeCrosshair>, Without<CenterDot>)>,
) {
    if !(weapon.is_changed() || knife.is_changed()) {
        return;
    }
    let sniper_active = weapon.slot == WeaponSlot::Primary;
    let dot_want = if sniper_active && !knife.active {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let reticle_want = if knife.active {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if let Ok(mut v) = dot.single_mut() {
        *v = dot_want;
    }
    if let Ok(mut v) = reticle.single_mut() {
        *v = reticle_want;
    }
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
    asset_server: Res<AssetServer>,
    sounds: Res<GameSounds>,
) {
    // Only the most recent shot matters if several land in one frame.
    let Some(ev) = events.read().last() else {
        return;
    };
    for e in &existing {
        commands.entity(e).despawn();
    }

    // `TrickScoredEvent` only ever fires for a kill *this* client just scored
    // (Practice resolves it locally; online, `net::receive_trick_scores`
    // already filters the server's broadcast down to our own shooter id).
    commands.spawn((
        AudioPlayer::new(sounds.kill_enemy.clone()),
        PlaybackSettings::DESPAWN,
    ));

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
                        font: asset_server.load(HUD_FONT),
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
fn setup_ammo_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
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
                font: asset_server.load(HUD_FONT),
                font_size: 30.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// Top-left frames-per-second readout.
fn setup_fps_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
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
                font: asset_server.load(HUD_FONT),
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
#[allow(clippy::type_complexity)]
fn ads_tuning_ui(
    mut contexts: EguiContexts,
    mut poses: ResMut<ViewModelPoses>,
    mut tuning: ResMut<AdsTuning>,
    mut muzzle: ResMut<MuzzleFlashSettings>,
    mut smoke: ResMut<SmokeSettings>,
    mut rocks: ResMut<RockSettings>,
    mut dust: ResMut<DustSettings>,
    mut movement: ResMut<MovementSettings>,
    (mut slide_cfg, mut footsteps, mut sound_vol, mut crosshair_cfg): (
        ResMut<SlideSettings>,
        ResMut<FootstepSettings>,
        ResMut<SoundVolumes>,
        ResMut<CrosshairSettings>,
    ),
    mut sway: ResMut<WeaponSwaySettings>,
    mut shake_cfg: ResMut<ShakeSettings>,
    mut anim: ResMut<AnimationSettings>,
    mut scene: ResMut<SceneTuning>,
    mut tracer: ResMut<TracerSettings>,
    binds: Res<KeyBindings>,
    // Bundled — a system function tops out at 16 top-level params.
    misc: (
        Res<Shake>,
        Res<Ads>,
        ResMut<MapSettings>,
        ResMut<BloodSettings>,
        ResMut<NoScopeSpread>,
        ResMut<IdleSwaySettings>,
        ResMut<AimSwaySettings>,
    ),
) -> Result {
    let (shake, ads, mut map, mut blood, mut noscope, mut idle_sway, mut aim_sway) = misc;
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

            ui.separator();
            ui.collapsing("FOV", |ui| {
                ui.add(
                    egui::Slider::new(&mut tuning.fov_deg, 3.0f32..=45.0)
                        .text("main ADS FOV°  (lower = more zoom)"),
                );
                ui.add(
                    egui::Slider::new(&mut tuning.scope_fov_deg, 1.0f32..=30.0)
                        .text("scope FOV°  (lower = more magnification)"),
                );
                if ui.button("Reset FOV").clicked() {
                    let d = AdsTuning::default();
                    tuning.fov_deg = d.fov_deg;
                    tuning.scope_fov_deg = d.scope_fov_deg;
                }
            });

            ui.separator();
            ui.collapsing("ADS speed", |ui| {
                ui.add(
                    egui::Slider::new(&mut tuning.ads_duration_ms, 20.0f32..=1000.0)
                        .text("ADS time (ms)  (lower = snappier)")
                        .suffix(" ms")
                        .max_decimals(0),
                );
                ui.add(
                    egui::Slider::new(&mut tuning.ads_ease, 0.0f32..=1.0)
                        .text("easing  (0 = linear, 1 = ease in/out)"),
                );
                ui.add(
                    egui::Slider::new(&mut tuning.scope_picture_at, 0.0f32..=0.95)
                        .text("scope picture in at (ads.t)  (higher = later)"),
                );
                if ui.button("Reset ADS speed").clicked() {
                    let d = AdsTuning::default();
                    tuning.ads_duration_ms = d.ads_duration_ms;
                    tuning.ads_ease = d.ads_ease;
                    tuning.scope_picture_at = d.scope_picture_at;
                }
            });

            ui.separator();
            ui.collapsing("ADS pose", |ui| {
                let a = &mut poses.ads;
                ui.add(egui::Slider::new(&mut a.translation.x, -0.4f32..=0.4).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut a.translation.y, -0.4f32..=0.4).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut a.translation.z, -0.8f32..=0.0).text("z  (forward -)"),
                );
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
            });

            ui.separator();
            ui.collapsing("Hip pose", |ui| {
                let h = &mut poses.hip;
                ui.add(egui::Slider::new(&mut h.translation.x, -0.4f32..=0.4).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut h.translation.y, -0.4f32..=0.4).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut h.translation.z, -0.8f32..=0.0).text("z  (forward -)"),
                );
                ui.add(
                    egui::Slider::new(&mut h.yaw, (-PI)..=PI)
                        .text("yaw")
                        .step_by(0.001),
                );
                ui.add(
                    egui::Slider::new(&mut h.pitch, -0.6f32..=0.6)
                        .text("pitch")
                        .step_by(0.001),
                );
                ui.add(
                    egui::Slider::new(&mut h.scale, 0.001f32..=0.05)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy hip pose to console").clicked() {
                    info!(
                        "hip: ViewModelOffset {{ translation: Vec3::new({:.4}, {:.4}, {:.4}), \
                         yaw: {:.4}, pitch: {:.4}, scale: {:.5} }},",
                        h.translation.x, h.translation.y, h.translation.z, h.yaw, h.pitch, h.scale,
                    );
                }
                if ui.button("Reset hip pose to default").clicked() {
                    *h = ViewModelPoses::default().hip;
                }
            });

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
            ui.collapsing("Tracer", |ui| {
                let tr = &mut *tracer;
                ui.label("flash (just fired)");
                ui.add(
                    egui::Slider::new(&mut tr.flash_secs, 0.0f32..=0.3).text("flash duration (s)"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.flash_radius, 0.005f32..=0.15)
                        .text("flash radius (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.flash_emissive_boost, 0.0f32..=15.0)
                        .text("flash glow (emissive ×)"),
                );
                ui.horizontal(|ui| {
                    ui.label("flash color");
                    ui.color_edit_button_rgb(&mut tr.flash_color);
                });

                ui.label("smoke trail");
                ui.add(
                    egui::Slider::new(&mut tr.smoke_secs, 0.1f32..=8.0).text("fade duration (s)"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.smoke_start_alpha, 0.0f32..=1.0)
                        .text("starting opacity"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.smoke_radius, 0.01f32..=0.4).text("end radius (m)"),
                );
                ui.horizontal(|ui| {
                    ui.label("smoke color");
                    ui.color_edit_button_rgb(&mut tr.smoke_color);
                });

                if ui.button("Reset tracer").clicked() {
                    *tr = TracerSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Map", |ui| {
                let mp = &mut *map;
                ui.label("position");
                ui.add(egui::Slider::new(&mut mp.position.x, -100.0f32..=100.0).text("x"));
                ui.add(egui::Slider::new(&mut mp.position.y, -20.0f32..=20.0).text("y"));
                ui.add(egui::Slider::new(&mut mp.position.z, -100.0f32..=100.0).text("z"));
                ui.add(
                    egui::Slider::new(&mut mp.rotation_deg, -180.0f32..=180.0)
                        .text("rotation°  (yaw)"),
                );
                ui.add(
                    egui::Slider::new(&mut mp.scale, 0.1f32..=5.0)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy map transform to console").clicked() {
                    info!(
                        "map: position Vec3::new({:.2}, {:.2}, {:.2}), rotation_deg: {:.1}, \
                         scale: {:.3}",
                        mp.position.x, mp.position.y, mp.position.z, mp.rotation_deg, mp.scale,
                    );
                }
                if ui.button("Reset map transform").clicked() {
                    *mp = MapSettings::default();
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
            ui.collapsing("Blood splatter", |ui| {
                let b = &mut *blood;
                ui.label("squirted from a bot along the shot where it hits");
                ui.add(egui::Slider::new(&mut b.count, 0u32..=60).text("droplets per hit"));
                ui.add(egui::Slider::new(&mut b.speed, 0.0f32..=25.0).text("squirt speed (m/s)"));
                ui.add(egui::Slider::new(&mut b.spread_deg, 0.0f32..=90.0).text("spray cone (°)"));
                ui.add(egui::Slider::new(&mut b.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(egui::Slider::new(&mut b.drag, 0.0f32..=10.0).text("drag (/s)"));
                ui.add(egui::Slider::new(&mut b.scale, 0.01f32..=0.8).text("droplet size (m)"));
                ui.add(egui::Slider::new(&mut b.growth, 1.0f32..=4.0).text("grow ×  (over life)"));
                ui.add(egui::Slider::new(&mut b.lifetime, 0.1f32..=4.0).text("lifetime (s)"));
                ui.add(egui::Slider::new(&mut b.opacity, 0.0f32..=1.0).text("opacity"));
                ui.horizontal(|ui| {
                    ui.label("tint  (white = texture as-is)");
                    ui.color_edit_button_rgb(&mut b.color);
                });
                if ui.button("Reset blood").clicked() {
                    *b = BloodSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Footsteps", |ui| {
                let f = &mut *footsteps;
                ui.checkbox(&mut f.enabled, "enabled");
                ui.add(egui::Slider::new(&mut f.volume, 0.0f32..=10.0).text("overall volume"));
                ui.label("stride — metres per step (lower = faster cadence)");
                ui.add(egui::Slider::new(&mut f.walk_stride, 0.5f32..=5.0).text("walk"));
                ui.add(egui::Slider::new(&mut f.sprint_stride, 0.5f32..=5.0).text("sprint"));
                ui.add(egui::Slider::new(&mut f.crouch_stride, 0.5f32..=5.0).text("crouch"));
                ui.add(egui::Slider::new(&mut f.prone_stride, 0.5f32..=5.0).text("prone"));
                ui.label("per-stance volume");
                ui.add(egui::Slider::new(&mut f.walk_volume, 0.0f32..=1.0).text("walk"));
                ui.add(egui::Slider::new(&mut f.sprint_volume, 0.0f32..=1.0).text("sprint"));
                ui.add(egui::Slider::new(&mut f.crouch_volume, 0.0f32..=1.0).text("crouch"));
                ui.add(egui::Slider::new(&mut f.prone_volume, 0.0f32..=1.0).text("prone"));
                ui.add(
                    egui::Slider::new(&mut f.pitch_jitter, 0.0f32..=0.5).text("pitch jitter (±)"),
                );
                ui.add(
                    egui::Slider::new(&mut f.min_speed, 0.0f32..=3.0).text("stopped below (m/s)"),
                );
                if ui.button("Reset footsteps").clicked() {
                    *f = FootstepSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Sound volumes", |ui| {
                let v = &mut *sound_vol;
                ui.label("per-sound multiplier (1 = built-in level)");
                for (label, slot) in [
                    ("shot", &mut v.shot),
                    ("rechamber", &mut v.rechamber),
                    ("reload", &mut v.reload),
                    ("ambient", &mut v.ambient),
                    ("aim in", &mut v.aim_in),
                    ("aim out", &mut v.aim_out),
                    ("out of ammo", &mut v.out_of_ammo),
                    ("slide", &mut v.slide),
                    ("dive", &mut v.dive),
                    ("kill enemy", &mut v.kill_enemy),
                    ("jump land", &mut v.jump_land),
                    ("teleport", &mut v.teleport),
                ] {
                    ui.add(egui::Slider::new(slot, 0.0f32..=10.0).text(label));
                }
                if ui.button("Reset sound volumes").clicked() {
                    *v = SoundVolumes::default();
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
                ui.add(egui::Slider::new(&mut s.dive_jump, 0.0f32..=12.0).text("dive hop (m/s)"));
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
            ui.collapsing("Idle sway", |ui| {
                let s = &mut *idle_sway;
                ui.label("weapon 'breathing' drift while standing still, hip only");
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.x, 0.0f32..=2.0)
                        .text("amplitude X (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.y, 0.0f32..=2.0)
                        .text("amplitude Y (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.x, 0.02f32..=1.0)
                        .text("frequency X (Hz)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.y, 0.02f32..=1.0)
                        .text("frequency Y (Hz)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.blend_speed, 0.2f32..=10.0)
                        .text("blend speed (stop / go)"),
                );
                if ui.button("Reset idle sway").clicked() {
                    *s = IdleSwaySettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Aim sway", |ui| {
                let s = &mut *aim_sway;
                ui.label("reticle 'breathing' drift while aiming, scales up into ADS");
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.x, 0.0f32..=1.0)
                        .text("amplitude X (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.y, 0.0f32..=1.0)
                        .text("amplitude Y (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.x, 0.02f32..=1.0)
                        .text("frequency X (Hz)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.y, 0.02f32..=1.0)
                        .text("frequency Y (Hz)"),
                );
                if ui.button("Reset aim sway").clicked() {
                    *s = AimSwaySettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Crosshair", |ui| {
                let c = &mut *crosshair_cfg;

                ui.label("size on the glass");
                ui.add(
                    egui::Slider::new(&mut c.scale, 0.3f32..=3.0)
                        .text("scale  (>1 pushes the ends past the edge)"),
                );

                ui.label("aim-in drift — starts off-centre, slides to the middle");
                ui.add(
                    egui::Slider::new(&mut c.aim_in_frac.x, -1.5f32..=3.0)
                        .text("start X  (+ = left)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.aim_in_frac.y, -1.5f32..=3.0)
                        .text("start Y  (+ = up)"),
                );

                ui.label("turn lag — trails the aim a beat behind the gun");
                ui.add(
                    egui::Slider::new(&mut c.strength, 0.0f32..=0.3).text("strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.return_speed, 0.5f32..=12.0)
                        .text("catch-up speed  (lower = trails longer)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.max_offset_deg, 0.0f32..=6.0)
                        .text("max offset (° of scope view)"),
                );

                ui.checkbox(&mut c.center_dot_always, "keep centre dot on (no fade)");

                if ui.button("Reset crosshair").clicked() {
                    *c = CrosshairSettings::default();
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
                ui.add(
                    egui::Slider::new(&mut c.ads_scale, 0.0f32..=1.0).text(
                        "ADS scale — jitter + punch + shudder left at full ADS \
                         (ramps to full at the hip)",
                    ),
                );
                ui.separator();
                ui.label("view punch — rotates gun + cameras together (scaled by ADS scale)");
                ui.add(
                    egui::Slider::new(&mut c.view_punch_deg, 0.0f32..=12.0)
                        .text("view punch up (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.view_jitter_deg, 0.0f32..=6.0)
                        .text("view punch chaos (°)"),
                );
                ui.separator();
                ui.label(
                    "weapon shudder — gun kicks back toward the eye, muzzle climbs \
                     (scaled by ADS scale)",
                );
                ui.add(
                    egui::Slider::new(&mut c.weapon_kick, 0.0f32..=0.4)
                        .text("weapon kick back (m)"),
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
                         recoil_return {:.4}, ads_scale {:.4}",
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
                        c.ads_scale,
                    );
                }
                if ui.button("Reset camera shake").clicked() {
                    *c = ShakeSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("No-scope spread", |ui| {
                let n = &mut *noscope;
                ui.label("random up/down + L/R miss angle — wide at the hip, gone at full ADS");
                ui.add(
                    egui::Slider::new(&mut n.hip_max_deg, 0.0f32..=15.0)
                        .text("max miss at hip (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut n.curve, 1.0f32..=6.0)
                        .text("accuracy curve  (1 = linear, higher = tightens late)"),
                );
                ui.label(format!(
                    "cap now @ ads.t {:.2}: ±{:.2}°",
                    ads.t,
                    noscope_spread_angle(n, ads.t).to_degrees(),
                ));
                if ui.button("Reset no-scope spread").clicked() {
                    *n = NoScopeSpread::default();
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
    mut knife: ResMut<ThrowingKnife>,
    mut view_model: Query<(&ViewModelAnimation, &mut Visibility), With<ViewModel>>,
    mut players: Query<&mut AnimationPlayer, With<SniperAnimationPlayer>>,
) {
    *weapon = Weapon::default();
    *knife = ThrowingKnife::default();
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
            if cancelled || spd < cfg.min_speed || slide.timer >= cfg.max_time || !grounded {
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

/// Spawn one random footstep clip: a fresh pick that isn't an instant repeat, at
/// `volume`, with a small random pitch wobble so back-to-back steps differ.
fn play_footstep(
    commands: &mut Commands,
    clips: &[Handle<AudioSource>],
    state: &mut FootstepState,
    volume: f32,
    pitch_jitter: f32,
) {
    if clips.is_empty() {
        return;
    }
    state.seq = state.seq.wrapping_add(1);
    let s = state.seq.wrapping_mul(2_654_435_761).wrapping_add(0xf007);
    let mut idx = (rand01(s) * clips.len() as f32) as usize % clips.len();
    if clips.len() > 1 && idx == state.last {
        idx = (idx + 1) % clips.len();
    }
    state.last = idx;
    let pitch = 1.0 + (rand01(s ^ 0x5bd1_e995) * 2.0 - 1.0) * pitch_jitter;
    commands.spawn((
        AudioPlayer::new(clips[idx].clone()),
        PlaybackSettings::DESPAWN
            .with_volume(Volume::Linear(volume.max(0.0)))
            .with_speed(pitch.clamp(0.1, 4.0)),
    ));
}

/// Footstep cadence for the local player, Call-of-Duty style: a distance
/// accumulator releases a step every `stride` metres, so the pace tracks the
/// player's actual speed and each stance (crouch-walk / walk / sprint / prone)
/// gets its own stride length and loudness. No steps while airborne, sliding,
/// diving or standing still; the first step after a standstill fires at once.
#[allow(clippy::too_many_arguments)]
fn footsteps(
    time: Res<Time>,
    cfg: Res<FootstepSettings>,
    sounds: Res<GameSounds>,
    slide: Res<Slide>,
    sprinting: Res<Sprinting>,
    physics: Single<&PlayerPhysics, With<Player>>,
    mut state: ResMut<FootstepState>,
    mut commands: Commands,
) {
    let planar = Vec3::new(
        physics.horizontal_velocity.x,
        0.0,
        physics.horizontal_velocity.z,
    );
    let speed = planar.length();
    let on_foot = cfg.enabled
        && physics.grounded
        && !matches!(slide.stance, Stance::Sliding | Stance::Diving)
        && speed > cfg.min_speed;
    if !on_foot {
        state.accum = 0.0;
        state.was_moving = false;
        return;
    }

    let (stride, stance_vol) = match slide.stance {
        Stance::Crouching => (cfg.crouch_stride, cfg.crouch_volume),
        Stance::Prone => (cfg.prone_stride, cfg.prone_volume),
        _ if sprinting.0 => (cfg.sprint_stride, cfg.sprint_volume),
        _ => (cfg.walk_stride, cfg.walk_volume),
    };
    let stride = stride.max(0.1);
    let volume = (stance_vol * cfg.volume).max(0.0);

    if !state.was_moving {
        state.was_moving = true;
        state.accum = 0.0;
        play_footstep(
            &mut commands,
            &sounds.footsteps,
            &mut state,
            volume,
            cfg.pitch_jitter,
        );
        return;
    }

    state.accum += speed * time.delta_secs();
    // `while`, not `if`, so a big frame hitch at speed still spaces steps evenly
    // rather than dropping them; cap the catch-up so it can't spam on a stall.
    let mut budget = 4;
    while state.accum >= stride && budget > 0 {
        state.accum -= stride;
        budget -= 1;
        play_footstep(
            &mut commands,
            &sounds.footsteps,
            &mut state,
            volume,
            cfg.pitch_jitter,
        );
    }
    state.accum = state.accum.min(stride);
}

/// Snaps the player back to the current [`TeleportPoint`], restoring the
/// saved facing, and plays the teleport sound.
fn teleport_home(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    point: Res<TeleportPoint>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
    mut player: Single<(&mut Transform, &mut PlayerPhysics), (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    if binds.teleport_home.just_pressed(&keys, &mouse) {
        let (transform, physics) = &mut *player;
        transform.translation = point.position;
        transform.rotation = Quat::from_rotation_y(point.yaw);
        head.rotation = Quat::from_rotation_x(point.pitch);
        physics.horizontal_velocity = Vec3::ZERO;
        physics.vertical_velocity = 0.0;
        physics.grounded = true;
        commands.spawn((
            AudioPlayer::new(sounds.teleport.clone()),
            PlaybackSettings::DESPAWN,
        ));
    }
}

/// Reset the [`TeleportPoint`] to the player's current position and facing,
/// and flash a "Teleport point saved" toast in the centre of the screen.
fn save_teleport_point(
    (keys, mouse): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    binds: Res<KeyBindings>,
    asset_server: Res<AssetServer>,
    mut point: ResMut<TeleportPoint>,
    player: Single<&Transform, (With<Player>, Without<PlayerHead>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<Player>)>,
    existing: Query<Entity, With<TeleportToast>>,
    mut commands: Commands,
) {
    if !binds.save_teleport_point.just_pressed(&keys, &mouse) {
        return;
    }
    point.position = player.translation;
    point.yaw = player.rotation.to_euler(EulerRot::YXZ).0;
    point.pitch = head.rotation.to_euler(EulerRot::YXZ).1;

    for e in &existing {
        commands.entity(e).despawn();
    }
    commands
        .spawn((
            TeleportToast { age: 0.0 },
            StateScoped(AppState::InGame),
            GlobalZIndex(9),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(47.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_child((
            Text::new("Teleport point saved"),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 30.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// The centre-screen "Teleport point saved" message: held briefly, then faded.
#[derive(Component)]
struct TeleportToast {
    age: f32,
}

const TELEPORT_TOAST_HOLD: f32 = 1.0;
const TELEPORT_TOAST_TTL: f32 = 1.8;

/// Hold the toast, then fade it out and despawn — mirrors [`update_score_popups`].
fn update_teleport_toast(
    time: Res<Time>,
    mut toasts: Query<(Entity, &mut TeleportToast, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (entity, mut toast, children) in &mut toasts {
        toast.age += time.delta_secs();
        if toast.age >= TELEPORT_TOAST_TTL {
            commands.entity(entity).despawn();
            continue;
        }
        let a = if toast.age < TELEPORT_TOAST_HOLD {
            1.0
        } else {
            1.0 - (toast.age - TELEPORT_TOAST_HOLD) / (TELEPORT_TOAST_TTL - TELEPORT_TOAST_HOLD)
        };
        for child in children {
            if let Ok(mut tc) = texts.get_mut(*child) {
                tc.0 = Color::WHITE.with_alpha(a.clamp(0.0, 1.0));
            }
        }
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

/// Pull the player down and stop them on whichever surface is under them —
/// the basic-map cubes/ramps/bridge while over their footprint, otherwise the
/// ground. Walk off an edge and there's nothing under the feet, so the player
/// falls.
fn apply_gravity(
    time: Res<Time>,
    settings: Res<MovementSettings>,
    slide: Res<Slide>,
    map: Res<MapSettings>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let dt = time.delta_secs();
    let (mut transform, mut physics) = player.into_inner();
    let was_grounded = physics.grounded;

    physics.vertical_velocity -= settings.gravity * dt;

    let feet_now = transform.translation.y - EYE_HEIGHT;
    let feet_next = feet_now + physics.vertical_velocity * dt;

    // Only land on the map's surface from above/at its level — not when
    // walking through its base at ground height.
    let mut surface = 0.0f32;
    if let Some(h) = map_surface_height(transform.translation.x, transform.translation.z, &map) {
        if feet_now >= h - GROUND_SNAP {
            surface = surface.max(h);
        }
    }

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

    // Landing thump: airborne to grounded this frame. A dive lands on the
    // belly and already has its own sound (`SND_DIVE`), so it's excluded here.
    if !was_grounded && physics.grounded && !dive_landed {
        commands.spawn((
            AudioPlayer::new(sounds.jump_land.clone()),
            PlaybackSettings::DESPAWN,
        ));
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

    // Base sensitivity × the player's multiplier, eased toward the player's ADS
    // sensitivity multiplier as they zoom in.
    let sens = MOUSE_SENSITIVITY
        * settings.sensitivity
        * 1.0f32.lerp(settings.ads_sensitivity, ease(ads.t));

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
    let step = time.delta_secs() / (tuning.ads_duration_ms.max(1.0) / 1000.0);
    ads.t = if ads.t < target {
        (ads.t + step).min(target)
    } else {
        (ads.t - step).max(target)
    };
}

/// Blend the world-camera FOV and the view-model pose between hip and ADS.
/// Pure: the world camera's vertical FOV (radians) for a given hip FOV setting,
/// blended toward the ADS zoom by `ads_t`. Shared by the live [`apply_ads`]
/// system and the kill-cam replay, so a replay renders at the *shooter's* hip
/// FOV instead of the viewer's own.
pub(crate) fn ads_fov_rad(hip_fov_deg: f32, tuning: &AdsTuning, ads_t: f32) -> f32 {
    hip_fov_deg
        .to_radians()
        .lerp(tuning.fov_deg.to_radians(), ads_ease(ads_t, tuning.ads_ease))
}

/// How far the magnified scope picture has faded in: `0` until `Ads::t` reaches
/// `scope_picture_at`, easing to `1` by full ADS. Keeps the raise reading as a
/// plain zoom-and-lift instead of a second, pre-zoomed image of the target.
pub(crate) fn scope_picture_amount(ads_t: f32, tuning: &AdsTuning) -> f32 {
    let span = (1.0 - tuning.scope_picture_at).max(1e-3);
    ease(((ads_t - tuning.scope_picture_at) / span).clamp(0.0, 1.0))
}

pub(crate) fn apply_ads(
    ads: Res<Ads>,
    poses: Res<ViewModelPoses>,
    tuning: Res<AdsTuning>,
    settings: Res<Settings>,
    mut world_projection: Single<&mut Projection, With<WorldModelCamera>>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let e = ads_ease(ads.t, tuning.ads_ease);

    if let Projection::Perspective(perspective) = world_projection.as_mut() {
        perspective.fov = ads_fov_rad(settings.fov, &tuning, ads.t);
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
/// [`consume_look_delta`] clears this frame's turn once this and [`crosshair_sway`]
/// have read it.
fn weapon_sway(
    time: Res<Time>,
    tuning: Res<WeaponSwaySettings>,
    ads: Res<Ads>,
    look: Res<LookDelta>,
    mut state: ResMut<WeaponSwayState>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let dt = time.delta_secs().max(1e-5);
    let applied = look.applied;

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

/// Layer a slow procedural "breathing" drift onto the view model while the
/// player stands still — runs after [`weapon_sway`] and multiplies its own
/// offset on top of that one's. Blends toward zero the instant the player
/// moves (and back in once they settle) and fades out approaching full ADS,
/// where [`update_scope`]'s aim sway takes over instead.
fn idle_weapon_sway(
    time: Res<Time>,
    tuning: Res<IdleSwaySettings>,
    ads: Res<Ads>,
    player: Single<&PlayerPhysics, With<Player>>,
    mut state: ResMut<IdleSwayState>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let dt = time.delta_secs();
    let idle = player.grounded && player.horizontal_velocity.length() < 0.1;
    let target = if idle { 1.0 } else { 0.0 };
    let k = 1.0 - (-tuning.blend_speed * dt).exp();
    state.blend = state.blend.lerp(target, k);

    if state.blend > 1e-3 {
        state.clock += dt;
    }

    let amp = Vec2::new(
        tuning.amplitude_deg.x.to_radians(),
        tuning.amplitude_deg.y.to_radians(),
    ) * state.blend
        * (1.0 - ads.t.clamp(0.0, 1.0));
    let offset = breathing_offset(state.clock, tuning.frequency_hz, amp);

    let sway = Quat::from_euler(EulerRot::YXZ, offset.x, offset.y, 0.0);
    **view_model = Transform::from_rotation(sway) * **view_model;
}

/// Trail the scope reticle behind the player's aim while scoped, then let it
/// ease back to centre — see [`CrosshairSettings`]. Same maths as [`weapon_sway`]
/// on a softer spring and scaled by `Ads::t`, so the crosshair visibly lags the
/// gun model instead of moving locked to it. [`update_scope`] reads
/// [`CrosshairSwayState`] and offsets the reticle quad by it.
fn crosshair_sway(
    time: Res<Time>,
    tuning: Res<CrosshairSettings>,
    ads: Res<Ads>,
    look: Res<LookDelta>,
    mut state: ResMut<CrosshairSwayState>,
) {
    let dt = time.delta_secs().max(1e-5);
    let scoped = ads.t.clamp(0.0, 1.0);
    let max = tuning.max_offset_deg.to_radians();
    // `-applied / dt` is the view's angular velocity, opposite the turn.
    let target = (-look.applied / dt * tuning.strength * scoped)
        .clamp(Vec2::splat(-max), Vec2::splat(max));

    let k = 1.0 - (-tuning.return_speed * dt).exp();
    state.offset = state.offset.lerp(target, k);
}

/// Clear this frame's [`LookDelta`] after [`weapon_sway`] and [`crosshair_sway`]
/// have read it. `look_around` early-returns on a still frame without writing,
/// so without this the last turn would keep feeding the sways forever.
fn consume_look_delta(mut look: ResMut<LookDelta>) {
    look.applied = Vec2::ZERO;
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

/// Pushes `Settings::shadow_quality` onto the sun's shadow map whenever it
/// changes. Mirrors Call of Duty's "Shadow Map" option: Disabled turns shadows
/// off outright, and each tier up trades performance for resolution / cascade
/// count / draw distance.
fn apply_shadow_quality(
    settings: Res<Settings>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut sun: Single<(&mut DirectionalLight, &mut CascadeShadowConfig)>,
    mut applied: Local<Option<ShadowQuality>>,
) {
    if applied.is_some_and(|q| q == settings.shadow_quality) {
        return;
    }
    *applied = Some(settings.shadow_quality);

    let (light, cascades) = &mut *sun;
    let (enabled, size, num_cascades, maximum_distance) = match settings.shadow_quality {
        ShadowQuality::Disabled => (false, shadow_map.size, 1, 40.0),
        ShadowQuality::Low => (true, 512, 1, 40.0),
        ShadowQuality::Normal => (true, 1024, 2, 80.0),
        ShadowQuality::High => (true, 2048, 4, 120.0),
        ShadowQuality::Extra => (true, 4096, 4, 160.0),
    };

    light.shadows_enabled = enabled;
    shadow_map.size = size;
    **cascades = CascadeShadowConfigBuilder {
        num_cascades,
        first_cascade_far_bound: 20.0,
        maximum_distance,
        ..default()
    }
    .build();
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
/// keep its magnification in sync, slide the reticle in from the corner, and
/// fade the scope image in on the lens.
#[allow(clippy::type_complexity)]
fn update_scope(
    (ads, sway, crosshair, aim_sway_cfg, time, mut aim_sway): (
        Res<Ads>,
        Res<CrosshairSwayState>,
        Res<CrosshairSettings>,
        Res<AimSwaySettings>,
        Res<Time>,
        ResMut<AimSwayState>,
    ),
    tuning: Res<AdsTuning>,
    settings: Res<Settings>,
    scope_camera: Single<
        (&mut Camera, &mut Projection, &mut Transform),
        (With<ScopeCamera>, Without<ScopeReticle>),
    >,
    mut reticle: Single<&mut Transform, (With<ScopeReticle>, Without<ScopeCamera>)>,
    mut lens: Query<(&MeshMaterial3d<StandardMaterial>, &mut Visibility), With<ScopeLens>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let active = ads.t > SCOPE_SHOW_AT;

    // How far the magnified sight picture has come in: held off until the optic
    // is nearly centred on the eye, then ramped to full by `ads.t == 1`, so the
    // transition reads as a plain zoom-and-raise rather than a second,
    // wrongly-zoomed copy of the target sliding in.
    let picture = scope_picture_amount(ads.t, &tuning);
    // Follows the pose blend, so the drift below tracks the glass coming up.
    let e = ads_ease(ads.t, tuning.ads_ease);

    let (mut camera, mut projection, mut cam_transform) = scope_camera.into_inner();
    camera.is_active = active;
    // Before the picture comes in, keep the scope camera at the world FOV so the
    // render target matches the view *behind* the glass 1:1; converge to the
    // real scope magnification as the picture arrives.
    let world_fov = ads_fov_rad(settings.fov, &tuning, ads.t);
    let scope_fov = world_fov.lerp(tuning.scope_fov_deg.to_radians(), picture);
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = scope_fov;
    }

    // CoD-style aim-in drift: at the hip the reticle starts toward a corner
    // (`aim_in_frac` of the scope's half-view) and the scope camera looks that
    // way too, so the point the eye is already aiming at stays pinned under the
    // reticle while the glass slides up into it. Eases to zero by full ADS.
    let half_fov = scope_fov * 0.5;
    let aim_in = Vec2::new(
        half_fov * crosshair.aim_in_frac.x,
        half_fov * crosshair.aim_in_frac.y,
    ) * (1.0 - e);
    cam_transform.rotation = Quat::from_euler(EulerRot::YXZ, -aim_in.x, -aim_in.y, 0.0);

    // "Aiming idle sway" — a slow breathing drift on the reticle, scaled up
    // toward full ADS (the opposite fade direction of `IdleSwaySettings`'s
    // weapon-model sway, which hands off to this one as ADS comes up).
    aim_sway.clock += time.delta_secs();
    let aim_amp = Vec2::new(
        aim_sway_cfg.amplitude_deg.x.to_radians(),
        aim_sway_cfg.amplitude_deg.y.to_radians(),
    ) * e;
    let breathing = breathing_offset(aim_sway.clock, aim_sway_cfg.frequency_hz, aim_amp);

    // Fill the scope camera's square view (`scale` lets the crosshair art run
    // past the glass edge), then offset the whole reticle: the aim-in drift,
    // `crosshair_sway`'s turn lag (which trails a beat behind the gun model),
    // and the aim-sway breathing drift above.
    // Rotating the quad about the camera mirrors how `weapon_sway` moves the gun.
    let fill = 2.0 * RETICLE_DIST * half_fov.tan() * crosshair.scale.max(0.01);
    let offset = sway.offset + aim_in + breathing;
    let drift = Quat::from_euler(EulerRot::YXZ, offset.x, offset.y, 0.0);
    **reticle = Transform::from_rotation(drift)
        * Transform {
            translation: Vec3::new(0.0, 0.0, -RETICLE_DIST),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(fill, fill, 1.0),
        };

    // The lens is always drawn: a reflective glass disc at the hip, the sight
    // picture while scoped. `e` crossfades between the two looks, tied to the
    // picture ramp so the glass stays mirror-like until the picture is due.
    let e = picture;
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
            // Dial the mirror sheen out as the player scopes in — reflectance to
            // zero at full ADS so the sun leaves no glint on the glass.
            material.perceptual_roughness = LENS_ROUGHNESS_HIP.lerp(LENS_ROUGHNESS_ADS, e);
            material.metallic = LENS_METALLIC_HIP * k;
            material.reflectance = LENS_REFLECTANCE_HIP * k;
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
            let fleck_amt = if speck > 0.86 {
                (speck - 0.86) / 0.14
            } else {
                0.0
            };

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

/// Fire, reload and weapon-swap (bindings). Firing spends a round and plays
/// Shoot → Rechamber to cycle the bolt. The shot that empties the mag leaves
/// the spent case sitting in the chamber (nothing left in the mag to cycle
/// into it) and, since the Reload clip only swaps the magazine and never
/// touches the bolt, that reload always ends with a Rechamber to load the
/// first round of the fresh mag — either right away (auto-reload) or once a
/// manual reload is pressed. Reloading with a round already chambered (mag
/// still has rounds) skips that trailing Rechamber; refilling the mag/reserve
/// counters happens once the whole queue finishes. Swapping to the secondary
/// plays Hide in full then drops the sniper model;
/// swapping back plays Show in full and restarts any reload / rechamber the swap
/// cut short. Nothing new is accepted while an animation is mid-play (except the
/// swap key), so shots are impossible until a reload finishes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
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
        Query<&mut AnimationPlayer, With<SniperAnimationPlayer>>,
    ),
    mut weapon: ResMut<Weapon>,
    mut knife: ResMut<ThrowingKnife>,
    mut pending_shot: ResMut<PendingShot>,
    mut shake: ResMut<Shake>,
    mut muzzle: ResMut<MuzzleFlashState>,
    mut smoke: ResMut<SmokeEmission>,
    mut shots: EventWriter<practice::LocalShot>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    (shake_cfg, sounds, anim, ads, spread_cfg, settings): (
        Res<ShakeSettings>,
        Res<GameSounds>,
        Res<AnimationSettings>,
        Res<Ads>,
        Res<NoScopeSpread>,
        Res<Settings>,
    ),
    mut commands: Commands,
) {
    let node = view_model.index;
    let Some(mut player) = players.iter_mut().next() else {
        return;
    };
    let locked = window.cursor_options.grab_mode != CursorGrabMode::None;

    // Throwing knife — a separate, instant hide/show layered over whatever's
    // equipped (doesn't touch `weapon.slot`). Pressing it snaps the sniper away
    // with no Hide animation, so a shot's rechamber can be cut off for a
    // "silent" quickscope; releasing it (or pressing swap-weapon while it's
    // held) draws the sniper back out with the normal Show animation and
    // resumes whatever the hold interrupted.
    if locked && binds.throwing_knife.just_pressed(&keys, &mouse) && !knife.active {
        knife.active = true;
        if weapon.slot == WeaponSlot::Primary {
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
            if let Some(active_anim) = player.animation_mut(node) {
                active_anim.seek_to(0.0);
                active_anim.pause();
            }
            **view_model_vis = Visibility::Hidden;
        }
        return;
    }
    if knife.active {
        let release = !binds.throwing_knife.pressed(&keys, &mouse)
            || binds.swap_weapon.just_pressed(&keys, &mouse);
        if release {
            knife.active = false;
            if weapon.slot == WeaponSlot::Primary {
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
                } else if next.name == SEGMENTS[SEG_RELOAD].name {
                    // Auto-reload rolling straight out of the Shoot segment.
                    commands.spawn((
                        AudioPlayer::new(sounds.reload.clone()),
                        PlaybackSettings::DESPAWN,
                        WeaponActionSound,
                    ));
                    snd.note(killcam::SND_RELOAD);
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

        // Aim direction, thrown off the crosshair by no-scope inaccuracy: a
        // random up/down + left/right angle, each up to `noscope_spread_angle`
        // (wide at the hip, zero at full ADS). `write_input` and
        // `resolve_local_shot` both take this exact ray so local feedback and
        // the server hit agree.
        let cam_gt = cam.single().ok();
        let dir = cam_gt
            .map(|cam| {
                let max = noscope_spread_angle(&spread_cfg, ads.t);
                let yaw = (rand01(muzzle.shots.wrapping_mul(0x9E37_79B9)) * 2.0 - 1.0) * max;
                let pitch =
                    (rand01(muzzle.shots.wrapping_mul(0x85EB_CA6B) ^ 0xDEAD_BEEF) * 2.0 - 1.0) * max;
                cam.rotation() * Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z
            })
            .unwrap_or(Vec3::NEG_Z);
        pending_shot.0 = Some(dir); // net::write_input turns this into a fire request

        // Hand the shot ray to `practice::resolve_local_shot`: it kicks up the
        // ground dust locally (instant, and the only path in solo Practice) and,
        // in Practice, resolves the hit + scoring against the offline bots. In a
        // real game the server also broadcasts this shot; `net::receive_shots`
        // drops the echo for our own peer so nothing double-spawns.
        if let Some(cam) = cam_gt {
            shots.write(practice::LocalShot {
                origin: cam.translation(),
                dir,
            });
        }
        play_segment(&mut player, node, SEGMENTS[SEG_SHOOT]);

        // Bolt-action cycle. After a shot that leaves rounds in the mag, work
        // the bolt (Rechamber) to eject the spent case and feed the next round.
        // The shot that empties the mag leaves the spent case sitting in the
        // chamber — the Reload clip only swaps the magazine, it never touches
        // the bolt — so with auto-reload on we go Shoot → Reload → Rechamber,
        // working the bolt only once the fresh mag is seated (that one motion
        // both ejects the old case and feeds the first round of the new mag).
        // With auto-reload off the sniper just holds on the fired pose until a
        // manual reload.
        let (remaining, on_finish) = if weapon.mag > 0 {
            (
                vec![SEGMENTS[SEG_SHOOT], SEGMENTS[SEG_RECHAMBER]],
                WeaponFinish::Nothing,
            )
        } else if settings.auto_reload && weapon.reserve > 0 {
            (
                vec![
                    SEGMENTS[SEG_SHOOT],
                    SEGMENTS[SEG_RELOAD],
                    SEGMENTS[SEG_RECHAMBER],
                ],
                WeaponFinish::Reload,
            )
        } else {
            (vec![SEGMENTS[SEG_SHOOT]], WeaponFinish::Nothing)
        };
        weapon.busy = Some(WeaponBusy {
            remaining,
            seg_end: SEGMENTS[SEG_SHOOT].end_secs(),
            on_finish,
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
        // `mag == 0` only when the last round was fired and never rechambered
        // (there's no separate "round chambered" flag — an empty mag is the
        // one moment the chamber is guaranteed empty too), so the bolt still
        // needs working after the fresh mag goes in. Reloading with a round
        // already chambered (mag > 0, the TEMP full-mag case above) is a
        // tactical swap — the chamber's already loaded, so no bolt work.
        let mut remaining = vec![SEGMENTS[SEG_RELOAD]];
        if weapon.mag == 0 {
            remaining.push(SEGMENTS[SEG_RECHAMBER]);
        }
        weapon.busy = Some(WeaponBusy {
            remaining,
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
                    tint: [1.0, 1.0, 1.0],
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
                    tint: [1.0, 1.0, 1.0],
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

/// Squirt a short blood burst out of a bot the instant a shot connects, along
/// the shot's travel direction from the hit point. World-space, so every client
/// and camera angle sees it. Mirrors `spawn_ground_impact`.
fn spawn_blood_impact(
    mut events: EventReader<BloodImpact>,
    assets: Res<ImpactAssets>,
    blood: Res<BloodSettings>,
    existing: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let mut budget = IMPACT_MAX.saturating_sub(existing.iter().count());

    for ev in events.read() {
        let jet = ev.dir.normalize_or(Vec3::NEG_Y);
        *seq = seq.wrapping_add(1);

        for i in 0..blood.count {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let s = seq
                .wrapping_mul(2_246_822_519)
                .wrapping_add(i.wrapping_mul(83_492_791))
                .wrapping_add(0xb100d);
            let dir = cone_dir(jet, blood.spread_deg.to_radians(), s);
            let speed = blood.speed * (0.35 + 0.65 * rand01(s ^ 0x9e37));
            // `blood-splatter-texture.png` is one wide field of droplets on
            // transparent; show a small random window into it per particle so
            // each droplet sprite is a handful of specks, not the whole field.
            // Bias toward the dense centre-left so a window is never empty.
            let win = Vec2::new(0.16, 0.26);
            let uv_off = Vec2::new(
                0.10 + rand01(s ^ 0x3).powf(1.5) * (0.62 - win.x),
                0.05 + rand01(s ^ 0x5) * (0.88 - win.y),
            );
            let mut material = impact_material(assets.blood.clone());
            material.uv_transform = Affine2::from_scale_angle_translation(win, 0.0, uv_off);
            let scale0 = (blood.scale * (0.6 + 0.8 * rand01(s ^ 0x2c))).max(1.0e-4);
            commands.spawn((
                StateScoped(AppState::InGame),
                ImpactParticle {
                    velocity: dir * speed,
                    gravity: blood.gravity,
                    drag: blood.drag,
                    age: 0.0,
                    lifetime: blood.lifetime.max(0.1) * (0.7 + 0.6 * rand01(s ^ 0x1234)),
                    fade_in: 0.0,
                    roll: rand_roll(s ^ 0x77),
                    spin: 0.0,
                    scale0,
                    scale1: scale0 * blood.growth.max(0.1),
                    peak_alpha: blood.opacity,
                    tint: blood.color,
                },
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(material)),
                Transform::from_translation(ev.point).with_scale(Vec3::splat(scale0)),
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
            let [tr, tg, tb] = p.tint;
            m.base_color = Color::srgba(tr, tg, tb, p.peak_alpha * envelope.clamp(0.0, 1.0));
        }
    }
}

/// Spawn a streak for each [`FireTracer`] this frame: a thin cylinder spanning
/// `start -> end`, starting in the bright additive "flash" look — `update_tracers`
/// carries it into the smoke phase.
fn spawn_tracers(
    mut events: EventReader<FireTracer>,
    assets: Res<TracerAssets>,
    settings: Res<TracerSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for ev in events.read() {
        let delta = ev.end - ev.start;
        let len = delta.length();
        if len < 0.05 {
            continue; // too short to read as a streak (e.g. an instant self-hit)
        }
        let dir = delta / len;
        let mid = ev.start + delta * 0.5;
        let flash = color_from_parts(settings.flash_color);
        let material = materials.add(StandardMaterial {
            base_color: flash,
            emissive: LinearRgba::from(flash) * settings.flash_emissive_boost,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        commands.spawn((
            Tracer { age: 0.0 },
            StateScoped(AppState::InGame),
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(material),
            Transform {
                translation: mid,
                rotation: Quat::from_rotation_arc(Vec3::Y, dir),
                scale: Vec3::new(
                    settings.flash_radius * 2.0,
                    len,
                    settings.flash_radius * 2.0,
                ),
            },
            NotShadowCaster,
        ));
    }
}

/// Drive each tracer through its two phases — a steady bright flash, then a
/// smoke trail that widens and fades to nothing — then despawn it and free
/// its (per-instance) material.
fn update_tracers(
    time: Res<Time>,
    settings: Res<TracerSettings>,
    mut tracers: Query<(
        Entity,
        &mut Tracer,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let flash_secs = settings.flash_secs.max(0.0);
    let smoke_secs = settings.smoke_secs.max(1.0e-4);
    let total = flash_secs + smoke_secs;

    for (entity, mut tracer, mut transform, material) in &mut tracers {
        tracer.age += dt;
        if tracer.age >= total {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        let Some(m) = materials.get_mut(&material.0) else {
            continue;
        };
        if tracer.age < flash_secs {
            let flash = color_from_parts(settings.flash_color);
            m.alpha_mode = AlphaMode::Add;
            m.base_color = flash;
            m.emissive = LinearRgba::from(flash) * settings.flash_emissive_boost;
            transform.scale.x = settings.flash_radius * 2.0;
            transform.scale.z = settings.flash_radius * 2.0;
        } else {
            let t = ((tracer.age - flash_secs) / smoke_secs).clamp(0.0, 1.0);
            let smoke = color_from_parts(settings.smoke_color);
            m.alpha_mode = AlphaMode::Blend;
            m.emissive = LinearRgba::BLACK;
            m.base_color = smoke.with_alpha(settings.smoke_start_alpha * (1.0 - t));
            let radius = settings.flash_radius.lerp(settings.smoke_radius, t);
            transform.scale.x = radius * 2.0;
            transform.scale.z = radius * 2.0;
        }
    }
}

/// The one multiplier that scales *every* per-shot shake — camera positional
/// jitter, view punch, and the weapon shudder — down as the player scopes in:
/// `1.0` at the hip, ramping linearly to `cfg.ads_scale` at full ADS.
pub(crate) fn shake_ads_scale(cfg: &ShakeSettings, ads_t: f32) -> f32 {
    1.0 - (1.0 - cfg.ads_scale.clamp(0.0, 1.0)) * ads_t.clamp(0.0, 1.0)
}

/// Pure: the `CameraShake` node's local transform for a given shake state — the
/// positional jitter and view punch — a directional pitch-up plus rotational
/// chaos, both scaled by `trauma` / `trauma²` and eased down while scoped. At
/// rest (`trauma <= 0`) this is exactly identity.
///
/// Shared by the live [`camera_shake`] system and the kill-cam replay
/// (`killcam::drive_killcam`), so a replayed `(trauma, phase)` reproduces
/// on-screen exactly what the shooter saw.
pub(crate) fn shake_camera_pose(
    cfg: &ShakeSettings,
    ads_t: f32,
    trauma: f32,
    phase: f32,
) -> Transform {
    if trauma <= 0.0 {
        return Transform::IDENTITY;
    }
    let s = phase;
    let amt = trauma * trauma;

    // Scale the whole shake down as the player scopes in, so ADS stays
    // controllable while the hip still kicks hard (see [`shake_ads_scale`]).
    let ads_scale = shake_ads_scale(cfg, ads_t);

    // Positional jitter: up / down + side / side, no Z.
    let translation = Vec3::new(
        (s * 1.53 + 0.4).sin() * cfg.pos_max * amt * ads_scale,
        (s * 1.19 + 3.3).sin() * cfg.pos_max * amt * ads_scale,
        0.0,
    );

    // View punch: a directional pitch-up that recovers with `trauma`, plus
    // rotational chaos (`trauma²`) on top. +X rotation looks up.
    let punch = trauma * cfg.view_punch_deg.to_radians() * ads_scale;
    let jitter = cfg.view_jitter_deg.to_radians() * amt * ads_scale;
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        (s * 0.91 + 1.7).sin() * jitter,       // yaw
        punch + (s * 0.63).sin() * jitter,     // pitch (up)
        (s * 1.27 + 2.1).sin() * jitter * 0.6, // roll
    );

    Transform {
        translation,
        rotation,
        scale: Vec3::ONE,
    }
}

/// Rebuild the shake nodes' local transforms each frame from the current state,
/// always from zero, so they settle back to exactly identity and never drift aim.
///
/// * `CameraShake` (gun + cameras): the positional jitter and view punch from
///   [`shake_camera_pose`]. Rotating this node turns the gun and the cameras as
///   one, so the gun stays screen-locked while the world swings.
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
    if shake.trauma > 0.0 {
        shake.phase += dt * cfg.frequency;
    } else {
        shake.phase = 0.0;
    }

    let want = shake_camera_pose(&cfg, ads.t, shake.trauma, shake.phase);
    if **rig != want {
        **rig = want;
    }
}

/// A short, sharp shudder layered onto the view model on top of the sway pose:
/// the gun punches back toward the eye and the muzzle climbs, then settles as
/// `Shake::trauma` decays. Separate from the view punch in [`camera_shake`] —
/// this moves the gun *relative* to the camera, the way a CoD weapon recoils,
/// and never touches aim. Pre-multiplied in camera space (+Z toward the eye,
/// +Y up), so it is independent of the view model's own orientation.
/// Pure: the shudder kick premultiplied onto the view model — the gun punches
/// back toward the eye and the muzzle climbs, scaled by `trauma` and by the same
/// [`shake_ads_scale`] as the camera shake. Shared by the live
/// [`weapon_recoil_shudder`] system and the kill-cam replay.
pub(crate) fn weapon_kick_pose(
    cfg: &ShakeSettings,
    ads_t: f32,
    trauma: f32,
    phase: f32,
) -> Transform {
    if trauma <= 0.0 {
        return Transform::IDENTITY;
    }
    let amt = trauma * trauma;
    let s = phase;
    let ads_scale = shake_ads_scale(cfg, ads_t);

    let back = amt * cfg.weapon_kick * ads_scale;
    let rise = amt * cfg.weapon_kick * 0.5 * ads_scale;
    let wobble = (s * 0.8).sin() * cfg.weapon_kick * 0.25 * amt * ads_scale;

    Transform {
        translation: Vec3::new(wobble, rise, back),
        rotation: Quat::from_rotation_x(trauma * cfg.weapon_kick_deg.to_radians() * ads_scale),
        scale: Vec3::ONE,
    }
}

fn weapon_recoil_shudder(
    cfg: Res<ShakeSettings>,
    ads: Res<Ads>,
    shake: Res<Shake>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let kick = weapon_kick_pose(&cfg, ads.t, shake.trauma, shake.phase);
    **view_model = kick * **view_model;
}

/// Keep the bottom-right readout in sync with the ammo counts.
fn update_ammo_ui(weapon: Res<Weapon>, mut text: Single<&mut Text, With<AmmoText>>) {
    let wanted = format!("{} / {}", weapon.mag, weapon.reserve);
    if text.0 != wanted {
        text.0 = wanted;
    }
}
