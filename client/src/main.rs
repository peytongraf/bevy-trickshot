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

mod changelog;
mod environment;
mod keybinds;
mod killcam;
mod lobby_ui;
mod menu;
mod net;
mod player;
mod practice;
mod settings;
mod ui;
mod updater;
mod util;
mod weapons;

use environment::*;
use player::*;
use util::{color_from_parts, rand01, rand_roll, srgb_parts};
use weapons::*;

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
use std::time::Duration;

use keybinds::KeyBindings;
use settings::{Settings, ShadowQuality};

use bevy::{
    animation::{prelude::AnimationTransitions, RepeatAnimation},
    audio::{SpatialListener, Volume},
    core_pipeline::bloom::Bloom,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
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
use bevy_rapier3d::prelude::*;

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

/// Bold condensed display face used across the in-game HUD (score, ammo, fps,
/// score popups) and the kill-cam banner — the closest free/open stand-in for
/// the tall all-caps look modern Call of Duty titles use for this kind of text.
pub(crate) const HUD_FONT: &str = "fonts/BebasNeue-Regular.ttf";

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
        // Collision only (see the `bevy_rapier3d` dependency comment) — no
        // `RigidBody` is ever spawned, so `PhysicsSet::StepSimulation` has
        // nothing to integrate each frame, just static colliders to query.
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
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
        .init_resource::<KnifeViewModelSettings>()
        .init_resource::<Ads>()
        .init_resource::<AdsTuning>()
        .init_resource::<LookDelta>()
        .init_resource::<WeaponSwayState>()
        .init_resource::<WeaponSwaySettings>()
        .init_resource::<IdleSwayState>()
        .init_resource::<IdleSwaySettings>()
        .init_resource::<AimSwayState>()
        .init_resource::<AimSwaySettings>()
        .init_resource::<CrosshairSettings>()
        .init_resource::<TeleportPoint>()
        .init_resource::<NoScopeSpread>()
        .init_resource::<Weapon>()
        .init_resource::<ThrowingKnife>()
        .init_resource::<KnifeAnimState>()
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
        .init_resource::<Jumping>()
        .init_resource::<Slide>()
        .init_resource::<SlideSettings>()
        .init_resource::<FootstepSettings>()
        .init_resource::<FootstepState>()
        .init_resource::<SoundVolumes>()
        .init_resource::<SceneTuning>()
        .init_resource::<ShipmentSceneTuning>()
        .init_resource::<MapSettings>()
        .init_resource::<CurrentMap>()
        .init_resource::<ShipmentSettings>()
        .init_resource::<WaterSettings>()
        .init_resource::<ShipmentLightSettings>()
        .init_resource::<FluoroLightSettings>()
        .init_resource::<BulbLightSettings>()
        .init_resource::<RainSettings>()
        .init_resource::<RemoteAvatarSettings>()
        .init_resource::<RemoteSoundSettings>()
        .init_resource::<SoldierAnimSettings>()
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
                setup_soldier_assets,
                setup_rain,
            ),
        )
        .add_systems(
            OnEnter(AppState::InGame),
            (
                grab_cursor,
                start_ambient.after(lobby_ui::sync_current_map),
                reset_slide,
                reset_trick,
                reset_weapon,
            ),
        )
        .add_systems(OnEnter(AppState::MainMenu), release_cursor)
        // `InLobby` never used to be reachable straight from `InGame` (only
        // leaving early went through `MainMenu`), so this had no counterpart
        // until the match-results screen's CONTINUE button started calling
        // `next.set(AppState::InLobby)` directly. Without it the cursor stays
        // grabbed: `menu::cursor_and_hud` only reacts to `Menu` changes, and
        // on the frame CONTINUE fires, `AppState` is still `InGame` (state
        // transitions apply at end-of-frame) while the menu just closed, so
        // it explicitly re-grabs — then never runs again since nothing else
        // touches `Menu` afterward.
        .add_systems(OnEnter(AppState::InLobby), release_cursor)
        .add_systems(Update, (hud_visibility, crosshair_root_visibility))
        .add_systems(Update, apply_master_volume)
        // Unconditional (not gated on `AppState`) so the map (and its ground)
        // behind the menu/lobby UI is already right the instant a game starts.
        .add_systems(
            Update,
            (
                sync_map_model,
                sync_shipment_only_visibility,
                sync_sky_texture,
            ),
        )
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
                    resolve_wall_collisions,
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
                update_scope.after(aim_idle_sway),
                (fade_crosshair, update_crosshair_visibility),
                // Must read `LookDelta` before `consume_look_delta` (registered
                // in a separate `add_systems` below) zeroes it for the frame.
                track_trick
                    .after(look_around)
                    .before(consume_look_delta)
                    .run_if(killcam::no_killcam),
                (
                    spawn_score_popup,
                    update_score_popups,
                    update_teleport_toast,
                ),
                (sky_follow_camera, scroll_water_normal),
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
                (
                    apply_map_transform,
                    apply_shipment_transform,
                    apply_water_settings,
                    apply_shipment_lights,
                    sync_light_marker_visibility,
                    apply_fluoro_light,
                    sync_fluoro_marker_visibility,
                    apply_bulb_lights,
                    sync_bulb_marker_visibility,
                    update_rain,
                    apply_rain_assets,
                    apply_knife_transform,
                ),
                debug_cursor_toggle,
            )
                .after(update_ads)
                .run_if(in_state(AppState::InGame)),
        )
        // Weapon sway rides on top of the ADS pose, using this frame's turn;
        // idle sway then layers a "breathing" drift on top while the player's
        // still, and the recoil shudder rides on top of both — all cosmetic,
        // the view model only. `aim_idle_sway` is the odd one out: it rotates
        // the *real* `WorldModelCamera` (see its doc comment), which
        // `update_scope` then reads to keep the scope camera's picture in
        // lockstep. All four stop during a kill cam — `killcam::drive_killcam`
        // reproduces the cosmetic ones from the recorded sway / shake state
        // instead (so replay isn't stacked on top of these reading live,
        // frozen state), and a viewer's own live aim sway must never leak into
        // someone else's replay.
        .add_systems(
            Update,
            (
                (weapon_sway, idle_weapon_sway, weapon_recoil_shudder).chain(),
                aim_idle_sway,
            )
                .after(look_around)
                .after(apply_ads)
                .run_if(in_state(AppState::InGame).and(killcam::no_killcam)),
        )
        // `consume_look_delta` clears this frame's turn once `weapon_sway` and
        // `track_trick` (ordered `.before(consume_look_delta)` above) have had
        // it — `look_around` early-returns on a still frame without clearing
        // it itself.
        .add_systems(
            Update,
            consume_look_delta
                .after(weapon_sway)
                .run_if(in_state(AppState::InGame)),
        )
        .run();
}

// ---------------------------------------------------------------------------
// Components / resources
// ---------------------------------------------------------------------------

/// `basic_map.glb`'s daytime look, live-tweakable from the debug panel's
/// "Fog & Sky (Basic Map)" section and pushed onto the fog / sun / ambient /
/// bloom by `apply_scene_tuning` whenever that's the selected map — see
/// [`ShipmentSceneTuning`] for `Shipment`'s (separately tunable) look.
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

/// `shipment.glb`'s look — same shape as [`SceneTuning`] (see that struct's
/// fields), just a separate resource so `Shipment` can be tuned to its own
/// MW3-style setting (dark, foggy, overcast — near dawn/dusk under heavy
/// cloud, out on open water) without touching `basic_map.glb`'s daytime one.
/// Live-tweakable from the debug panel's "Fog & Sky (Shipment)" section;
/// `apply_scene_tuning` pushes whichever of the two is currently selected
/// ([`CurrentMap`]) onto the shared fog / sun / ambient / bloom.
#[derive(Resource)]
struct ShipmentSceneTuning(SceneTuning);

impl Default for ShipmentSceneTuning {
    fn default() -> Self {
        Self(SceneTuning {
            fog_visibility_m: 200.0,
            // r24 g30 b37 (0-255) — a dark, cool overcast grey.
            fog_color: srgb_parts(Color::srgb(24.0 / 255.0, 30.0 / 255.0, 37.0 / 255.0)),
            fog_sun_exponent: 7.0,
            sun_lux: 1000.0,
            sun_color: srgb_parts(Color::srgb(0.75, 0.78, 0.85)),
            ambient_color: srgb_parts(Color::srgb(0.35, 0.38, 0.42)),
            ambient_lux: 50.0,
            bloom_intensity: 0.1,
        })
    }
}

/// On every target-bot visual (practice-local *and* networked avatars), so the
/// kill cam can hide the live bots and show its own frozen snapshot instead.
#[derive(Component)]
pub(crate) struct TargetBotVisual;

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

/// Tags a remote player's `SceneRoot` entity (`models/soldier.glb`) so
/// `start_soldier_animation` can tell it apart from any other spawned scene.
#[derive(Component)]
pub(crate) struct SoldierVisual;

/// Which of `models/soldier.glb`'s movement/aim clips a remote avatar should
/// be playing, picked from its interpolated pose's frame-to-frame speed and
/// `ads_t` by `net::animate_remote_avatars`. `AimWalk`/`AimSprint` both play
/// `runAndShooting` — there's no separate walking-while-aiming clip — split
/// out only so each can get its own playback speed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum SoldierAnimState {
    #[default]
    Idle,
    Walk,
    Sprint,
    /// Aiming, not moving: `shooting`, looped.
    Aim,
    /// Aiming while walking: `runAndShooting`, looped at the walk speed.
    AimWalk,
    /// Aiming while sprinting: `runAndShooting`, looped at the sprint speed.
    AimSprint,
    /// Crouched, not moving: `crouch`, looped.
    Crouch,
    /// Crouch-walking: `crouchWalk`, looped.
    CrouchWalk,
    /// Reloading: `reload`, played once — takes priority over every other
    /// state above, and drops back to whichever of those applies once
    /// `pose.reloading` clears.
    Reload,
    /// Airborne from a jump: `jump`, played once at launch and held on its
    /// last frame for the rest of the jump arc (`pose.jumping` stays true
    /// until landing) — takes priority over everything, including `Reload`.
    Jump,
    /// Strafing right with no forward/back component: `strafeRight`, looped.
    StrafeRight,
    /// Strafing left with no forward/back component: `strafeLeft`, looped.
    StrafeLeft,
    /// Moving backward with no strafe component: `backpaddle`, looped.
    Backward,
    /// Dead (`pose.alive == false`): `death`, played once and held on its
    /// last frame (the model falling onto its back) until `pose.alive` goes
    /// back to `true` on respawn — outranks every other state, including
    /// `Jump`.
    Dead,
}

/// Graph + node indices for `models/soldier.glb`'s `idleWgun` / `walk` /
/// `run` / `shooting` / `runAndShooting` / `crouch` / `crouchWalk` / `reload`
/// / `jump` / `strafeRight` / `strafeLeft` / `backpaddle` / `death` clips —
/// 13 of its 18 animations wired up so far. Built once at startup.
#[derive(Resource, Clone)]
pub(crate) struct SoldierAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    sprint: AnimationNodeIndex,
    aim_idle: AnimationNodeIndex,
    aim_move: AnimationNodeIndex,
    crouch: AnimationNodeIndex,
    crouch_walk: AnimationNodeIndex,
    reload: AnimationNodeIndex,
    jump: AnimationNodeIndex,
    strafe_right: AnimationNodeIndex,
    strafe_left: AnimationNodeIndex,
    backward: AnimationNodeIndex,
    death: AnimationNodeIndex,
}

impl SoldierAnimations {
    pub(crate) fn node_for(&self, state: SoldierAnimState) -> AnimationNodeIndex {
        match state {
            SoldierAnimState::Idle => self.idle,
            SoldierAnimState::Walk => self.walk,
            SoldierAnimState::Sprint => self.sprint,
            SoldierAnimState::Aim => self.aim_idle,
            SoldierAnimState::AimWalk | SoldierAnimState::AimSprint => self.aim_move,
            SoldierAnimState::Crouch => self.crouch,
            SoldierAnimState::CrouchWalk => self.crouch_walk,
            SoldierAnimState::Reload => self.reload,
            SoldierAnimState::Jump => self.jump,
            SoldierAnimState::StrafeRight => self.strafe_right,
            SoldierAnimState::StrafeLeft => self.strafe_left,
            SoldierAnimState::Backward => self.backward,
            SoldierAnimState::Dead => self.death,
        }
    }
}

/// Playback-speed multipliers for the remote-player walk/sprint/aim clips,
/// calibrated for a remote player moving at exactly the default
/// `WALK_SPEED`/`SPRINT_SPEED` (hand-tuned so the feet don't slide). Exposed
/// on the "Remote players" debug-panel section for re-tuning if the clips
/// themselves change; `net::animate_remote_avatars` scales these by how far
/// `MovementSettings.walk_speed`/`sprint_speed` (the "Movement" panel's
/// player-speed controls) have been moved away from those defaults, so
/// dialing player speed up or down keeps the feet matching the ground
/// without needing separate, redundant animation-speed sliders.
///
/// `runAndShooting` has no walking-only counterpart, so `base_aim_walk_speed`
/// and `base_aim_sprint_speed` both drive that same clip — split into two
/// settings only because it needs a different speed depending on whether the
/// remote player is walking or sprinting while aiming.
///
/// `base_crouch_walk_speed` is calibrated against `SlideSettings.crouch_speed`
/// (the "Slide" panel's crouch-move-speed control) the same way the others
/// are calibrated against `WALK_SPEED`/`SPRINT_SPEED`.
///
/// `base_strafe_speed`/`base_backpaddle_speed` are calibrated against the
/// *effective* strafe/backward speed at the default settings — i.e.
/// `WALK_SPEED * STRAFE_SPEED_MULT` / `WALK_SPEED * BACKWARD_SPEED_MULT` —
/// since `strafeRight`/`strafeLeft`/`backpaddle` only ever play at walk pace
/// (no separate sprint-strafe clip exists), scaled by how far
/// `MovementSettings.strafe_speed_mult`/`backward_speed_mult` (the
/// "Movement" panel's controls) have moved away from their defaults.
///
/// `death_speed` isn't calibrated against anything — it's a flat playback
/// speed multiplier for the `death` clip (`1.0` = authored speed), since
/// there's no real-world reference like a movement speed to match it to.
#[derive(Resource, Clone, Copy)]
pub(crate) struct SoldierAnimSettings {
    pub(crate) base_walk_speed: f32,
    pub(crate) base_sprint_speed: f32,
    pub(crate) base_aim_walk_speed: f32,
    pub(crate) base_aim_sprint_speed: f32,
    pub(crate) base_crouch_walk_speed: f32,
    pub(crate) base_strafe_speed: f32,
    pub(crate) base_backpaddle_speed: f32,
    pub(crate) death_speed: f32,
}

impl Default for SoldierAnimSettings {
    fn default() -> Self {
        Self {
            base_walk_speed: 2.2,
            base_sprint_speed: 1.4,
            base_aim_walk_speed: 0.8,
            base_aim_sprint_speed: 1.5,
            base_crouch_walk_speed: 1.4,
            base_strafe_speed: 2.0,
            base_backpaddle_speed: 1.5,
            death_speed: 1.0,
        }
    }
}

/// Set by `start_soldier_animation` once it finds the `AnimationPlayer` inside
/// a remote-player avatar's spawned scene. Mirrors [`BotAnimationPlayer`],
/// kept as its own type since it points into a different graph. Read by
/// `net::animate_remote_avatars` to switch between idle/walk/sprint, the same
/// way `play_bot_death` uses `BotAnimationPlayer`.
#[derive(Component)]
pub(crate) struct SoldierAnimationPlayer(pub(crate) Entity);

/// Panel-adjustable uniform scale for the `models/soldier.glb` remote-player
/// avatar ("Remote players" debug-panel section), applied by
/// `net::follow_remote_avatars`. Live-tweakable rather than a baked constant
/// like `BOT_MODEL_SCALE` — the model's authored size relative to a real
/// player isn't known up front.
#[derive(Resource)]
pub(crate) struct RemoteAvatarSettings {
    pub(crate) scale: f32,
}

impl Default for RemoteAvatarSettings {
    fn default() -> Self {
        Self { scale: 21.0 }
    }
}

/// Build the remote-player animation graph. Added to the same `Startup` tuple
/// as `setup_bot_assets` — see its comment for why a separate
/// `add_systems(Startup, ...)` (even from a plugin) can't be used instead.
fn setup_soldier_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Indices into `models/soldier.glb`'s 18 animations: 0 idleWgun, 1 walk,
    // 3 run, 4 shooting, 6 runAndShooting, 7 strafeRight, 8 strafeLeft,
    // 9 backpaddle, 10 jump, 11 crouch, 12 crouchWalk, 13 reload, 15 death.
    let idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("models/soldier.glb"));
    let walk_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(1).from_asset("models/soldier.glb"));
    let sprint_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(3).from_asset("models/soldier.glb"));
    let aim_idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(4).from_asset("models/soldier.glb"));
    let aim_move_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(6).from_asset("models/soldier.glb"));
    let crouch_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(11).from_asset("models/soldier.glb"));
    let crouch_walk_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(12).from_asset("models/soldier.glb"));
    let reload_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(13).from_asset("models/soldier.glb"));
    let jump_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(10).from_asset("models/soldier.glb"));
    let strafe_right_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(7).from_asset("models/soldier.glb"));
    let strafe_left_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(8).from_asset("models/soldier.glb"));
    let backward_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(9).from_asset("models/soldier.glb"));
    let death_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(15).from_asset("models/soldier.glb"));
    let (graph, indices) = AnimationGraph::from_clips([
        idle_clip,
        walk_clip,
        sprint_clip,
        aim_idle_clip,
        aim_move_clip,
        crouch_clip,
        crouch_walk_clip,
        reload_clip,
        jump_clip,
        strafe_right_clip,
        strafe_left_clip,
        backward_clip,
        death_clip,
    ]);
    let graph = graphs.add(graph);
    commands.insert_resource(SoldierAnimations {
        graph,
        idle: indices[0],
        walk: indices[1],
        sprint: indices[2],
        aim_idle: indices[3],
        aim_move: indices[4],
        crouch: indices[5],
        crouch_walk: indices[6],
        reload: indices[7],
        jump: indices[8],
        strafe_right: indices[9],
        strafe_left: indices[10],
        backward: indices[11],
        death: indices[12],
    });
}

/// Fires once a remote-player avatar's `SceneRoot` (tagged [`SoldierVisual`])
/// finishes spawning: starts the idle animation looping and remembers which
/// descendant holds the `AnimationPlayer`, via [`SoldierAnimationPlayer`].
/// Mirrors `start_bot_animation`.
fn start_soldier_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    soldiers: Query<(), With<SoldierVisual>>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<SoldierAnimations>,
) {
    let root = trigger.target();
    if !soldiers.contains(root) {
        return;
    }
    for entity in children.iter_descendants(root) {
        // Same fix as the bot / sniper view model: a skinned mesh is
        // frustum-culled against its *rest-pose* AABB, not the animated one.
        commands.entity(entity).insert(NoFrustumCulling);

        if let Ok(mut player) = players.get_mut(entity) {
            let mut transitions = AnimationTransitions::new();
            transitions
                .play(&mut player, anims.idle, Duration::ZERO)
                .set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert((AnimationGraphHandle(anims.graph.clone()), transitions));
            commands.entity(root).insert(SoldierAnimationPlayer(entity));
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
    /// `Shipment`'s own ambience bed — a cargo ship out on open water, so
    /// nothing like `ambient`'s outdoor-nature loop. `start_ambient` picks
    /// between the two by [`CurrentMap`]; `basic_map.glb` keeps `ambient`.
    shipment_ambient: Handle<AudioSource>,
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

/// Linear volume shared by both ambience loops (`ambient` and
/// `shipment_ambient`) before their own "Sound volumes" multiplier.
const AMBIENT_VOLUME: f32 = 0.5;

/// Panel-adjustable per-sound volume multipliers ("Sound volumes" panel
/// section). `1.0` leaves a sound at its built-in level; every one-shot is
/// scaled by its entry when it spawns (`apply_sound_volumes`), and whichever
/// ambience loop is currently playing by `ambient` / `shipment_ambient`
/// (folded into `apply_master_volume`). Footsteps have their own controls in
/// the "Footsteps" section and aren't here.
#[derive(Resource)]
struct SoundVolumes {
    shot: f32,
    rechamber: f32,
    reload: f32,
    ambient: f32,
    shipment_ambient: f32,
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
            shipment_ambient: 2.5,
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

/// Tags a sound spawned by `net::receive_remote_sounds` for another player's
/// action (reload, footstep, shot, ...), positioned at their world location.
/// `apply_sound_volumes` skips these — their volume is already fully baked in
/// at spawn time (category volume × distance falloff × the "Remote sounds"
/// panel's master volume × global volume), since re-deriving it post-spawn
/// would need the same distance calculation all over again for no benefit.
#[derive(Component)]
pub(crate) struct RemoteSoundEmitter;

/// Distance falloff for other players' positional sounds ("Remote sounds"
/// debug-panel section). Applied by `net::receive_remote_sounds` on top of
/// this same clip's normal `SoundVolumes` category multiplier.
#[derive(Resource, Clone, Copy)]
pub(crate) struct RemoteSoundSettings {
    /// Overall gain on every remote-player sound, on top of its usual
    /// per-category volume.
    pub(crate) volume: f32,
    /// Distance (m) at which a remote sound has faded to silence.
    pub(crate) max_distance: f32,
}

impl Default for RemoteSoundSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            max_distance: 60.0,
        }
    }
}

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
            rise_rate: 0.15,
            spread: 0.1,
            fade_in: 0.1,
            fade_time: 0.4,
            spawn_rate: 40.0,
            duration: 1.5,
            max_opacity: 0.03,
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
            smoke_secs: 2.0,
            flash_color: srgb_parts(Color::srgb(1.0, 0.8, 0.35)),
            flash_emissive_boost: 6.0,
            flash_radius: 0.018,
            smoke_color: srgb_parts(Color::srgb(0.72, 0.7, 0.66)),
            smoke_start_alpha: 0.03,
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

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

fn setup_world(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    water: Res<WaterSettings>,
    light: Res<ShipmentLightSettings>,
    fluoro: Res<FluoroLightSettings>,
    bulbs: Res<BulbLightSettings>,
) {
    // Ground: a 200 m plane wrapped in a seamless procedural asphalt texture
    // (see `build_ground_texture`), tiled every ~2 m. Hidden, and its
    // `Collider` disabled, by `sync_shipment_only_visibility` for maps that ship
    // their own ground (so far just `shipment.glb`'s `Ground` node) — active
    // otherwise, as the fallback floor `apply_gravity`'s raycast lands on
    // off the edge of any map's own mesh collision (or on a map, like
    // `basic_map.glb`, that never had a ground mesh of its own to begin
    // with).
    commands.spawn((
        ProceduralGround,
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(100.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(build_ground_texture())),
            uv_transform: Affine2::from_scale(Vec2::splat(100.0)),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Collider::halfspace(Vec3::Y).expect("Vec3::Y is already a unit vector"),
    ));

    // Ocean: a big, tinted, semi-transparent plane just below Shipment's
    // ground level, ringing the yard the way MW3 Shipment's cargo-ship
    // setting does. `water_normal.png` supplies rippling detail via a
    // normal map — loaded `is_srgb: false` since it encodes surface
    // directions, not colour (the default `true` would gamma-decode the
    // vectors and distort them), and with `Repeat` addressing so
    // `scroll_water_normal`'s `uv_transform` can both tile it across the
    // plane and animate it drifting over time; the default sampler clamps
    // instead (see `build_ground_texture`, which needs the same override
    // for the same reason). No collider: the plane sits outside the yard's
    // walls, unreachable during normal play. Spawned hidden to match the
    // default map (`BasicMap`, no nautical setting); `sync_shipment_only_visibility`
    // shows it for `Shipment`. Initial transform/material values come from
    // `WaterSettings` (its `Default`, at this point) so there's one source
    // of truth instead of duplicating numbers that `apply_water_settings`
    // would immediately overwrite on the first `Update` anyway.
    let water_normal = asset_server.load_with_settings(
        "textures/water_normal.png",
        |settings: &mut ImageLoaderSettings| {
            settings.is_srgb = false;
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                ..default()
            });
        },
    );
    let [r, g, b] = water.tint;
    let water_material = materials.add(StandardMaterial {
        base_color: Color::srgba(r, g, b, water.alpha),
        perceptual_roughness: water.roughness,
        reflectance: water.reflectance,
        alpha_mode: AlphaMode::Blend,
        normal_map_texture: Some(water_normal),
        // Bevy's transparent pass sorts by distance-to-camera taken from each
        // entity's own `Transform::translation` — fine for small effects, but
        // this is one huge 300m-radius plane anchored at a single point, so
        // that one-point distance can land closer than a near-camera effect
        // (barrel smoke, tracers, blood) that's actually right in front of the
        // lens, and the water then draws over it. A large negative bias pins
        // the water's sort order to always-background, regardless: it only
        // reorders transparent draw order, not the real depth buffer, so nothing
        // about how the water actually looks or z-fights changes.
        depth_bias: -1000.0,
        ..default()
    });
    // `Plane3d`'s generated mesh has positions/normals/UVs but no tangents
    // (see `bevy_mesh`'s plane primitive) — `normal_map_texture` needs them
    // to know which way "into the surface" faces, so without this the
    // normal map is silently ignored and the plane renders as flat as if it
    // had none (a glTF-loaded mesh missing tangents gets these computed
    // automatically by `bevy_gltf`'s loader; a procedural mesh like this one
    // doesn't go through that loader, so it needs this explicit call
    // instead).
    let mut water_mesh: Mesh = Plane3d::new(Vec3::Y, Vec2::splat(300.0)).into();
    water_mesh
        .generate_tangents()
        .expect("a flat, UV-mapped, indexed plane always has enough data to derive tangents");
    commands.spawn((
        WaterPlane,
        Mesh3d(meshes.add(water_mesh)),
        MeshMaterial3d(water_material.clone()),
        WaterMaterial(water_material),
        Transform::from_xyz(0.0, -water.level_drop, 0.0),
        Visibility::Hidden,
    ));

    // Shipment floodlights: spotlights mounted high on the cargo ship,
    // lighting the yard below — see `ShipmentLightSettings` for why each
    // one's initial transform/params come from that resource rather than
    // being hardcoded here (same reasoning as the water plane above them).
    // Hidden for `BasicMap`; `sync_shipment_only_visibility` shows them for
    // `Shipment`. The bulb/rod gizmo mesh + material (see
    // `ShipmentLightMarker`) are built once and shared (`.clone()`d) across
    // every light's pair, rather than once per light — they're geometrically
    // identical, so there's nothing light-specific to bake into either.
    let marker_bulb_mesh = meshes.add(Sphere::new(LIGHT_MARKER_BULB_RADIUS));
    let marker_bulb_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.2),
        unlit: true,
        ..default()
    });
    let marker_rod_mesh = meshes.add(Cuboid::new(
        LIGHT_MARKER_ROD_THICKNESS,
        LIGHT_MARKER_ROD_THICKNESS,
        LIGHT_MARKER_ROD_LENGTH,
    ));
    let marker_rod_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.9, 1.0),
        unlit: true,
        ..default()
    });
    // Unlike the debug-only gizmo mesh/material above, [`ShipmentLightGlow`]'s
    // material carries each light's own colour/brightness, so (mesh aside)
    // it's built fresh per light below rather than shared.
    let glow_mesh = meshes.add(Sphere::new(LIGHT_GLOW_BULB_RADIUS));
    for (index, cfg) in light.lights.iter().enumerate() {
        let glow_color = color_from_parts(cfg.color);
        let glow_material = materials.add(StandardMaterial {
            base_color: glow_color,
            emissive: LinearRgba::from(glow_color) * cfg.glow_intensity / GLOW_EMISSIVE_PER_LUMEN,
            ..default()
        });
        commands
            .spawn((
                ShipmentSpotLight(index),
                SpotLight {
                    color: color_from_parts(cfg.color),
                    intensity: cfg.intensity,
                    range: cfg.range,
                    inner_angle: cfg.inner_angle_deg.to_radians(),
                    outer_angle: cfg.outer_angle_deg.to_radians(),
                    shadows_enabled: cfg.shadows_enabled,
                    ..default()
                },
                Transform {
                    translation: cfg.position,
                    rotation: Quat::from_euler(
                        EulerRot::YXZ,
                        cfg.yaw_deg.to_radians(),
                        cfg.pitch_deg.to_radians(),
                        0.0,
                    ),
                    ..default()
                },
                Visibility::Hidden,
            ))
            .with_children(|light| {
                // Debug-only gizmo — a bulb at the fixture and a rod
                // pointing along its beam direction, so the position/aim
                // controls in the "Shipment Lights" panel have something
                // visible to calibrate against. `unlit` so it reads clearly
                // regardless of how dark/foggy the scene itself is.
                light.spawn((
                    ShipmentLightMarker,
                    Mesh3d(marker_bulb_mesh.clone()),
                    MeshMaterial3d(marker_bulb_material.clone()),
                    Visibility::Hidden,
                ));
                light.spawn((
                    ShipmentLightMarker,
                    Mesh3d(marker_rod_mesh.clone()),
                    MeshMaterial3d(marker_rod_material.clone()),
                    // A `Cuboid`'s local Z already spans the rod's length,
                    // centred on its parent's origin — shift it half a
                    // length forward (local -Z, `SpotLight`'s own shine
                    // direction) so it starts at the bulb and extends
                    // outward instead of piercing through it.
                    Transform::from_xyz(0.0, 0.0, -LIGHT_MARKER_ROD_LENGTH / 2.0),
                    Visibility::Hidden,
                ));
                // Always-visible glow — see `ShipmentLightGlow`. No explicit
                // `Visibility` (defaults to `Inherited`), unlike the two debug
                // gizmos above: it should show whenever its parent light does.
                light.spawn((
                    ShipmentLightGlow(index),
                    Mesh3d(glow_mesh.clone()),
                    MeshMaterial3d(glow_material),
                    PointLight {
                        color: glow_color,
                        intensity: cfg.glow_intensity,
                        range: LIGHT_GLOW_RANGE,
                        shadows_enabled: false,
                        ..default()
                    },
                ));
            });
    }

    // Fluorescent light: the tube fixture model inside one of
    // `shipment_visual.glb`'s containers — see `FluoroLightSettings`. Hidden
    // for `BasicMap`; `sync_shipment_only_visibility` shows it for
    // `Shipment`, same as the floodlights above.
    commands
        .spawn((
            ContainerFluoroLight,
            PointLight {
                color: color_from_parts(fluoro.color),
                intensity: fluoro.intensity,
                range: fluoro.range,
                shadows_enabled: fluoro.shadows_enabled,
                ..default()
            },
            Transform::from_translation(fluoro.position),
            Visibility::Hidden,
        ))
        .with_child((
            ContainerFluoroLightMarker,
            Mesh3d(marker_bulb_mesh.clone()),
            MeshMaterial3d(marker_bulb_material.clone()),
            Visibility::Hidden,
        ));

    // Bulb lights: the two bare-bulb fixture models in another container —
    // see `BulbLightSettings`. Reuses the same debug-gizmo bulb mesh/
    // material as the fluorescent light and the floodlights' markers above
    // (all just "a small sphere at this position" in the end).
    for (index, &pos) in bulbs.positions.iter().enumerate() {
        commands
            .spawn((
                ContainerBulbLight(index),
                PointLight {
                    color: color_from_parts(bulbs.color),
                    intensity: bulbs.intensity,
                    range: bulbs.range,
                    shadows_enabled: bulbs.shadows_enabled,
                    ..default()
                },
                Transform::from_translation(pos),
                Visibility::Hidden,
            ))
            .with_child((
                ContainerBulbLightMarker,
                Mesh3d(marker_bulb_mesh.clone()),
                MeshMaterial3d(marker_bulb_material.clone()),
                Visibility::Hidden,
            ));
    }

    // The selected map's model is kept in sync by `sync_map_model` (not
    // spawned here) so it can be swapped per-lobby — its `AsyncSceneCollider`
    // generates real mesh collision for whichever `.glb` that loads, so
    // `apply_gravity` and `resolve_wall_collisions` work the same way for
    // every map without any hand-authored per-map data.

    // Sky: the equirectangular HDR mapped onto the inside of a big UV sphere
    // that follows the camera (see `sky_follow_camera`). Not a true cubemap
    // skybox, but it needs no offline conversion and reads fine as a
    // backdrop. Which HDR shows depends on the map (`sky_texture_path`) —
    // `sync_sky_texture` swaps it on `CurrentMap` changes; this is just the
    // initial load for whichever map is selected at startup (`BasicMap` by
    // default).
    let sky_material = materials.add(StandardMaterial {
        base_color_texture: Some(asset_server.load(sky_texture_path(shared::MapId::default()))),
        unlit: true,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        SkySphere,
        Mesh3d(meshes.add(Sphere::new(SKY_RADIUS).mesh().uv(128, 64))),
        MeshMaterial3d(sky_material.clone()),
        SkyMaterial(sky_material),
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
    knife_settings: Res<KnifeViewModelSettings>,
) {
    // Build a one-clip animation graph for the sniper's baked animation.
    let clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("models/sniper.glb"));
    let (graph, index) = AnimationGraph::from_clip(clip);
    let graph = graphs.add(graph);

    // Same, for the knife's own baked clip.
    let knife_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("models/knife.glb"));
    let (knife_graph, knife_index) = AnimationGraph::from_clip(knife_clip);
    let knife_graph = graphs.add(knife_graph);

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
                PlayerBodyCapsule,
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
                                        // The player's "ears" for remote-player
                                        // positional sounds (`net::receive_remote_sounds`).
                                        // Offsets deliberately reversed from
                                        // `SpatialListener::new`'s default
                                        // (left at +X, right at -X) — with the
                                        // "correct" orientation, sounds panned
                                        // opposite the source's actual side.
                                        SpatialListener {
                                            left_ear_offset: Vec3::X * 0.15,
                                            right_ear_offset: Vec3::X * -0.15,
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

                            // The knife view model — the other half of
                            // `WeaponSlot`. Hidden by default: `Primary` (the
                            // sniper) is the slot the player starts equipped
                            // with, same as the sniper above starts visible.
                            rig.spawn((
                                KnifeViewModel,
                                KnifeAnimation {
                                    graph: knife_graph,
                                    index: knife_index,
                                },
                                SceneRoot(asset_server.load(
                                    GltfAssetLabel::Scene(0).from_asset("models/knife.glb"),
                                )),
                                Transform {
                                    translation: knife_settings.translation,
                                    rotation: Quat::from_euler(EulerRot::YXZ, PI, 0.0, 0.0),
                                    scale: Vec3::splat(knife_settings.scale),
                                },
                                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                                Visibility::Hidden,
                            ))
                            .observe(start_knife_animation);

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
        shipment_ambient: asset_server.load("audio/shipment_ambient.ogg"),
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

/// Start the map's looping ambience bed when the player enters the world —
/// `basic_map.glb`'s outdoor-nature loop, or `shipment.glb`'s own cargo-ship
/// one, by [`CurrentMap`]. Ordered `.after(lobby_ui::sync_current_map)`
/// (both run on `OnEnter(AppState::InGame)`) so this always sees the map
/// just selected, not whatever `CurrentMap` was left at after the previous
/// match.
fn start_ambient(
    mut commands: Commands,
    sounds: Res<GameSounds>,
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    current: Res<CurrentMap>,
) {
    let (clip, volume_mult) = match current.0 {
        shared::MapId::BasicMap => (sounds.ambient.clone(), vols.ambient),
        shared::MapId::Shipment => (sounds.shipment_ambient.clone(), vols.shipment_ambient),
    };
    commands.spawn((
        AmbientAudio,
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(
            AMBIENT_VOLUME * volume_mult * settings.master_volume,
        )),
    ));
}

/// Push `Settings::master_volume` onto Bevy's `GlobalVolume`, which scales
/// every one-shot sound spawned from here on (shots, footsteps, UI, ...) with
/// no per-call-site changes needed. `GlobalVolume` doesn't retroactively touch
/// audio that's already playing, though, so the looping ambience needs its own
/// direct nudge here too — also picking up the "Sound volumes" panel's
/// multiplier for whichever ambience loop `start_ambient` actually started.
fn apply_master_volume(
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    current: Res<CurrentMap>,
    mut global_volume: ResMut<GlobalVolume>,
    mut ambient: Query<&mut AudioSink, With<AmbientAudio>>,
) {
    if !settings.is_changed() && !vols.is_changed() {
        return;
    }
    global_volume.volume = Volume::Linear(settings.master_volume);
    let ambient_mult = match current.0 {
        shared::MapId::BasicMap => vols.ambient,
        shared::MapId::Shipment => vols.shipment_ambient,
    };
    for mut sink in &mut ambient {
        sink.set_volume(Volume::Linear(
            AMBIENT_VOLUME * ambient_mult * settings.master_volume,
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
    mut fresh: Query<
        (&AudioPlayer, &mut AudioSink),
        (Added<AudioSink>, Without<RemoteSoundEmitter>),
    >,
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
/// stack. `total` is the shot's combined points (shown as its own line at the
/// top); `lines` are the itemised `(label, points)` that added up to it, top
/// to bottom below that.
#[derive(Event)]
pub(crate) struct TrickScoredEvent {
    pub(crate) total: u32,
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
            // The combined total leads the stack, bigger than the breakdown
            // below it, so it reads as the headline with the itemised lines
            // explaining where it came from.
            col.spawn((
                Text::new(format!("+{}  TOTAL", ev.total)),
                TextFont {
                    font: asset_server.load(HUD_FONT),
                    font_size: 32.0,
                    ..default()
                },
                TextColor(SCORE_YELLOW),
            ));
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
        ResMut<ShipmentSettings>,
        ResMut<BloodSettings>,
        ResMut<NoScopeSpread>,
        ResMut<IdleSwaySettings>,
        ResMut<AimSwaySettings>,
        ResMut<RemoteAvatarSettings>,
        ResMut<SoldierAnimSettings>,
        ResMut<RemoteSoundSettings>,
        ResMut<WaterSettings>,
        ResMut<ShipmentSceneTuning>,
        ResMut<ShipmentLightSettings>,
        (
            ResMut<RainSettings>,
            ResMut<KnifeViewModelSettings>,
            ResMut<FluoroLightSettings>,
            ResMut<BulbLightSettings>,
        ),
    ),
) -> Result {
    let (
        shake,
        ads,
        mut map,
        mut shipment,
        mut blood,
        mut noscope,
        mut idle_sway,
        mut aim_sway,
        mut remote_avatar,
        mut soldier_anim,
        mut remote_sound,
        mut water,
        mut shipment_scene,
        mut shipment_light,
        (mut rain, mut knife_view, mut fluoro, mut bulbs),
    ) = misc;
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
            ui.collapsing("Knife", |ui| {
                let k = &mut *knife_view;
                ui.label("models/knife.glb — position and scale only (see WeaponSlot::Secondary)");
                ui.label(
                    "Position range is wide on purpose — push it out past the normal hip-pose \
                     range to stand it next to a bot in world space and check the scale reads \
                     right, then dial it back in for the actual view-model pose.",
                );
                ui.add(
                    egui::Slider::new(&mut k.translation.x, -20.0f32..=20.0).text("x  (right +)"),
                );
                ui.add(egui::Slider::new(&mut k.translation.y, -20.0f32..=20.0).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut k.translation.z, -40.0f32..=5.0).text("z  (forward -)"),
                );
                ui.add(
                    egui::Slider::new(&mut k.scale, 0.001f32..=0.05)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy knife pose to console").clicked() {
                    info!(
                        "knife: translation: Vec3::new({:.4}, {:.4}, {:.4}), scale: {:.5}",
                        k.translation.x, k.translation.y, k.translation.z, k.scale,
                    );
                }
                if ui.button("Reset knife pose to default").clicked() {
                    *k = KnifeViewModelSettings::default();
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
            ui.collapsing("Shipment map", |ui| {
                let sh = &mut *shipment;
                ui.label("models/shipment.glb — spawned at the origin, scale only");
                ui.add(
                    egui::Slider::new(&mut sh.scale, 0.05f32..=2.0)
                        .text("scale")
                        .logarithmic(true),
                );
                ui.label(
                    "Only affects this client's own rendering + collision — the server's \
                     spawn/respawn placement always uses shared::map::SHIPMENT_SCALE, so \
                     update that constant to match once you've found the right number.",
                );

                if ui.button("Copy shipment scale to console").clicked() {
                    info!("shipment: SHIPMENT_SCALE = {:.3};", sh.scale);
                }
                if ui.button("Reset shipment scale").clicked() {
                    *sh = ShipmentSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Water", |ui| {
                let w = &mut *water;
                ui.label("Shipment only — the MW3-style cargo-ship setting's ocean plane");
                ui.add(
                    egui::Slider::new(&mut w.level_drop, -3.0f32..=8.0)
                        .text("level drop below ground (m)"),
                );
                ui.horizontal(|ui| {
                    ui.label("tint");
                    ui.color_edit_button_rgb(&mut w.tint);
                });
                ui.add(egui::Slider::new(&mut w.alpha, 0.0f32..=1.0).text("opacity"));
                ui.add(
                    egui::Slider::new(&mut w.roughness, 0.0f32..=1.0)
                        .text("roughness  (lower = shinier)"),
                );
                ui.add(egui::Slider::new(&mut w.reflectance, 0.0f32..=1.0).text("reflectance"));
                ui.add(
                    egui::Slider::new(&mut w.normal_tiling, 5.0f32..=200.0)
                        .text("ripple tiling  (higher = smaller ripples)")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut w.scroll_speed.x, -0.1f32..=0.1).text("ripple scroll x"),
                );
                ui.add(
                    egui::Slider::new(&mut w.scroll_speed.y, -0.1f32..=0.1).text("ripple scroll y"),
                );

                if ui.button("Copy water settings to console").clicked() {
                    info!(
                        "water: level_drop: {:.3}, tint: Color::srgb({:.3}, {:.3}, {:.3}), \
                         alpha: {:.3}, roughness: {:.3}, reflectance: {:.3}, normal_tiling: \
                         {:.1}, scroll_speed: Vec2::new({:.4}, {:.4})",
                        w.level_drop,
                        w.tint[0],
                        w.tint[1],
                        w.tint[2],
                        w.alpha,
                        w.roughness,
                        w.reflectance,
                        w.normal_tiling,
                        w.scroll_speed.x,
                        w.scroll_speed.y,
                    );
                }
                if ui.button("Reset water").clicked() {
                    *w = WaterSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Remote players", |ui| {
                let ra = &mut *remote_avatar;
                ui.label("models/soldier.glb");
                ui.add(
                    egui::Slider::new(&mut ra.scale, 0.01f32..=100.0)
                        .text("scale")
                        .logarithmic(true),
                );
                if ui.button("Reset remote player scale").clicked() {
                    *ra = RemoteAvatarSettings::default();
                }

                ui.separator();
                ui.label("Player walk/sprint speed also drives these clips — see \"Movement\".");
                let sa = &mut *soldier_anim;
                ui.add(
                    egui::Slider::new(&mut sa.base_walk_speed, 0.1f32..=8.0)
                        .text(format!("walk anim speed (× at {WALK_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_sprint_speed, 0.1f32..=8.0)
                        .text(format!("sprint anim speed (× at {SPRINT_SPEED} m/s)")),
                );
                ui.label("No walk-and-shoot clip — runAndShooting covers both aiming states:");
                ui.add(
                    egui::Slider::new(&mut sa.base_aim_walk_speed, 0.1f32..=8.0)
                        .text(format!("aim+walk anim speed (× at {WALK_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_aim_sprint_speed, 0.1f32..=8.0)
                        .text(format!("aim+sprint anim speed (× at {SPRINT_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_crouch_walk_speed, 0.1f32..=8.0)
                        .text(format!("crouch walk anim speed (× at {CROUCH_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_strafe_speed, 0.1f32..=8.0).text(format!(
                        "strafe anim speed (× at {} m/s)",
                        WALK_SPEED * STRAFE_SPEED_MULT
                    )),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_backpaddle_speed, 0.1f32..=8.0).text(format!(
                        "backpaddle anim speed (× at {} m/s)",
                        WALK_SPEED * BACKWARD_SPEED_MULT
                    )),
                );
                ui.add(
                    egui::Slider::new(&mut sa.death_speed, 0.1f32..=5.0)
                        .text("death anim speed (×)"),
                );
                if ui.button("Reset remote player anim speed").clicked() {
                    *sa = SoldierAnimSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Remote sounds", |ui| {
                ui.label("Other players' footsteps/jump/slide/reload/rechamber/shot/dive.");
                let rs = &mut *remote_sound;
                ui.add(egui::Slider::new(&mut rs.volume, 0.0f32..=3.0).text("volume (×)"));
                ui.add(
                    egui::Slider::new(&mut rs.max_distance, 5.0f32..=300.0)
                        .text("max distance (m)"),
                );
                if ui.button("Reset remote sounds").clicked() {
                    *rs = RemoteSoundSettings::default();
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
                    ("shipment ambient", &mut v.shipment_ambient),
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
                ui.add(
                    egui::Slider::new(&mut m.strafe_speed_mult, 0.1f32..=1.5)
                        .text("strafe speed (×)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.backward_speed_mult, 0.1f32..=1.5)
                        .text("backward speed (×)"),
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
                ui.label(
                    "the gun angles away from the turn and catches up — this is the ADS \
                     sway now (the scope reticle itself never moves, see Crosshair)",
                );
                ui.add(
                    egui::Slider::new(&mut w.hip_strength, 0.0f32..=2.0)
                        .text("hip strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.ads_strength, 0.0f32..=1.0)
                        .text("ADS strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.return_speed, 1.0f32..=20.0).text("catch-up speed"),
                );
                ui.add(
                    egui::Slider::new(&mut w.max_offset_deg, 0.0f32..=30.0).text("max offset (°)"),
                );
                ui.label("shift — the gun also translates the way it's angled");
                ui.add(
                    egui::Slider::new(&mut w.hip_shift_m, 0.0f32..=0.1)
                        .text("hip shift (m per rad of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.ads_shift_m, 0.0f32..=0.2)
                        .text("ADS shift (m per rad of lag)"),
                );
                ui.label(format!(
                    "live strength @ ads.t {:.2} = {:.4}",
                    ads.t,
                    w.hip_strength.lerp(w.ads_strength, ads.t.clamp(0.0, 1.0)),
                ));

                if ui.button("Copy weapon sway to console").clicked() {
                    info!(
                        "weapon sway: hip_strength {:.4}, ads_strength {:.4}, \
                         return_speed {:.4}, max_offset_deg {:.4}, hip_shift_m {:.4}, \
                         ads_shift_m {:.4}",
                        w.hip_strength,
                        w.ads_strength,
                        w.return_speed,
                        w.max_offset_deg,
                        w.hip_shift_m,
                        w.ads_shift_m,
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
                    egui::Slider::new(&mut s.amplitude_deg.x, 0.0f32..=2.0).text("amplitude X (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.y, 0.0f32..=2.0).text("amplitude Y (°)"),
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
                ui.label(
                    "REAL aim breathing while scoped (scales up into ADS) — actually turns \
                     the camera, so it moves where a shot lands. The reticle stays pinned to \
                     the screen; the world drifts under it instead.",
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.x, 0.0f32..=1.0).text("amplitude X (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.y, 0.0f32..=1.0).text("amplitude Y (°)"),
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
                ui.label(
                    "that raise slide is the reticle's only motion — no turn lag any more, \
                     it's pinned to dead centre once scoped (see Weapon sway / Aim sway \
                     instead: that's where the sway went).",
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
                ui.add(egui::Slider::new(&mut c.ads_scale, 0.0f32..=1.0).text(
                    "ADS scale — jitter + punch + shudder left at full ADS \
                         (ramps to full at the hip)",
                ));
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
            ui.collapsing("Fog & Sky (Basic Map)", |ui| {
                scene_tuning_sliders(ui, &mut scene);
                if ui.button("Reset fog & sky").clicked() {
                    *scene = SceneTuning::default();
                }
            });

            ui.separator();
            ui.collapsing("Fog & Sky (Shipment)", |ui| {
                ui.label("MW3-style setting: dark, foggy, overcast, out on open water");
                scene_tuning_sliders(ui, &mut shipment_scene.0);
                if ui.button("Reset fog & sky").clicked() {
                    *shipment_scene = ShipmentSceneTuning::default();
                }
            });

            ui.separator();
            ui.collapsing("Shipment Lights", |ui| {
                ui.checkbox(
                    &mut shipment_light.markers_visible,
                    "show position/aim markers",
                );
                ui.label(
                    "Off by default — the bulb + rod gizmo is only there to help place the \
                     lights, not something to leave on.",
                );
            });

            ui.separator();
            ui.collapsing("Shipment Light 1", |ui| {
                shipment_light_sliders(ui, &mut shipment_light.lights[0], 0);
            });

            ui.separator();
            ui.collapsing("Shipment Light 2", |ui| {
                shipment_light_sliders(ui, &mut shipment_light.lights[1], 1);
            });

            ui.separator();
            ui.collapsing("Fluorescent Light", |ui| {
                let f = &mut *fluoro;
                ui.label("Shipment only — the tube fixture model inside one container");
                ui.checkbox(&mut f.markers_visible, "show position marker");
                ui.add(egui::Slider::new(&mut f.position.x, -80.0f32..=80.0).text("x"));
                ui.add(egui::Slider::new(&mut f.position.y, 0.0f32..=60.0).text("y (height)"));
                ui.add(egui::Slider::new(&mut f.position.z, -80.0f32..=80.0).text("z"));
                point_light_sliders(
                    ui,
                    &mut f.color,
                    &mut f.intensity,
                    &mut f.range,
                    &mut f.shadows_enabled,
                );

                if ui.button("Copy fluorescent light to console").clicked() {
                    info!(
                        "fluoro: position: Vec3::new({:.2}, {:.2}, {:.2}), color: \
                         Color::srgb({:.3}, {:.3}, {:.3}), intensity: {:.0}, range: {:.1}, \
                         shadows_enabled: {}",
                        f.position.x,
                        f.position.y,
                        f.position.z,
                        f.color[0],
                        f.color[1],
                        f.color[2],
                        f.intensity,
                        f.range,
                        f.shadows_enabled,
                    );
                }
                if ui.button("Reset fluorescent light").clicked() {
                    *f = FluoroLightSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Bulb Lights", |ui| {
                let b = &mut *bulbs;
                ui.label(
                    "Shipment only — two bare bulbs in another container; everything below is \
                     shared between both, only position is per-bulb",
                );
                ui.checkbox(&mut b.markers_visible, "show position markers");
                ui.label("Bulb 1 position");
                ui.add(egui::Slider::new(&mut b.positions[0].x, -80.0f32..=80.0).text("x"));
                ui.add(
                    egui::Slider::new(&mut b.positions[0].y, 0.0f32..=60.0).text("y (height)"),
                );
                ui.add(egui::Slider::new(&mut b.positions[0].z, -80.0f32..=80.0).text("z"));
                ui.label("Bulb 2 position");
                ui.add(egui::Slider::new(&mut b.positions[1].x, -80.0f32..=80.0).text("x"));
                ui.add(
                    egui::Slider::new(&mut b.positions[1].y, 0.0f32..=60.0).text("y (height)"),
                );
                ui.add(egui::Slider::new(&mut b.positions[1].z, -80.0f32..=80.0).text("z"));
                ui.separator();
                ui.label("Shared");
                point_light_sliders(
                    ui,
                    &mut b.color,
                    &mut b.intensity,
                    &mut b.range,
                    &mut b.shadows_enabled,
                );

                if ui.button("Copy bulb lights to console").clicked() {
                    info!(
                        "bulbs: positions: [Vec3::new({:.2}, {:.2}, {:.2}), Vec3::new({:.2}, \
                         {:.2}, {:.2})], color: Color::srgb({:.3}, {:.3}, {:.3}), intensity: \
                         {:.0}, range: {:.1}, shadows_enabled: {}",
                        b.positions[0].x,
                        b.positions[0].y,
                        b.positions[0].z,
                        b.positions[1].x,
                        b.positions[1].y,
                        b.positions[1].z,
                        b.color[0],
                        b.color[1],
                        b.color[2],
                        b.intensity,
                        b.range,
                        b.shadows_enabled,
                    );
                }
                if ui.button("Reset bulb lights").clicked() {
                    *b = BulbLightSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Rain", |ui| {
                let r = &mut *rain;
                ui.label("Shipment only — real 3D streaks, not a screen overlay");
                ui.checkbox(&mut r.enabled, "enabled");
                ui.add(egui::Slider::new(&mut r.count, 0..=RAIN_MAX_DROPS).text("streak count"));
                ui.add(
                    egui::Slider::new(&mut r.radius, 2.0f32..=60.0)
                        .text("radius around player (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut r.spawn_height, 2.0f32..=60.0)
                        .text("spawn height above player (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut r.fall_speed, 0.5f32..=30.0).text("fall speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut r.wind.x, -10.0f32..=10.0).text("wind x (m/s)"));
                ui.add(egui::Slider::new(&mut r.wind.y, -10.0f32..=10.0).text("wind z (m/s)"));
                ui.separator();
                ui.label("Streak look");
                ui.add(
                    egui::Slider::new(&mut r.streak_length, 0.05f32..=3.0)
                        .text("streak length (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut r.streak_radius, 0.002f32..=0.1)
                        .text("streak thickness (m)"),
                );
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut r.color);
                    ui.label("colour");
                });
                ui.add(egui::Slider::new(&mut r.opacity, 0.0f32..=1.0).text("opacity"));

                if ui.button("Reset rain").clicked() {
                    *r = RainSettings::default();
                }
            });
        });
    Ok(())
}

/// Fog/sun/ambient/bloom sliders shared by "Fog & Sky (Basic Map)" and
/// "Fog & Sky (Shipment)" — same [`SceneTuning`] shape, different resource
/// (and therefore different defaults) behind each.
fn scene_tuning_sliders(ui: &mut egui::Ui, s: &mut SceneTuning) {
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
        egui::Slider::new(&mut s.fog_sun_exponent, 1.0f32..=100.0).text("sun-scatter tightness"),
    );
    ui.separator();
    ui.add(egui::Slider::new(&mut s.sun_lux, 0.0f32..=120_000.0).text("sun (lux)"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.sun_color);
        ui.label("sun colour");
    });
    ui.add(egui::Slider::new(&mut s.ambient_lux, 0.0f32..=6000.0).text("sky ambient (lux)"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.ambient_color);
        ui.label("ambient colour");
    });
    ui.add(egui::Slider::new(&mut s.bloom_intensity, 0.0f32..=0.5).text("bloom"));
}

/// Position/aim/cone sliders + copy/reset buttons for one [`ShipmentLight`]
/// slot — shared by "Shipment Light 1" and "Shipment Light 2". `index` is
/// only needed for the reset button, to pull that slot's own default back
/// out of [`ShipmentLightSettings::default`] rather than some other light's.
fn shipment_light_sliders(ui: &mut egui::Ui, l: &mut ShipmentLight, index: usize) {
    ui.add(egui::Slider::new(&mut l.position.x, -80.0f32..=80.0).text("x"));
    ui.add(egui::Slider::new(&mut l.position.y, 0.0f32..=60.0).text("y (height)"));
    ui.add(egui::Slider::new(&mut l.position.z, -80.0f32..=80.0).text("z"));
    ui.add(egui::Slider::new(&mut l.yaw_deg, -180.0f32..=180.0).text("yaw°  (heading)"));
    ui.add(
        egui::Slider::new(&mut l.pitch_deg, -89.0f32..=89.0).text("pitch°  (negative tilts down)"),
    );
    ui.separator();
    ui.label("Cone / beam");
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut l.color);
        ui.label("colour");
    });
    ui.add(
        egui::Slider::new(&mut l.intensity, 0.0f32..=100_000_000.0)
            .logarithmic(true)
            .text("intensity (lumens)"),
    );
    ui.add(egui::Slider::new(&mut l.range, 1.0f32..=200.0).text("range (m)"));
    ui.add(
        egui::Slider::new(&mut l.inner_angle_deg, 0.0f32..=89.0)
            .text("inner cone half-angle°  (hard core)"),
    );
    ui.add(
        egui::Slider::new(&mut l.outer_angle_deg, 0.0f32..=89.0)
            .text("outer cone half-angle°  (full spread — the \"triangle\")"),
    );
    ui.checkbox(&mut l.shadows_enabled, "cast shadows");
    ui.separator();
    ui.label("Glow  (the always-visible bulb at the fixture — see ShipmentLightGlow)");
    ui.add(
        egui::Slider::new(&mut l.glow_intensity, 0.0f32..=20_000_000.0)
            .logarithmic(true)
            .text("glow intensity (lumens)"),
    );

    if ui.button("Copy light settings to console").clicked() {
        info!(
            "shipment light {index}: position: Vec3::new({:.2}, {:.2}, {:.2}), yaw_deg: {:.1}, \
             pitch_deg: {:.1}, color: Color::srgb({:.3}, {:.3}, {:.3}), intensity: {:.0}, \
             range: {:.1}, inner_angle_deg: {:.1}, outer_angle_deg: {:.1}, shadows_enabled: {}, \
             glow_intensity: {:.0}",
            l.position.x,
            l.position.y,
            l.position.z,
            l.yaw_deg,
            l.pitch_deg,
            l.color[0],
            l.color[1],
            l.color[2],
            l.intensity,
            l.range,
            l.inner_angle_deg,
            l.outer_angle_deg,
            l.shadows_enabled,
            l.glow_intensity,
        );
    }
    if ui.button("Reset light").clicked() {
        *l = ShipmentLightSettings::default().lights[index].clone();
    }
}

/// Colour/intensity/range/shadow sliders shared by the "Fluorescent Light"
/// and "Bulb Lights" debug-panel sections — the parts of a `PointLight` that
/// aren't position.
fn point_light_sliders(
    ui: &mut egui::Ui,
    color: &mut [f32; 3],
    intensity: &mut f32,
    range: &mut f32,
    shadows_enabled: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(color);
        ui.label("colour");
    });
    ui.add(
        egui::Slider::new(intensity, 0.0f32..=6_000_000.0)
            .logarithmic(true)
            .text("intensity (lumens)"),
    );
    ui.add(egui::Slider::new(range, 0.5f32..=60.0).text("range (m)"));
    ui.checkbox(shadows_enabled, "cast shadows");
}

// ---------------------------------------------------------------------------
// Update systems
// ---------------------------------------------------------------------------

/// Push `SceneTuning` onto the live fog / sun / ambient / bloom whenever it
/// changes (also once at startup, which just re-applies the consts).
/// Picks whichever of [`SceneTuning`] (`BasicMap`) or [`ShipmentSceneTuning`]
/// (`Shipment`) is currently selected and pushes it onto the shared fog /
/// sun / ambient light / bloom — there's only one of each in the world, so
/// switching maps re-points them at a different look rather than swapping
/// entities.
fn apply_scene_tuning(
    current: Res<CurrentMap>,
    scene: Res<SceneTuning>,
    shipment_scene: Res<ShipmentSceneTuning>,
    mut ambient: ResMut<AmbientLight>,
    mut sun: Single<&mut DirectionalLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
    mut bloom: Single<&mut Bloom, With<WorldModelCamera>>,
) {
    if !current.is_changed() && !scene.is_changed() && !shipment_scene.is_changed() {
        return;
    }
    let active = match current.0 {
        shared::MapId::BasicMap => &*scene,
        shared::MapId::Shipment => &shipment_scene.0,
    };

    ambient.color = color_from_parts(active.ambient_color);
    ambient.brightness = active.ambient_lux;

    sun.illuminance = active.sun_lux;
    sun.color = color_from_parts(active.sun_color);

    fog.color = color_from_parts(active.fog_color);
    fog.directional_light_color = color_from_parts(active.sun_color);
    fog.directional_light_exponent = active.fog_sun_exponent;
    fog.falloff = FogFalloff::from_visibility(active.fog_visibility_m);

    bloom.intensity = active.bloom_intensity;
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
        let roll = rand_roll(s.wrapping_mul(7));

        // Pose and fade it exactly as `update_smoke` would on its first tick —
        // otherwise `commands.spawn` doesn't land until the next sync point, so
        // this frame's render sees the entity before `update_smoke` ever runs
        // on it: the default identity rotation (not facing the camera) and the
        // material's default opaque white, i.e. one full-opacity, wrong-facing
        // frame before the fade-in/billboard even starts.
        let cam_up = cam_rotation * Vec3::Y;
        let cam_right_fallback = cam_rotation * Vec3::X;
        let rotation = smoke_billboard_rotation(origin, player.translation, cam_up, cam_right_fallback, roll);
        let alpha = peak_alpha * smoke_envelope(0.0, fade_in, lifetime);

        let material = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, alpha),
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
                roll,
                peak_alpha,
            },
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(origin)
                .with_rotation(rotation)
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
        transform.rotation = smoke_billboard_rotation(
            transform.translation,
            cam_pos,
            cam_up,
            cam_right_fallback,
            particle.roll,
        );
        transform.scale = scale;

        let alpha = particle.peak_alpha * smoke_envelope(particle.age, particle.fade_in, particle.lifetime);
        if let Some(material) = materials.get_mut(&material.0) {
            material.base_color = Color::srgba(1.0, 1.0, 1.0, alpha);
        }
    }
}

/// Spherical-billboard rotation for a smoke puff at `pos`: local +Z points
/// straight at `cam_pos`, up follows `cam_up` (falling back to
/// `cam_right_fallback` on the degenerate case where the puff sits dead-on
/// with the camera's up axis), then spun by `roll` around that facing axis.
/// Shared by `emit_smoke` (the puff's pose the instant it spawns) and
/// `update_smoke` (every frame after) so a fresh puff is never caught one
/// frame short of `update_smoke` reaching it — `commands.spawn` doesn't land
/// until the next sync point, so without this a brand-new puff would render
/// at the identity rotation for a frame.
fn smoke_billboard_rotation(
    pos: Vec3,
    cam_pos: Vec3,
    cam_up: Vec3,
    cam_right_fallback: Vec3,
    roll: f32,
) -> Quat {
    let to_cam = cam_pos - pos;
    if to_cam.length_squared() <= 1.0e-6 {
        return Quat::IDENTITY;
    }
    let normal = to_cam.normalize();
    let mut right = cam_up.cross(normal);
    if right.length_squared() < 1.0e-6 {
        right = cam_right_fallback;
    }
    let right = right.normalize();
    let up = normal.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, up, normal)) * Quat::from_rotation_z(roll)
}

/// A smoke puff's opacity envelope at `age`: ramps 0 -> 1 over `fade_in`, then
/// 1 -> 0 over the rest of `lifetime`. Shared by `emit_smoke` (so a puff's
/// material starts at the same alpha this gives for `age: 0.0`, rather than
/// the material's default opaque white for the one frame before
/// `update_smoke` first reaches it) and `update_smoke`.
fn smoke_envelope(age: f32, fade_in: f32, lifetime: f32) -> f32 {
    let envelope = if age < fade_in {
        age / fade_in.max(1.0e-4)
    } else {
        let fade_out = (lifetime - fade_in).max(1.0e-4);
        1.0 - (age - fade_in) / fade_out
    };
    envelope.clamp(0.0, 1.0)
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

/// Keep the bottom-right readout in sync with the ammo counts.
fn update_ammo_ui(weapon: Res<Weapon>, mut text: Single<&mut Text, With<AmmoText>>) {
    let wanted = format!("{} / {}", weapon.mag, weapon.reserve);
    if text.0 != wanted {
        text.0 = wanted;
    }
}
