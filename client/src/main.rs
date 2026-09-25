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

mod audio;
mod avatars;
mod changelog;
mod death_effect;
mod debug_ui;
mod environment;
mod fall_death;
mod game_start;
mod health;
mod hit_marker;
mod hud;
mod keybinds;
mod killcam;
mod knife_sounds;
mod lobby_ui;
mod match_end;
mod menu;
mod net;
mod pause;
mod player;
mod respawn;
mod settings;
mod thrown_knife;
mod ui;
mod updater;
mod util;
mod vfx;
mod weapons;
mod zombies_hud;

use audio::*;
use avatars::*;
use debug_ui::*;
use environment::*;
use hud::*;
use player::*;
use util::{color_from_parts, rand_roll};
use vfx::*;
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
    /// In the shared world.
    InGame,
}

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::{
    audio::SpatialListener,
    core_pipeline::bloom::Bloom,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    math::Affine2,
    pbr::{CascadeShadowConfigBuilder, DistanceFog, FogFalloff, NotShadowCaster},
    prelude::*,
    render::{
        camera::CameraOutputMode,
        render_asset::RenderAssetUsages,
        render_resource::{BlendState, Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::{NoFrustumCulling, RenderLayers},
    },
    window::{CursorGrabMode, PrimaryWindow},
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass};
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
pub(crate) const UI_LAYER: usize = 4;

/// Bold condensed display face used across the in-game HUD (score, ammo, fps,
/// score popups) and the kill-cam banner — the closest free/open stand-in for
/// the tall all-caps look modern Call of Duty titles use for this kind of text.
pub(crate) const HUD_FONT: &str = "fonts/BebasNeue-Regular.ttf";

/// Plain readable face for blocks of body text (the main-menu "What's New"
/// notes, and similar copy) where [`HUD_FONT`]'s tall condensed caps get hard
/// to read at length. Inter, a clean neutral sans close to the system UI face
/// on macOS.
pub(crate) const BODY_FONT: &str = "fonts/Inter-Variable.ttf";

fn main() {
    // Check for a newer release and, if there is one, replace this executable and
    // relaunch before Bevy starts. No-op under `cargo run`. See src/updater.rs.
    updater::bootstrap();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy-trickshot".into(),
                // Matches `Settings::vsync`'s own default (off) so there's no
                // startup flash of vsync-on-then-off; `settings::apply_vsync`
                // takes over and corrects this to match the player's saved
                // choice from here on. See that function's doc comment for
                // why off is the default: Bevy's own default, `Fifo` (vsync
                // on), queues up to ~3 frames before display and caps the
                // render rate to the monitor's refresh rate — extra
                // input-to-screen latency and frame-pacing judder that reads
                // as sluggish mouse look next to an uncapped shooter like
                // Call of Duty.
                present_mode: bevy::window::PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        // Egui lives on the HUD camera (`setup_ui_camera`), not whichever
        // camera egui finds first — on a 3D camera it's drawn before (and
        // warped by) the shroom post-process and depends on spawn order.
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
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
            killcam::KillCamPlugin,
            death_effect::DeathEffectPlugin,
            fall_death::FallDeathPlugin,
            game_start::GameStartPlugin,
            thrown_knife::ThrownKnifePlugin,
            knife_sounds::KnifeSoundsPlugin,
            health::HealthPlugin,
            hit_marker::HitMarkerPlugin,
            match_end::MatchEndPlugin,
            respawn::RespawnResetPlugin,
            BulletHolePlugin,
        ))
        .add_plugins(ShroomPlugin)
        // After `ShroomPlugin`: its pass chains onto the shroom one.
        .add_plugins(DrunkPlugin)
        .add_plugins(ShroomXrayPlugin)
        .add_plugins(zombies_hud::ZombiesHudPlugin)
        .add_plugins(pause::PausePlugin)
        .add_plugins(DrinkArmsPlugin)
        .insert_resource(AmbientLight {
            color: SKY_AMBIENT_COLOR,
            brightness: SKY_AMBIENT_LUX,
            ..default()
        })
        .init_resource::<ViewModelPoses>()
        .init_resource::<KnifeViewModelSettings>()
        .init_resource::<ThrowArmsSettings>()
        .init_resource::<ThrowKnifeModelSettings>()
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
        .init_resource::<LensSettings>()
        .init_resource::<TeleportPoint>()
        .init_resource::<NoScopeSpread>()
        .init_resource::<Weapon>()
        .init_resource::<ThrowingKnife>()
        .init_resource::<KnifeAnimState>()
        .init_resource::<PendingShot>()
        .init_resource::<PendingMelee>()
        .add_event::<LocalShot>()
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
        .init_resource::<Mantle>()
        .init_resource::<MantleSettings>()
        .init_resource::<FootstepSettings>()
        .init_resource::<FootstepState>()
        .init_resource::<SoundVolumes>()
        .init_resource::<SceneTuning>()
        .init_resource::<ShipmentSceneTuning>()
        .init_resource::<ShipmentDaySceneTuning>()
        .init_resource::<BreakPointSceneTuning>()
        .init_resource::<MapSettings>()
        .init_resource::<CurrentMap>()
        .init_resource::<MapLoadState>()
        .init_resource::<ShipmentSettings>()
        .init_resource::<WaterSettings>()
        .init_resource::<ShipmentLightSettings>()
        .init_resource::<FluoroLightSettings>()
        .init_resource::<BulbLightSettings>()
        .init_resource::<RainSettings>()
        .init_resource::<RemoteAvatarSettings>()
        .init_resource::<SniperGlintSettings>()
        .init_resource::<NameTagSettings>()
        .init_resource::<BotLookSettings>()
        .init_resource::<LedgeJumpSettings>()
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
                setup_soldier_assets,
                setup_rain,
            ),
        )
        .add_systems(PostStartup, setup_sniper_glint_assets)
        .add_systems(
            OnEnter(AppState::InGame),
            (
                grab_cursor,
                start_ambient.after(lobby_ui::sync_current_map),
                reset_slide,
                reset_trick,
                reset_weapon,
                spawn_debug_readout,
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
        .add_systems(Update, spawn_name_tags.run_if(in_state(AppState::InGame)))
        .add_systems(
            Update,
            // Right after mouse look, under the same conditions (no menu, kill
            // cam, fall death or death look-at in charge of the view).
            shroom_aim_assist.after(look_around).run_if(
                in_state(AppState::InGame)
                    .and(menu::game_active)
                    .and(killcam::no_killcam)
                    .and(fall_death::no_fall_death)
                    .and(not(death_effect::death_effect_active))
                    .and(not(fall_death::effect_active)),
            ),
        )
        .add_systems(
            Update,
            ping_bots.run_if(
                in_state(AppState::InGame)
                    .and(menu::game_active)
                    .and(killcam::no_killcam),
            ),
        )
        .add_systems(
            PostUpdate,
            (apply_name_tag_scale, update_name_tags)
                .before(bevy::ui::UiSystem::Layout)
                .run_if(in_state(AppState::InGame)),
        )
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
            update_ads.run_if(
                in_state(AppState::InGame)
                    .and(killcam::no_killcam)
                    .and(fall_death::no_fall_death),
            ),
        )
        .add_systems(
            Update,
            (
                // Looks for a ledge to catch *before* this frame's normal
                // movement/gravity chain runs, so a frame that starts a climb
                // doesn't also fall/collide normally — see `not_mantling`,
                // which that chain is gated on right below.
                try_mantle.before(toggle_sprint).run_if(
                    menu::game_active
                        .and(killcam::no_killcam)
                        .and(fall_death::no_fall_death)
                        .and(not_mantling),
                ),
                drive_mantle.run_if(
                    menu::game_active
                        .and(killcam::no_killcam)
                        .and(fall_death::no_fall_death),
                ),
            )
                .run_if(in_state(AppState::InGame)),
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
                    resolve_body_collisions,
                    teleport_home,
                    jump,
                    apply_gravity,
                    // Reads this frame's velocity + grounded state, so last in
                    // the chain.
                    footsteps,
                )
                    .chain()
                    .run_if(
                        menu::game_active
                            .and(killcam::no_killcam)
                            .and(fall_death::no_fall_death)
                            .and(not_mantling),
                    ),
                (
                    // Additionally defers to the death-effect's / fall-death's
                    // own forced look-at, so mouse input can't fight it during
                    // either brief window (see `death_effect` / `fall_death`).
                    look_around.run_if(
                        not(death_effect::death_effect_active).and(not(fall_death::effect_active)),
                    ),
                    save_teleport_point,
                )
                    .run_if(
                        menu::game_active
                            .and(killcam::no_killcam)
                            .and(fall_death::no_fall_death),
                    ),
                weapon_system.run_if(
                    menu::game_active
                        .and(killcam::no_killcam)
                        .and(fall_death::no_fall_death)
                        .and(not_mantling),
                ),
                // Visuals / HUD — keep running so shake, smoke and the scope
                // settle even while paused.
                apply_ads,
                // After `weapon_sway` (and, via the kill cam's own ordering, its
                // replayed sway) so the reticle's counter-sway reads the same
                // frame's offset the gun was just tipped by.
                update_scope.after(aim_idle_sway).after(weapon_sway),
                (fade_crosshair, update_crosshair_visibility),
                // Must read `LookDelta` before `consume_look_delta` (registered
                // in a separate `add_systems` below) zeroes it for the frame.
                track_trick
                    .after(look_around)
                    .before(consume_look_delta)
                    .run_if(killcam::no_killcam.and(fall_death::no_fall_death)),
                (
                    spawn_score_popup,
                    update_score_popups,
                    update_teleport_toast,
                ),
                (sky_follow_camera, scroll_water_normal),
                camera_shake.run_if(killcam::no_killcam.and(fall_death::no_fall_death)),
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
                (apply_loadout, update_ammo_ui, update_knife_hud, update_weapon_icon, scale_ammo_hud).chain(),
                (update_fps_ui, update_debug_readout),
                (apply_scene_tuning, sync_scope_fog).chain(),
                (apply_shadow_quality, apply_crosshair_texture),
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
                    (slide_throw_arms, apply_throw_arms_transform, update_throw_knife_model).chain(),
                ),
                (debug_cursor_toggle, log_player_position),
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
                (
                    weapon_sway,
                    knife_weapon_sway,
                    throw_arms_weapon_sway,
                    idle_weapon_sway,
                    knife_idle_sway,
                    throw_arms_idle_sway,
                    weapon_recoil_shudder,
                )
                    .chain(),
                aim_idle_sway,
            )
                .after(look_around)
                .after(apply_ads)
                .after(apply_knife_transform)
                .after(apply_throw_arms_transform)
                .run_if(in_state(AppState::InGame).and(killcam::no_killcam)),
        )
        .add_systems(Update, resolve_local_shot.run_if(in_state(AppState::InGame)))
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
    // (see `build_ground_texture`), tiled every ~2 m. Spawned hidden with its
    // `Collider` disabled — every current map ships its own ground mesh now
    // (`basic_map.glb`'s `Plane` node, `shipment.glb`'s `Ground` node), which
    // already gets real collision for free from `sync_map_model`'s
    // whole-scene `AsyncSceneCollider`. Leaving this one active too would
    // stack two flat, coincident colliders at the same height, and
    // `apply_gravity`'s raycast can then land on either from one frame to the
    // next — flipping which of two near-identical surface heights it reports
    // for the same spot re-triggers grounded-edge detection (and so the
    // landing sound) every time, even standing still on flat ground. Kept
    // around (rather than deleted outright) as a ready-made fallback floor
    // for a future map that doesn't ship its own ground — see
    // `sync_shipment_only_visibility`, which no longer touches it now that
    // no current map needs it shown.
    commands.spawn((
        ProceduralGround,
        Visibility::Hidden,
        ColliderDisabled,
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

    // Targets in the world are the server-owned bots.

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
    arms_settings: Res<ThrowArmsSettings>,
    drink_settings: Res<DrinkArmsSettings>,
    throw_knife_settings: Res<ThrowKnifeModelSettings>,
    settings: Res<settings::Settings>,
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

    // And the throwing arms' single `throw` clip.
    let arms_clip: Handle<AnimationClip> = asset_server
        .load(GltfAssetLabel::Animation(0).from_asset("models/arms_throwing.glb"));
    let (arms_graph, arms_index) = AnimationGraph::from_clip(arms_clip);
    let arms_graph = graphs.add(arms_graph);

    // And the drinking arms' single clip (trimmed at playback — see
    // `weapons::drink_arms`).
    let drink_clip: Handle<AnimationClip> = asset_server
        .load(GltfAssetLabel::Animation(0).from_asset("models/arms_drinking.glb"));
    let (drink_graph, drink_index) = AnimationGraph::from_clip(drink_clip);
    let drink_graph = graphs.add(drink_graph);
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

    // Reticle quad shown only inside the scope image — texture picked by
    // `Settings::crosshair` (Loadout screen); `apply_crosshair_texture` swaps
    // it live from here on if the setting changes.
    let reticle_mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let reticle_material = materials.add(StandardMaterial {
        base_color_texture: Some(asset_server.load(crosshair_asset_path(settings.crosshair))),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(ScopeReticleMaterial(reticle_material.clone()));

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
                                            // Explicit, not left to Bevy: with no blend
                                            // set, Bevy makes whichever window camera it
                                            // happens to iterate first opaque and blends
                                            // the rest — and that order shifts whenever
                                            // systems/plugins are added. When the HUD
                                            // camera lands first, its blit paints over
                                            // the whole 3D view (black world, HUD only).
                                            // So: world opaque, the rest alpha-blended.
                                            output_mode: CameraOutputMode::Write {
                                                blend_state: Some(BlendState::REPLACE),
                                                clear_color: ClearColorConfig::Default,
                                            },
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
                                            fov: LENS_CAL_SCOPE_FOV_DEG.to_radians(),
                                            ..default()
                                        }),
                                        // Placeholder: `sync_scope_fog` copies the
                                        // world camera's fog onto this every time
                                        // the map's scene tuning changes it.
                                        DistanceFog::default(),
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
                                            // See the world camera's `output_mode`.
                                            output_mode: CameraOutputMode::Write {
                                                blend_state: Some(BlendState::ALPHA_BLENDING),
                                                clear_color: ClearColorConfig::Default,
                                            },
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

                            // The throwing arms — shown while the throwing-
                            // knife key is held / mid-throw (see
                            // `ThrowingKnife`), hidden otherwise.
                            rig.spawn((
                                ThrowArmsViewModel,
                                ThrowArmsAnimation {
                                    graph: arms_graph,
                                    index: arms_index,
                                },
                                SceneRoot(asset_server.load(
                                    GltfAssetLabel::Scene(0).from_asset("models/arms_throwing.glb"),
                                )),
                                // Spawned fully slid out of view.
                                arms_settings.transform(0.0),
                                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                                Visibility::Hidden,
                            ))
                            .observe(start_throw_arms_animation)
                            // The knife in the arms' hand: a child, so it rides
                            // along with the arms (slide, future sway) and is
                            // positioned once in their local space.
                            .with_children(|arms| {
                                arms.spawn((
                                    ThrowKnifeModel,
                                    SceneRoot(asset_server.load(
                                        GltfAssetLabel::Scene(0)
                                            .from_asset("models/throwing_knife.glb"),
                                    )),
                                    throw_knife_settings.transform(),
                                    RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                                    Visibility::Hidden,
                                ))
                                .observe(start_throw_knife_model);
                            });

                            // The perk-drinking arms — debug-only for now,
                            // shown from the "Drinking arms" panel section.
                            rig.spawn((
                                DrinkArmsViewModel,
                                DrinkArmsAnimation {
                                    graph: drink_graph,
                                    index: drink_index,
                                },
                                SceneRoot(asset_server.load(
                                    GltfAssetLabel::Scene(0).from_asset("models/arms_drinking.glb"),
                                )),
                                drink_settings.transform(),
                                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                                Visibility::Hidden,
                            ))
                            .observe(start_drink_arms_animation);

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

/// Server told us the match clock ran out.
#[derive(Event)]
pub(crate) struct MatchEndedEvent {
    pub(crate) winner: String,
    pub(crate) score: u32,
}

// ---------------------------------------------------------------------------
// Update systems
// ---------------------------------------------------------------------------

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
