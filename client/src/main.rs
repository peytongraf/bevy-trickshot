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
mod environment;
mod hud;
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
mod vfx;
mod weapons;

use audio::*;
use avatars::*;
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
    /// In the shared world (or solo Practice).
    InGame,
}

use std::f32::consts::{FRAC_PI_2, PI};

use keybinds::KeyBindings;
use settings::Settings;

use bevy::{
    audio::SpatialListener,
    core_pipeline::bloom::Bloom,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    math::{Affine2, FloatExt},
    pbr::{CascadeShadowConfigBuilder, DistanceFog, FogFalloff, NotShadowCaster},
    prelude::*,
    render::{
        render_asset::RenderAssetUsages,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::{NoFrustumCulling, RenderLayers},
    },
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
pub(crate) const UI_LAYER: usize = 4;

/// Bold condensed display face used across the in-game HUD (score, ammo, fps,
/// score popups) and the kill-cam banner — the closest free/open stand-in for
/// the tall all-caps look modern Call of Duty titles use for this kind of text.
pub(crate) const HUD_FONT: &str = "fonts/BebasNeue-Regular.ttf";

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

/// Server told us the match clock ran out.
#[derive(Event)]
pub(crate) struct MatchEndedEvent {
    pub(crate) winner: String,
    pub(crate) score: u32,
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
