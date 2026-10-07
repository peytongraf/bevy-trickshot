//! Molotovs, client side (`Zombies` only — the server owns the flight, the
//! fire's spread and damage, and the drops; see `server::molotovs` and
//! `shared::molotov`):
//!
//! * **Held** — with the molotov as the lethal (`Weapon::lethal`), holding
//!   the lethal key brings up the throwing arms holding
//!   `models/weapons/molotov_2k.glb` ([`HeldMolotov`], a child of the arms),
//!   its rag burning ([`RagFlame`]) and lighting the scene around the player.
//!   The throw itself is the throwing knife's sequence (`weapon.rs`); only
//!   the request sent at the release point differs ([`send_throw_requests`]).
//! * **Thrown** — every player sees each [`ThrownMolotov`] fly as
//!   `models/weapons/molotov_1k.glb`, rag alight ([`MolotovAvatar`]).
//! * **Burning** — where one broke, a patch of flames over the server's
//!   fire spots ([`FirePatch`]), with a flickering light, fading in and out.
//! * **Nuked zombies** — a zombie a Nuke kills ([`shared::ZombieNuked`])
//!   burns where it falls for a few seconds ([`CorpseFire`]): flames laid
//!   along its body, which lies ahead of its feet when it falls face-first
//!   and behind them when it falls back. Only a look — it hurts no one.
//! * **Dropped** — a zombie's dropped molotov lies on its side, outlined in
//!   blue like a dropped throwing knife ([`DropAvatar`]); the pickup card and
//!   key are `knife_pickup`'s. Picking one up ([`receive_pickups`]) makes the
//!   molotov the lethal.
//!
//! The flames are camera-facing quads drawn with [`FireMaterial`]
//! (`assets/shaders/fire.wgsl`). Everything's look and placement is tuned in
//! the debug panel's "Molotov (Zombies)" section ([`molotov_section`]).
//!
//! Reset between games: every entity here is `StateScoped(InGame)` or a child
//! of one (the held molotov rides the arms rig), and each avatar goes as soon
//! as its server entity does; nuked bodies still waiting to fall
//! ([`PendingCorpseFires`]) are forgotten on leaving the game; the molotov count and lethal live on `Weapon`,
//! which resets with the game.

use std::f32::consts::FRAC_PI_2;

use bevy::ecs::system::SystemParam;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy_egui::egui;
use lightyear::prelude::*;

use shared::{MolotovDrop, MolotovFire, PlayerId, PlayerPose, ThrownMolotov, ZombieNuked};

use crate::killcam::ActiveKillCam;
use crate::knife_pickup::OutlineWhenLoaded;
use crate::net::{GameClient, RemoteAvatar};
use crate::pause::GamePaused;
use crate::{
    start_throw_knife_model, AppState, GameSounds, ThrowArmsSettings, ThrowArmsViewModel,
    ThrowingKnife, Weapon, VIEW_MODEL_RENDER_LAYER,
};

const SHADER_ASSET_PATH: &str = "shaders/fire.wgsl";
const HELD_MODEL: &str = "models/weapons/molotov_2k.glb";
const WORLD_MODEL: &str = "models/weapons/molotov_1k.glb";
/// The bottle's radius in model units (`molotov_*.glb` is ~13 units tall,
/// base at the origin, neck up `+Y`).
const BOTTLE_RADIUS_UNITS: f32 = 1.727;
/// How long (s) a fire takes to flare up to full.
const FIRE_FADE_IN_SECS: f32 = 0.35;
/// How long (s) a fire takes to die down at the end.
const FIRE_FADE_OUT_SECS: f32 = 1.2;

/// Flame layers on a burning rag (overlapping, each with its own seed, so
/// the flame has depth instead of reading as one flat sprite).
const RAG_LAYERS: usize = 3;
/// Each rag layer's `(width, height)` relative to the rag flame's size, and
/// its seed.
const RAG_LAYER_SHAPES: [(f32, f32, f32); RAG_LAYERS] =
    [(1.0, 1.0, 0.17), (0.72, 0.88, 0.53), (0.5, 0.68, 0.81)];

use params::FireParams;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct (as in `power_ups`).
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Mirrors `FireParams` in the shader.
    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct FireParams {
        pub(crate) core: Vec4,
        pub(crate) mid: Vec4,
        pub(crate) tip: Vec4,
        pub(crate) wind: Vec4,
        pub(crate) smoke: Vec4,
        pub(crate) brightness: f32,
        pub(crate) speed: f32,
        pub(crate) turbulence: f32,
        pub(crate) detail: f32,
        pub(crate) fade: f32,
        pub(crate) embers: f32,
        pub(crate) sink: f32,
        pub(crate) pull: f32,
        pub(crate) mode: f32,
    }
}

/// Flames (additive) or smoke (alpha blended, `mode` 1) — see `fire.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct FireMaterial {
    #[uniform(0)]
    params: FireParams,
}

impl Material for FireMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.params.mode > 0.5 {
            AlphaMode::Blend
        } else {
            AlphaMode::Add
        }
    }

    /// A billboard: both faces, and no depth write (it's light / smoke, not
    /// a surface) — walls in front still hide it.
    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = false;
        }
        Ok(())
    }
}

/// PhD Flopper's purple flames (its slide trail, `vfx::phd_trail`): linear
/// rgb of the hottest part, the body and the cooling tips — the molotov's
/// fire, recoloured.
const PHD_CORE: [f32; 3] = [1.0, 0.78, 1.0];
const PHD_MID: [f32; 3] = [0.55, 0.08, 1.0];
const PHD_TIP: [f32; 3] = [0.18, 0.0, 0.42];

impl FireMaterial {

    /// A flame like the molotov's (`settings`' look) in PhD Flopper's
    /// purple, `brightness` bright and faded in to `fade` (0..1).
    pub(crate) fn phd_flame(settings: &MolotovSettings, brightness: f32, fade: f32) -> Self {
        let c = |v: [f32; 3]| Vec4::new(v[0], v[1], v[2], 1.0);
        Self {
            params: FireParams {
                core: c(PHD_CORE),
                mid: c(PHD_MID),
                tip: c(PHD_TIP),
                ..settings.params(brightness, settings.pull, fade, Vec3::ZERO)
            },
        }
    }

    /// Set how far it's faded in (0..1).
    pub(crate) fn set_fade(&mut self, fade: f32) {
        self.params.fade = fade;
    }

    /// A flame in the molotov's own look (`settings`), `brightness` bright,
    /// its top bent by `wind` (m per m of height) — for fires elsewhere (a
    /// hellhound's back, `dogs`).
    pub(crate) fn flame(settings: &MolotovSettings, brightness: f32, wind: Vec3) -> Self {
        Self {
            params: settings.params(brightness, 0.0, 1.0, wind),
        }
    }
}

/// A flame / smoke quad's scale: `size` (m) across and up, and the seed the
/// shader reads back as z ÷ x − 1 — so each quad keeps its own look however
/// it moves or its parent is scaled.
pub(crate) fn seeded_scale(size: Vec2, seed: f32) -> Vec3 {
    let seed = 0.05 + seed.fract() * 0.9;
    Vec3::new(size.x, size.y, size.x * (1.0 + seed))
}

/// The debug panel's molotov tuning.
#[derive(Resource, Clone)]
pub(crate) struct MolotovSettings {
    // --- held in the throwing arms (the arms' local space, like
    // `ThrowKnifeModelSettings`: one unit = the arms' scale in metres) ---
    pub(crate) held_translation: Vec3,
    pub(crate) held_yaw: f32,
    pub(crate) held_pitch: f32,
    pub(crate) held_roll: f32,
    pub(crate) held_scale: f32,

    // --- the burning rag (held and thrown) ---
    /// Where the rag's flame sits, in the molotov model's own units (the
    /// bottle is ~13 tall, neck up `+Y`).
    pub(crate) rag_offset: Vec3,
    /// Flame size (m).
    pub(crate) rag_width: f32,
    pub(crate) rag_height: f32,
    /// How many overlapping flame layers burn on the rag (1..=3).
    pub(crate) rag_layers: u32,
    pub(crate) rag_brightness: f32,
    /// The light the burning rag gives off (lumens; range m).
    pub(crate) rag_light: f32,
    pub(crate) rag_light_range: f32,
    /// How far a moving flame trails behind: lean (m per m of flame height)
    /// per m/s of speed.
    pub(crate) trail: f32,
    /// The most a flame leans (m per m of height).
    pub(crate) max_lean: f32,

    // --- thrown / dropped model ---
    /// Model scale in the world (the bottle's ~13 units → ~28 cm).
    pub(crate) world_scale: f32,

    // --- the fire on the ground ---
    /// Flame size (m) — each flame is picked somewhere between 70 % and
    /// 100 % of it.
    pub(crate) flame_width: f32,
    pub(crate) flame_height: f32,
    /// Flames per fire spot (the server spreads ~35 spots over the patch).
    pub(crate) flames_per_spot: u32,
    /// How far (m) each flame may stray from its spot.
    pub(crate) flame_jitter: f32,
    pub(crate) fire_brightness: f32,
    /// The fire's light (lumens; range m; height m above the ground).
    pub(crate) fire_light: f32,
    pub(crate) fire_light_range: f32,
    pub(crate) fire_light_height: f32,
    pub(crate) fire_light_shadows: bool,
    /// Columns of smoke rising off a fire (0 = none).
    pub(crate) smoke_columns: u32,
    /// Smoke column size (m).
    pub(crate) smoke_width: f32,
    pub(crate) smoke_height: f32,
    /// Smoke colour (linear) and opacity.
    pub(crate) smoke_color: [f32; 3],
    pub(crate) smoke_opacity: f32,

    // --- fire on a nuked zombie's body ---
    /// How far (m) from its feet the middle of a body lies once it's fallen
    /// face-first (ahead of them) / onto its back (behind them).
    pub(crate) corpse_forward_offset: f32,
    pub(crate) corpse_back_offset: f32,
    /// Along the body (m) and across it, that the flames are spread over.
    pub(crate) corpse_length: f32,
    pub(crate) corpse_width: f32,
    pub(crate) corpse_flames: u32,
    /// Each flame's size (m).
    pub(crate) corpse_flame_width: f32,
    pub(crate) corpse_flame_height: f32,
    /// How long (s) the body burns, the last `corpse_fade_secs` of it dying
    /// down (and the first `corpse_grow_secs` flaring up, as it falls).
    pub(crate) corpse_fire_secs: f32,
    pub(crate) corpse_grow_secs: f32,
    pub(crate) corpse_fade_secs: f32,
    pub(crate) corpse_brightness: f32,
    /// Its light (lumens; range m).
    pub(crate) corpse_light: f32,
    pub(crate) corpse_light_range: f32,

    // --- the flames' look (all of them) ---
    /// Linear rgb of the hottest part, the body and the tips.
    pub(crate) core_color: [f32; 3],
    pub(crate) mid_color: [f32; 3],
    pub(crate) tip_color: [f32; 3],
    pub(crate) speed: f32,
    pub(crate) turbulence: f32,
    pub(crate) detail: f32,
    pub(crate) embers: f32,
    /// How far below its anchor a flame's base starts (fraction of height).
    pub(crate) sink: f32,
    /// How far (m) a ground flame is pulled toward the camera.
    pub(crate) pull: f32,
    /// Light colour (sRGB).
    pub(crate) light_color: [f32; 3],
    /// How much every light flickers (0 = steady, 1 = wild).
    pub(crate) flicker: f32,
}

impl Default for MolotovSettings {
    fn default() -> Self {
        Self {
            held_translation: Vec3::new(-20.0, -18.0, 0.1),
            held_yaw: -50.0,
            held_pitch: -35.0,
            held_roll: -10.0,
            held_scale: 1.8,

            rag_offset: Vec3::new(0.0, 13.8, 0.01),
            rag_width: 0.09,
            rag_height: 0.21,
            rag_layers: 3,
            rag_brightness: 0.9,
            rag_light: 60_000.0,
            rag_light_range: 6.0,
            trail: 0.12,
            max_lean: 1.4,

            world_scale: 0.022,

            flame_width: 1.7,
            flame_height: 1.5,
            flames_per_spot: 2,
            flame_jitter: 0.35,
            fire_brightness: 0.8,
            fire_light: 700_000.0,
            fire_light_range: 16.0,
            fire_light_height: 1.2,
            fire_light_shadows: false,
            smoke_columns: 4,
            smoke_width: 2.6,
            smoke_height: 7.0,
            smoke_color: [0.05, 0.047, 0.045],
            smoke_opacity: 0.55,

            corpse_forward_offset: 0.85,
            corpse_back_offset: 0.85,
            corpse_length: 1.5,
            corpse_width: 0.45,
            corpse_flames: 9,
            corpse_flame_width: 0.7,
            corpse_flame_height: 0.9,
            corpse_fire_secs: 3.0,
            corpse_grow_secs: 0.6,
            corpse_fade_secs: 1.4,
            corpse_brightness: 0.8,
            corpse_light: 150_000.0,
            corpse_light_range: 7.0,

            core_color: [1.0, 0.78, 0.42],
            mid_color: [1.0, 0.33, 0.04],
            tip_color: [0.5, 0.04, 0.0],
            speed: 1.0,
            turbulence: 1.0,
            detail: 2.2,
            embers: 2.0,
            sink: 0.06,
            pull: 0.25,
            light_color: [1.0, 0.6, 0.28],
            flicker: 0.35,
        }
    }
}

impl MolotovSettings {
    fn held_transform(&self) -> Transform {
        Transform {
            translation: self.held_translation,
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                self.held_yaw.to_radians(),
                self.held_pitch.to_radians(),
                self.held_roll.to_radians(),
            ),
            scale: Vec3::splat(self.held_scale),
        }
    }

    fn params(&self, brightness: f32, pull: f32, fade: f32, wind: Vec3) -> FireParams {
        let c = |v: [f32; 3]| Vec4::new(v[0], v[1], v[2], 1.0);
        FireParams {
            core: c(self.core_color),
            mid: c(self.mid_color),
            tip: c(self.tip_color),
            wind: wind.extend(0.0),
            smoke: Vec4::new(
                self.smoke_color[0],
                self.smoke_color[1],
                self.smoke_color[2],
                self.smoke_opacity,
            ),
            brightness,
            speed: self.speed,
            turbulence: self.turbulence,
            detail: self.detail,
            fade,
            embers: self.embers,
            sink: self.sink,
            pull,
            mode: 0.0,
        }
    }

    fn smoke_params(&self, fade: f32) -> FireParams {
        FireParams {
            mode: 1.0,
            sink: 0.0,
            ..self.params(1.0, self.pull, fade, Vec3::ZERO)
        }
    }

    pub(crate) fn light_color(&self) -> Color {
        Color::srgb(self.light_color[0], self.light_color[1], self.light_color[2])
    }

    /// How far a flame moving at `vel` (m/s) leans (m per m of height).
    pub(crate) fn lean(&self, vel: Vec3) -> Vec3 {
        (-vel * self.trail).clamp_length_max(self.max_lean)
    }

    /// Rag flame layer `layer`'s transform under a parent whose world scale
    /// is `parent_scale` — its size stays in metres however big the model is.
    fn rag_flame_transform(&self, parent_scale: f32, layer: usize) -> Transform {
        let s = parent_scale.max(1e-6);
        let (w, h, seed) = RAG_LAYER_SHAPES[layer];
        Transform {
            translation: self.rag_offset,
            rotation: Quat::IDENTITY,
            scale: seeded_scale(Vec2::new(self.rag_width * w, self.rag_height * h) / s, seed),
        }
    }
}

/// A light's brightness multiplier this moment: a steady base with fast,
/// uneven wobble (`amount` 0..1), different per `seed`.
pub(crate) fn flicker(t: f32, seed: f32, amount: f32) -> f32 {
    let w = (t * 11.0 + seed * 3.1).sin() * 0.45
        + (t * 17.3 + seed * 7.7).sin() * 0.35
        + (t * 29.1 + seed * 1.9).sin() * 0.2;
    (1.0 + w * amount).max(0.0)
}

/// Shared handles: the flame quad, both models, and the held rag's flame
/// material (each thrown molotov and ground fire gets its own, for its own
/// lean / fade).
#[derive(Resource)]
pub(crate) struct MolotovAssets {
    /// (Shared with PhD Flopper's slide trail, `vfx::phd_trail`.)
    pub(crate) quad: Handle<Mesh>,
    /// Both models, loaded up front (and kept) so the first molotov picked
    /// up or thrown doesn't hitch.
    held_model: Handle<Scene>,
    world_model: Handle<Scene>,
    held_flame: Handle<FireMaterial>,
}

/// How fast the held molotov's rag is moving (m/s, smoothed) — walking,
/// turning and the arms' sway all swing it — so its flame trails behind.
#[derive(Resource, Default)]
struct HeldMotion {
    last: Option<Vec3>,
    vel: Vec3,
}

/// Smooth a flame's measured speed over a few frames (m/s).
fn track_velocity(last: &mut Option<Vec3>, vel: &mut Vec3, now: Vec3, dt: f32) {
    if dt <= 0.0 {
        return;
    }
    if let Some(prev) = *last {
        let raw = (now - prev) / dt;
        let k = 1.0 - (-12.0 * dt).exp();
        *vel = vel.lerp(raw, k);
    }
    *last = Some(now);
}

/// The molotov in the throwing arms' hand — a pivot (child of the arms)
/// carrying the model, its rag flames and light. Shown only while
/// `ThrowingKnife::molotov_in_hand`.
#[derive(Component)]
struct HeldMolotov;

/// One flame layer of a burning rag — on the held molotov or a thrown one.
#[derive(Component)]
struct RagFlame {
    layer: usize,
}

/// The light a burning rag gives off.
#[derive(Component)]
struct RagLight {
    seed: f32,
}

/// Stands in for one server-owned [`ThrownMolotov`].
#[derive(Component)]
struct MolotovAvatar {
    src: Entity,
    /// This bottle's rag flames' material (its own, for its own lean).
    material: Handle<FireMaterial>,
    last: Option<Vec3>,
    vel: Vec3,
}

/// The thrown model's scaled pivot (under a [`MolotovAvatar`]).
#[derive(Component)]
struct WorldBottle;

/// The flames, smoke and light of one [`MolotovFire`].
#[derive(Component)]
struct FirePatch {
    src: Entity,
    material: Handle<FireMaterial>,
    smoke: Handle<FireMaterial>,
    /// Seconds it's been burning here (not counting pauses).
    age: f32,
}

/// One flame of a [`FirePatch`], at its full size.
#[derive(Component)]
struct PatchFlame {
    size: Vec2,
    seed: f32,
}

/// One smoke column of a [`FirePatch`], at its full size.
#[derive(Component)]
struct PatchSmoke {
    size: Vec2,
    seed: f32,
}

/// A [`FirePatch`]'s light.
#[derive(Component)]
struct PatchLight {
    seed: f32,
}

/// Nuked zombies (by peer) whose bodies haven't been set alight yet — each
/// waits for its avatar to start falling (so we know which way), with how
/// long (s) it's waited; given up on after [`CORPSE_FIRE_WAIT_SECS`].
#[derive(Resource, Default)]
struct PendingCorpseFires(Vec<(PeerId, f32)>);

/// The longest (s) a nuked zombie's fire waits for its body to fall.
const CORPSE_FIRE_WAIT_SECS: f32 = 3.0;

/// The flames and light over one nuked zombie's body.
#[derive(Component)]
struct CorpseFire {
    material: Handle<FireMaterial>,
    /// Seconds it's been burning (not counting pauses).
    age: f32,
}

/// One flame of a [`CorpseFire`], at its full size.
#[derive(Component)]
struct CorpseFlame {
    size: Vec2,
    seed: f32,
}

/// A [`CorpseFire`]'s light.
#[derive(Component)]
struct CorpseLight {
    seed: f32,
}

/// Stands in for one [`MolotovDrop`].
#[derive(Component)]
struct DropAvatar {
    src: Entity,
}

pub(crate) struct MolotovPlugin;

impl Plugin for MolotovPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<FireMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .init_resource::<MolotovSettings>()
        .init_resource::<HeldMotion>()
        .init_resource::<PendingCorpseFires>()
        .add_systems(Startup, setup_assets)
        .add_systems(OnExit(AppState::InGame), |mut pending: ResMut<PendingCorpseFires>| {
            pending.0.clear();
        })
        .add_systems(
            Update,
            (
                spawn_held_molotov,
                update_held_molotov,
                send_throw_requests,
                sync_molotov_avatars,
                update_rag_flames,
                sync_fire_patches,
                update_fire_patches,
                sync_drop_avatars,
                receive_pickups,
                play_light_sound,
                receive_bursts,
                receive_nuked,
                spawn_corpse_fires,
                update_corpse_fires,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

fn setup_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FireMaterial>>,
    settings: Res<MolotovSettings>,
) {
    commands.insert_resource(MolotovAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        held_model: asset_server.load(GltfAssetLabel::Scene(0).from_asset(HELD_MODEL)),
        world_model: asset_server.load(GltfAssetLabel::Scene(0).from_asset(WORLD_MODEL)),
        held_flame: materials.add(FireMaterial {
            params: settings.params(settings.rag_brightness, 0.0, 1.0, Vec3::ZERO),
        }),
    });
}

/// Give each (new) throwing-arms rig its molotov: a pivot posed by
/// [`MolotovSettings`], carrying the 2k model, the rag's flame layers and its
/// light. On the view-model layer, except the light, which lights the world
/// (and the arms) around the player.
fn spawn_held_molotov(
    arms: Query<Entity, Added<ThrowArmsViewModel>>,
    settings: Res<MolotovSettings>,
    arms_settings: Res<ThrowArmsSettings>,
    assets: Res<MolotovAssets>,
    mut commands: Commands,
) {
    for arms in &arms {
        let vm = RenderLayers::layer(VIEW_MODEL_RENDER_LAYER);
        commands
            .spawn((
                HeldMolotov,
                settings.held_transform(),
                vm.clone(),
                Visibility::Hidden,
                ChildOf(arms),
            ))
            .with_children(|pivot| {
                // (The scene's own entities get the view-model layer once
                // it's spawned — only the model's subtree, not the light.)
                pivot
                    .spawn((
                        SceneRoot(assets.held_model.clone()),
                        Transform::default(),
                        vm.clone(),
                    ))
                    .observe(start_throw_knife_model);
                for layer in 0..RAG_LAYERS {
                    pivot.spawn((
                        RagFlame { layer },
                        Mesh3d(assets.quad.clone()),
                        MeshMaterial3d(assets.held_flame.clone()),
                        settings.rag_flame_transform(arms_settings.scale * settings.held_scale, layer),
                        vm.clone(),
                        Visibility::Inherited,
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                }
                pivot.spawn((
                    RagLight { seed: 0.0 },
                    PointLight {
                        color: settings.light_color(),
                        intensity: settings.rag_light,
                        range: settings.rag_light_range,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(settings.rag_offset),
                    RenderLayers::from_layers(&[0, VIEW_MODEL_RENDER_LAYER]),
                ));
            });
    }
}

/// Pose the held molotov from the settings every frame and show it only
/// while it's in the hand (from the key press until the throw starts). Hidden
/// during a kill cam. Also measures how fast its rag is moving.
fn update_held_molotov(
    time: Res<Time>,
    settings: Res<MolotovSettings>,
    knife: Res<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    mut motion: ResMut<HeldMotion>,
    mut held: Query<(&mut Transform, &mut Visibility, &GlobalTransform), With<HeldMolotov>>,
) {
    let shown = knife.molotov_in_hand() && killcam.0.is_none();
    let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
    for (mut tf, mut vis, gt) in &mut held {
        tf.set_if_neq(settings.held_transform());
        vis.set_if_neq(want);
        if shown {
            let rag = gt.transform_point(settings.rag_offset);
            let HeldMotion { last, vel } = &mut *motion;
            track_velocity(last, vel, rag, time.delta_secs());
        } else {
            *motion = HeldMotion::default();
        }
    }
}

/// Send the molotov throw `weapon_system` filed when it left the hand.
fn send_throw_requests(
    mut knife: ResMut<ThrowingKnife>,
    mut sender: Query<&mut TriggerSender<shared::ThrowMolotov>, With<GameClient>>,
) {
    if !knife.has_molotov_request() {
        return;
    }
    let Some((origin, dir)) = knife.take_throw_request() else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::ThrowMolotov {
            origin: origin.to_array(),
            dir: dir.to_array(),
        });
    }
}

/// Keep one avatar (the 1k model, its rag burning) per thrown molotov,
/// following the interpolated flight (and measuring its speed, for the
/// flame's trail); drop it once the server removes the bottle. Hidden during
/// a kill cam.
#[allow(clippy::too_many_arguments)]
fn sync_molotov_avatars(
    time: Res<Time>,
    molotovs: Query<(Entity, &ThrownMolotov), With<Interpolated>>,
    all: Query<&ThrownMolotov>,
    mut avatars: Query<(Entity, &mut MolotovAvatar, &mut Transform, &mut Visibility)>,
    killcam: Res<ActiveKillCam>,
    settings: Res<MolotovSettings>,
    assets: Res<MolotovAssets>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    let mut have = Vec::new();
    for (entity, mut avatar, mut tf, mut vis) in &mut avatars {
        match all.get(avatar.src) {
            Ok(m) => {
                tf.translation = m.pos;
                tf.rotation = m.rot;
                vis.set_if_neq(wanted);
                let MolotovAvatar { last, vel, .. } = &mut *avatar;
                track_velocity(last, vel, m.pos, time.delta_secs());
                have.push(avatar.src);
            }
            Err(_) => {
                materials.remove(&avatar.material);
                commands.entity(entity).try_despawn();
            }
        }
    }
    for (src, m) in &molotovs {
        if have.contains(&src) {
            continue;
        }
        let material = materials.add(FireMaterial {
            params: settings.params(settings.rag_brightness, 0.0, 1.0, Vec3::ZERO),
        });
        commands
            .spawn((
                StateScoped(AppState::InGame),
                MolotovAvatar {
                    src,
                    material: material.clone(),
                    last: None,
                    vel: Vec3::ZERO,
                },
                Transform::from_translation(m.pos).with_rotation(m.rot),
                Visibility::default(),
            ))
            .with_children(|avatar| {
                avatar
                    .spawn((
                        WorldBottle,
                        Transform::from_scale(Vec3::splat(settings.world_scale)),
                        Visibility::default(),
                    ))
                    .with_children(|bottle| {
                        bottle.spawn((SceneRoot(assets.world_model.clone()), Transform::default()));
                        for layer in 0..RAG_LAYERS {
                            bottle.spawn((
                                RagFlame { layer },
                                Mesh3d(assets.quad.clone()),
                                MeshMaterial3d(material.clone()),
                                settings.rag_flame_transform(settings.world_scale, layer),
                                Visibility::Inherited,
                                NotShadowCaster,
                                NoFrustumCulling,
                            ));
                        }
                        bottle.spawn((
                            RagLight {
                                seed: (src.index() % 97) as f32,
                            },
                            PointLight {
                                color: settings.light_color(),
                                intensity: settings.rag_light,
                                range: settings.rag_light_range,
                                shadows_enabled: false,
                                ..default()
                            },
                            Transform::from_translation(settings.rag_offset),
                        ));
                    });
            });
    }
}

/// Every burning rag (held and thrown) follows the live settings — flame
/// layers' placement, size and look, the flickering light — and leans away
/// from the way it's moving.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_rag_flames(
    time: Res<Time>,
    settings: Res<MolotovSettings>,
    arms_settings: Res<ThrowArmsSettings>,
    assets: Res<MolotovAssets>,
    motion: Res<HeldMotion>,
    mut materials: ResMut<Assets<FireMaterial>>,
    held: Query<(), With<HeldMolotov>>,
    avatars: Query<&MolotovAvatar>,
    mut bottles: Query<&mut Transform, (With<WorldBottle>, Without<RagFlame>, Without<RagLight>)>,
    mut flames: Query<(&RagFlame, &ChildOf, &mut Transform, &mut Visibility), Without<RagLight>>,
    mut lights: Query<(&RagLight, &mut PointLight, &mut Transform), Without<RagFlame>>,
) {
    if let Some(m) = materials.get_mut(&assets.held_flame) {
        m.params = settings.params(settings.rag_brightness, 0.0, 1.0, settings.lean(motion.vel));
    }
    for avatar in &avatars {
        if let Some(m) = materials.get_mut(&avatar.material) {
            m.params = settings.params(settings.rag_brightness, 0.0, 1.0, settings.lean(avatar.vel));
        }
    }
    for mut tf in &mut bottles {
        tf.scale = Vec3::splat(settings.world_scale);
    }
    let held_scale = arms_settings.scale * settings.held_scale;
    for (flame, parent, mut tf, mut vis) in &mut flames {
        let scale = if held.contains(parent.parent()) {
            held_scale
        } else {
            settings.world_scale
        };
        tf.set_if_neq(settings.rag_flame_transform(scale, flame.layer));
        vis.set_if_neq(if flame.layer < settings.rag_layers as usize {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let t = time.elapsed_secs();
    let color = settings.light_color();
    for (rag, mut light, mut tf) in &mut lights {
        light.intensity = settings.rag_light * flicker(t, rag.seed, settings.flicker);
        light.range = settings.rag_light_range;
        light.color = color;
        tf.translation = settings.rag_offset;
    }
}

/// Light a fire patch for each [`MolotovFire`] the server spreads: flames
/// scattered over its spots, a few columns of smoke rising off it and a light
/// over the middle; put it out once the server removes the fire.
#[allow(clippy::too_many_arguments)]
fn sync_fire_patches(
    fires: Query<(Entity, &MolotovFire)>,
    patches: Query<(Entity, &FirePatch)>,
    settings: Res<MolotovSettings>,
    assets: Res<MolotovAssets>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut commands: Commands,
) {
    let mut have = Vec::new();
    for (entity, patch) in &patches {
        if fires.contains(patch.src) {
            have.push(patch.src);
        } else {
            materials.remove(&patch.material);
            materials.remove(&patch.smoke);
            commands.entity(entity).try_despawn();
        }
    }
    for (src, fire) in &fires {
        if have.contains(&src) || fire.spots.is_empty() {
            continue;
        }
        let mid = fire.spots.iter().copied().sum::<Vec3>() / fire.spots.len() as f32;
        let material = materials.add(FireMaterial {
            params: settings.params(settings.fire_brightness, settings.pull, 0.0, Vec3::ZERO),
        });
        let smoke = materials.add(FireMaterial {
            params: settings.smoke_params(0.0),
        });
        let seed = (src.index() % 997) as f32;
        let hash = |i: u32, k: u32| {
            crate::util::rand01(src.index().wrapping_mul(7919) ^ i.wrapping_mul(31) ^ k.wrapping_mul(0x9E37_79B9))
        };
        let tiny = Vec3::new(0.001, 0.001, 0.0015);
        commands
            .spawn((
                StateScoped(AppState::InGame),
                FirePatch {
                    src,
                    material: material.clone(),
                    smoke: smoke.clone(),
                    age: 0.0,
                },
                Transform::from_translation(mid),
                Visibility::default(),
            ))
            .with_children(|patch| {
                let mut i = 0u32;
                for spot in &fire.spots {
                    for _ in 0..settings.flames_per_spot.max(1) {
                        i += 1;
                        let a = hash(i, 1) * std::f32::consts::TAU;
                        let r = hash(i, 2).sqrt() * settings.flame_jitter;
                        let at = *spot + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                        let k = 0.7 + 0.3 * hash(i, 3);
                        let size = Vec2::new(
                            settings.flame_width * k,
                            settings.flame_height * (0.65 + 0.35 * hash(i, 4)) * k,
                        );
                        patch.spawn((
                            PatchFlame {
                                size,
                                seed: hash(i, 5),
                            },
                            Mesh3d(assets.quad.clone()),
                            MeshMaterial3d(material.clone()),
                            Transform::from_translation(at - mid).with_scale(tiny),
                            NotShadowCaster,
                            // The quad is drawn well outside its own bounds
                            // (billboarded, base on the anchor).
                            NoFrustumCulling,
                        ));
                    }
                }
                // Smoke off evenly spread spots, starting a little above the
                // flames' bases.
                let columns = (settings.smoke_columns as usize).min(fire.spots.len());
                for c in 0..columns {
                    let spot = fire.spots[c * fire.spots.len() / columns.max(1)];
                    let k = 0.75 + 0.25 * hash(1000 + c as u32, 6);
                    patch.spawn((
                        PatchSmoke {
                            size: Vec2::new(settings.smoke_width, settings.smoke_height) * k,
                            seed: hash(1000 + c as u32, 7),
                        },
                        Mesh3d(assets.quad.clone()),
                        MeshMaterial3d(smoke.clone()),
                        Transform::from_translation(spot - mid + Vec3::Y * 0.4).with_scale(tiny),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                }
                patch.spawn((
                    PatchLight { seed },
                    PointLight {
                        color: settings.light_color(),
                        intensity: 0.0,
                        range: settings.fire_light_range,
                        shadows_enabled: settings.fire_light_shadows,
                        ..default()
                    },
                    Transform::from_xyz(0.0, settings.fire_light_height, 0.0),
                ));
            });
    }
}

/// Burn each fire patch: flare up, flicker, die down at the end — the flames
/// grow / shrink with the fade, the smoke thickens and thins, and the light
/// follows it. Holds still while the game's paused (the server's fire does
/// too).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_fire_patches(
    time: Res<Time>,
    paused: Res<GamePaused>,
    settings: Res<MolotovSettings>,
    killcam: Res<ActiveKillCam>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut patches: Query<(&mut FirePatch, &Children, &mut Visibility)>,
    mut flames: Query<(&PatchFlame, &mut Transform), (Without<PatchLight>, Without<PatchSmoke>)>,
    mut smokes: Query<(&PatchSmoke, &mut Transform), (Without<PatchLight>, Without<PatchFlame>)>,
    mut lights: Query<
        (&PatchLight, &mut PointLight, &mut Transform),
        (Without<PatchFlame>, Without<PatchSmoke>),
    >,
) {
    let dt = if paused.0 { 0.0 } else { time.delta_secs() };
    let t = time.elapsed_secs();
    let hidden = killcam.0.is_some();
    for (mut patch, children, mut vis) in &mut patches {
        patch.age += dt;
        let fade_in = (patch.age / FIRE_FADE_IN_SECS).clamp(0.0, 1.0);
        let left = shared::molotov::FIRE_SECS - patch.age;
        let fade_out = (left / FIRE_FADE_OUT_SECS).clamp(0.0, 1.0);
        let fade = fade_in * fade_in * (3.0 - 2.0 * fade_in) * fade_out;
        // Smoke builds up more slowly than the flames.
        let smoke_fade = (patch.age / 1.2).clamp(0.0, 1.0) * fade_out;
        vis.set_if_neq(if hidden { Visibility::Hidden } else { Visibility::Inherited });
        if let Some(m) = materials.get_mut(&patch.material) {
            m.params = settings.params(settings.fire_brightness, settings.pull, fade, Vec3::ZERO);
        }
        if let Some(m) = materials.get_mut(&patch.smoke) {
            m.params = settings.smoke_params(smoke_fade);
        }
        // Flames grow from a flicker to full and sink back down at the end.
        let grow = 0.35 + 0.65 * fade;
        for child in children.iter() {
            if let Ok((flame, mut tf)) = flames.get_mut(child) {
                tf.scale = seeded_scale(flame.size * grow, flame.seed);
            } else if let Ok((smoke, mut tf)) = smokes.get_mut(child) {
                tf.scale = seeded_scale(smoke.size * (0.6 + 0.4 * smoke_fade), smoke.seed);
            } else if let Ok((pl, mut light, mut tf)) = lights.get_mut(child) {
                light.intensity = settings.fire_light * fade * flicker(t, pl.seed, settings.flicker);
                light.range = settings.fire_light_range;
                light.color = settings.light_color();
                light.shadows_enabled = settings.fire_light_shadows;
                tf.translation.y = settings.fire_light_height;
            }
        }
    }
}

/// Keep one outlined, lying-down 1k model per [`MolotovDrop`].
fn sync_drop_avatars(
    drops: Query<(Entity, &MolotovDrop)>,
    mut avatars: Query<(Entity, &DropAvatar, &mut Transform, &mut Visibility)>,
    killcam: Res<ActiveKillCam>,
    settings: Res<MolotovSettings>,
    assets: Res<MolotovAssets>,
    mut commands: Commands,
) {
    let place = |d: &MolotovDrop| Transform {
        translation: d.pos + Vec3::Y * BOTTLE_RADIUS_UNITS * settings.world_scale,
        rotation: Quat::from_rotation_y(d.yaw) * Quat::from_rotation_z(FRAC_PI_2),
        scale: Vec3::splat(settings.world_scale),
    };
    let wanted = if killcam.0.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    let mut have = Vec::new();
    for (entity, avatar, mut tf, mut vis) in &mut avatars {
        match drops.get(avatar.src) {
            Ok((_, d)) => {
                tf.set_if_neq(place(d));
                vis.set_if_neq(wanted);
                have.push(avatar.src);
            }
            Err(_) => commands.entity(entity).try_despawn(),
        }
    }
    for (src, d) in &drops {
        if have.contains(&src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            DropAvatar { src },
            OutlineWhenLoaded,
            place(d),
            Visibility::default(),
            children![(
                SceneRoot(assets.world_model.clone()),
                Transform::default(),
            )],
        ));
    }
}

/// The server handed us a molotov we picked up: it's now the only lethal, and the
/// pickup sound plays — just for us.
fn receive_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::MolotovPickedUp>>,
    mut weapon: ResMut<Weapon>,
    mut pending: ResMut<crate::knife_pickup::PendingPickup>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.add_molotov();
            pending.0 = None;
            commands.spawn((
                AudioPlayer::new(sounds.pick_up_equipment.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}

/// The looping "rag lit" sound while the local player holds a molotov.
#[derive(Component)]
struct LightSound;

/// Play the light sound — for the holder only, not positional — from the
/// lethal key's press until the molotov's thrown or the throw's cancelled
/// (or a death / kill cam cuts it short).
fn play_light_sound(
    knife: Res<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    sounds: Res<GameSounds>,
    playing: Query<Entity, With<LightSound>>,
    mut commands: Commands,
) {
    let want = knife.molotov_lit() && killcam.0.is_none() && !death.is_active();
    match (want, playing.iter().next()) {
        (true, None) => {
            commands.spawn((
                StateScoped(AppState::InGame),
                LightSound,
                AudioPlayer::new(sounds.molotov_light.clone()),
                PlaybackSettings::LOOP,
            ));
        }
        (false, Some(_)) => {
            for e in &playing {
                commands.entity(e).try_despawn();
            }
        }
        _ => {}
    }
}

/// The server announced a molotov breaking somewhere: play the burst from
/// there, fading with distance like other players' sounds.
#[allow(clippy::too_many_arguments)]
fn receive_bursts(
    mut receivers: Query<&mut MessageReceiver<shared::MolotovBurst>>,
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    killcam: Res<ActiveKillCam>,
    sounds: Res<GameSounds>,
    sound_vol: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    mut commands: Commands,
) {
    let ear = listener.single().ok().map(|t| t.translation());
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(ear) = ear else { continue };
            if killcam.0.is_some() {
                continue;
            }
            let pos = Vec3::from_array(msg.point);
            let loudness = sound_vol.molotov_burst
                * crate::distance_falloff(ear.distance(pos), &remote)
                * remote.volume;
            if loudness <= 0.0 {
                continue;
            }
            commands.spawn((
                StateScoped(AppState::InGame),
                crate::RemoteSoundEmitter,
                AudioPlayer::new(sounds.molotov_burst.clone()),
                Transform::from_translation(pos),
                // (Bevy multiplies in `GlobalVolume` itself when the sound starts.)
                crate::positional_playback(bevy::audio::Volume::Linear(loudness)),
            ));
        }
    }
}

/// What the debug panel's "Molotov (Zombies)" section needs (bundled so it
/// takes one slot of `ads_tuning_ui`'s params).
#[derive(SystemParam)]
pub(crate) struct MolotovDebug<'w, 's> {
    settings: ResMut<'w, MolotovSettings>,
    pub(crate) weapon: ResMut<'w, Weapon>,
    /// (The monkey bomb's section shares this bundle — it needs `weapon` too.)
    pub(crate) monkey: ResMut<'w, crate::monkey_bomb::MonkeyBombSettings>,
    pub(crate) monkey_fuse_tx: Query<'w, 's, &'static mut TriggerSender<shared::SetMonkeyFuse>, With<GameClient>>,
    /// (And the frag's.)
    pub(crate) frag: ResMut<'w, crate::frag::FragSettings>,
    /// (And the flash bang's.)
    pub(crate) flash: ResMut<'w, crate::flash_bang::FlashBangSettings>,
    local_id: Query<'w, 's, &'static LocalId, With<GameClient>>,
    lobbies: Query<'w, 's, &'static shared::Lobby>,
    test_tx: Query<'w, 's, &'static mut TriggerSender<shared::SetMolotovTest>, With<GameClient>>,
}

/// Remember each zombie a Nuke kills, to set its body alight once it falls.
fn receive_nuked(mut receivers: Query<&mut MessageReceiver<ZombieNuked>>, mut pending: ResMut<PendingCorpseFires>) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            pending.0.push((msg.peer, 0.0));
        }
    }
}

/// Set each nuked zombie's body alight as soon as its avatar starts to fall:
/// flames scattered over where the body will lie — ahead of the feet for a
/// face-first fall, behind them for one onto its back — and a light.
#[allow(clippy::too_many_arguments)]
fn spawn_corpse_fires(
    time: Res<Time>,
    paused: Res<GamePaused>,
    settings: Res<MolotovSettings>,
    assets: Res<MolotovAssets>,
    mut pending: ResMut<PendingCorpseFires>,
    avatars: Query<(&RemoteAvatar, &crate::ZombieMotion)>,
    poses: Query<(&PlayerPose, &PlayerId)>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut commands: Commands,
) {
    if pending.0.is_empty() {
        return;
    }
    let dt = if paused.0 { 0.0 } else { time.delta_secs() };
    pending.0.retain_mut(|(peer, waited)| {
        *waited += dt;
        let found = avatars.iter().find_map(|(avatar, motion)| {
            let (pose, id) = poses.get(avatar.src).ok()?;
            (id.0 == *peer).then_some((pose, motion.fell_forward()))
        });
        let Some((pose, fell_forward)) = found else {
            // No avatar (yet, or any more).
            return *waited < CORPSE_FIRE_WAIT_SECS;
        };
        let Some(fell_forward) = fell_forward else {
            // Still standing — the death hasn't reached us yet.
            return *waited < CORPSE_FIRE_WAIT_SECS;
        };
        let feet = pose.translation - Vec3::Y * crate::EYE_HEIGHT;
        // `pose.yaw` faces -Z; the body lies along the way it faced (ahead
        // of the feet face-first, behind them on its back).
        let facing = Quat::from_rotation_y(pose.yaw) * Vec3::NEG_Z;
        let (along, offset) = if fell_forward {
            (facing, settings.corpse_forward_offset)
        } else {
            (-facing, settings.corpse_back_offset)
        };
        let across = Vec3::Y.cross(along).normalize_or_zero();
        let mid = feet + along * offset;
        let material = materials.add(FireMaterial {
            params: settings.params(settings.corpse_brightness, settings.pull, 0.0, Vec3::ZERO),
        });
        let key = peer.to_bits() as u32 ^ (peer.to_bits() >> 32) as u32;
        let hash = |i: u32, k: u32| crate::util::rand01(key.wrapping_mul(7919) ^ i.wrapping_mul(31) ^ k.wrapping_mul(0x9E37_79B9));
        let tiny = Vec3::new(0.001, 0.001, 0.0015);
        let flames = settings.corpse_flames.max(1);
        commands
            .spawn((
                StateScoped(AppState::InGame),
                CorpseFire {
                    material: material.clone(),
                    age: 0.0,
                },
                Transform::from_translation(mid),
                Visibility::default(),
            ))
            .with_children(|fire| {
                for i in 0..flames {
                    // Evenly down the body, jittered, and scattered across it.
                    let u = ((i as f32 + 0.25 + 0.5 * hash(i, 1)) / flames as f32 - 0.5) * settings.corpse_length;
                    let v = (hash(i, 2) - 0.5) * settings.corpse_width;
                    let k = 0.7 + 0.3 * hash(i, 3);
                    let size = Vec2::new(
                        settings.corpse_flame_width * k,
                        settings.corpse_flame_height * (0.65 + 0.35 * hash(i, 4)) * k,
                    );
                    fire.spawn((
                        CorpseFlame { size, seed: hash(i, 5) },
                        Mesh3d(assets.quad.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(along * u + across * v).with_scale(tiny),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                }
                fire.spawn((
                    CorpseLight { seed: hash(0, 6) * 10.0 },
                    PointLight {
                        color: settings.light_color(),
                        intensity: 0.0,
                        range: settings.corpse_light_range,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.6, 0.0),
                ));
            });
        false
    });
}

/// Burn each nuked body's fire: flare up as it falls, flicker, die down and
/// go. Holds still while the game's paused.
#[allow(clippy::type_complexity)]
fn update_corpse_fires(
    time: Res<Time>,
    paused: Res<GamePaused>,
    settings: Res<MolotovSettings>,
    killcam: Res<ActiveKillCam>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut fires: Query<(Entity, &mut CorpseFire, &Children, &mut Visibility)>,
    mut flames: Query<(&CorpseFlame, &mut Transform), Without<CorpseLight>>,
    mut lights: Query<(&CorpseLight, &mut PointLight), Without<CorpseFlame>>,
    mut commands: Commands,
) {
    let dt = if paused.0 { 0.0 } else { time.delta_secs() };
    let t = time.elapsed_secs();
    let hidden = killcam.0.is_some();
    for (entity, mut fire, children, mut vis) in &mut fires {
        fire.age += dt;
        let left = settings.corpse_fire_secs - fire.age;
        if left <= 0.0 {
            materials.remove(&fire.material);
            commands.entity(entity).try_despawn();
            continue;
        }
        let fade_in = (fire.age / settings.corpse_grow_secs.max(0.01)).clamp(0.0, 1.0);
        let fade_out = (left / settings.corpse_fade_secs.max(0.01)).clamp(0.0, 1.0);
        let fade = fade_in * fade_in * (3.0 - 2.0 * fade_in) * fade_out;
        vis.set_if_neq(if hidden { Visibility::Hidden } else { Visibility::Inherited });
        if let Some(m) = materials.get_mut(&fire.material) {
            m.params = settings.params(settings.corpse_brightness, settings.pull, fade, Vec3::ZERO);
        }
        let grow = 0.35 + 0.65 * fade;
        for child in children.iter() {
            if let Ok((flame, mut tf)) = flames.get_mut(child) {
                tf.scale = seeded_scale(flame.size * grow, flame.seed);
            } else if let Ok((cl, mut light)) = lights.get_mut(child) {
                light.intensity = settings.corpse_light * fade * flicker(t, cl.seed, settings.flicker);
                light.range = settings.corpse_light_range;
                light.color = settings.light_color();
            }
        }
    }
}

/// The debug panel's "Molotov (Zombies)" section. `hold_key` is
/// `ThrowArmsSettings::debug_hold_key` (the panel already holds that
/// resource).
pub(crate) fn molotov_section(ui: &mut egui::Ui, d: &mut MolotovDebug, hold_key: &mut bool) {
    // The drop test lives on the server's lobby, like the power-up one.
    let me = d.local_id.iter().next().map(|l| l.0);
    match me.and_then(|me| d.lobbies.iter().find(|l| l.has(me)).map(|l| (l.molotov_test, l.leader == me))) {
        Some((test, is_leader)) => {
            let mut on = test;
            if ui
                .add_enabled(is_leader, egui::Checkbox::new(&mut on, "Every zombie kill drops a molotov (test)"))
                .changed()
            {
                if let Ok(mut tx) = d.test_tx.single_mut() {
                    tx.trigger::<shared::LobbyChannel>(shared::SetMolotovTest { on });
                }
            }
            if !is_leader {
                ui.label("Only the party leader can change this.");
            }
        }
        None => {
            ui.label("Not in a lobby.");
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Give me a molotov").clicked() {
            d.weapon.add_molotov();
        }
        ui.label(format!("carrying {}", d.weapon.molotovs));
    });
    ui.checkbox(hold_key, "Hold lethal key (as if held — untick to throw)");

    let s = &mut *d.settings;
    ui.collapsing("Held molotov (2k model)", |ui| {
        ui.label(
            "Positioned in the throwing arms' local space, like the throwing knife model: \
             one unit = the arms' scale in metres (~cm by default).",
        );
        ui.add(egui::Slider::new(&mut s.held_translation.x, -100.0f32..=100.0).text("x"));
        ui.add(egui::Slider::new(&mut s.held_translation.y, -100.0f32..=100.0).text("y"));
        ui.add(egui::Slider::new(&mut s.held_translation.z, -100.0f32..=100.0).text("z"));
        ui.add(egui::Slider::new(&mut s.held_yaw, -180.0f32..=180.0).text("yaw (°)"));
        ui.add(egui::Slider::new(&mut s.held_pitch, -180.0f32..=180.0).text("pitch (°)"));
        ui.add(egui::Slider::new(&mut s.held_roll, -180.0f32..=180.0).text("roll (°)"));
        ui.add(egui::Slider::new(&mut s.held_scale, 0.1f32..=20.0).text("scale").logarithmic(true));
    });
    ui.collapsing("Burning rag (held + thrown)", |ui| {
        ui.label("Flame position in the molotov model's own units (bottle ~13 tall, neck up +y).");
        ui.add(egui::Slider::new(&mut s.rag_offset.x, -8.0f32..=8.0).text("x"));
        ui.add(egui::Slider::new(&mut s.rag_offset.y, 0.0f32..=20.0).text("y"));
        ui.add(egui::Slider::new(&mut s.rag_offset.z, -8.0f32..=8.0).text("z"));
        ui.add(egui::Slider::new(&mut s.rag_width, 0.01f32..=0.5).text("flame width (m)"));
        ui.add(egui::Slider::new(&mut s.rag_height, 0.01f32..=0.8).text("flame height (m)"));
        ui.add(egui::Slider::new(&mut s.rag_layers, 1u32..=RAG_LAYERS as u32).text("flame layers"));
        ui.add(egui::Slider::new(&mut s.rag_brightness, 0.0f32..=10.0).text("brightness"));
        ui.add(egui::Slider::new(&mut s.rag_light, 0.0f32..=1_000_000.0).text("light (lm)").logarithmic(true));
        ui.add(egui::Slider::new(&mut s.rag_light_range, 0.5f32..=30.0).text("light range (m)"));
        ui.add(egui::Slider::new(&mut s.trail, 0.0f32..=0.5).text("trail behind motion (lean per m/s)"));
        ui.add(egui::Slider::new(&mut s.max_lean, 0.0f32..=4.0).text("most lean"));
    });
    ui.collapsing("Thrown / dropped model (1k)", |ui| {
        ui.add(egui::Slider::new(&mut s.world_scale, 0.002f32..=0.1).text("scale").logarithmic(true));
    });
    ui.collapsing("Fire on the ground", |ui| {
        ui.label("Flame / smoke count, size and scatter apply to the next fire.");
        ui.add(egui::Slider::new(&mut s.flame_width, 0.1f32..=4.0).text("flame width (m)"));
        ui.add(egui::Slider::new(&mut s.flame_height, 0.1f32..=5.0).text("flame height (m)"));
        ui.add(egui::Slider::new(&mut s.flames_per_spot, 1u32..=6).text("flames per spot"));
        ui.add(egui::Slider::new(&mut s.flame_jitter, 0.0f32..=1.0).text("flame scatter (m)"));
        ui.add(egui::Slider::new(&mut s.fire_brightness, 0.0f32..=10.0).text("brightness"));
        ui.add(egui::Slider::new(&mut s.fire_light, 0.0f32..=10_000_000.0).text("light (lm)").logarithmic(true));
        ui.add(egui::Slider::new(&mut s.fire_light_range, 1.0f32..=60.0).text("light range (m)"));
        ui.add(egui::Slider::new(&mut s.fire_light_height, 0.0f32..=4.0).text("light height (m)"));
        ui.checkbox(&mut s.fire_light_shadows, "light casts shadows");
        ui.separator();
        ui.add(egui::Slider::new(&mut s.smoke_columns, 0u32..=10).text("smoke columns"));
        ui.add(egui::Slider::new(&mut s.smoke_width, 0.2f32..=8.0).text("smoke width (m)"));
        ui.add(egui::Slider::new(&mut s.smoke_height, 0.5f32..=20.0).text("smoke height (m)"));
        ui.add(egui::Slider::new(&mut s.smoke_opacity, 0.0f32..=1.0).text("smoke opacity"));
        ui.horizontal(|ui| {
            ui.label("smoke colour");
            ui.color_edit_button_rgb(&mut s.smoke_color);
        });
    });
    ui.collapsing("Fire on nuked zombies", |ui| {
        ui.label("Where the body lies from the feet, and the flames over it — applies to the next nuked zombie.");
        ui.add(egui::Slider::new(&mut s.corpse_forward_offset, -2.0f32..=3.0).text("fell forward: ahead of feet (m)"));
        ui.add(egui::Slider::new(&mut s.corpse_back_offset, -2.0f32..=3.0).text("fell back: behind feet (m)"));
        ui.add(egui::Slider::new(&mut s.corpse_length, 0.1f32..=3.0).text("body length (m)"));
        ui.add(egui::Slider::new(&mut s.corpse_width, 0.0f32..=1.5).text("body width (m)"));
        ui.add(egui::Slider::new(&mut s.corpse_flames, 1u32..=24).text("flames"));
        ui.add(egui::Slider::new(&mut s.corpse_flame_width, 0.1f32..=3.0).text("flame width (m)"));
        ui.add(egui::Slider::new(&mut s.corpse_flame_height, 0.1f32..=3.0).text("flame height (m)"));
        ui.add(egui::Slider::new(&mut s.corpse_fire_secs, 0.5f32..=10.0).text("burns for (s)"));
        ui.add(egui::Slider::new(&mut s.corpse_grow_secs, 0.0f32..=3.0).text("flares up over (s)"));
        ui.add(egui::Slider::new(&mut s.corpse_fade_secs, 0.1f32..=5.0).text("dies down over (s)"));
        ui.add(egui::Slider::new(&mut s.corpse_brightness, 0.0f32..=10.0).text("brightness"));
        ui.add(egui::Slider::new(&mut s.corpse_light, 0.0f32..=5_000_000.0).text("light (lm)").logarithmic(true));
        ui.add(egui::Slider::new(&mut s.corpse_light_range, 0.5f32..=30.0).text("light range (m)"));
    });
    ui.collapsing("Flame look (all flames)", |ui| {
        ui.horizontal(|ui| {
            ui.label("core");
            ui.color_edit_button_rgb(&mut s.core_color);
            ui.label("body");
            ui.color_edit_button_rgb(&mut s.mid_color);
            ui.label("tips");
            ui.color_edit_button_rgb(&mut s.tip_color);
        });
        ui.add(egui::Slider::new(&mut s.speed, 0.0f32..=4.0).text("speed"));
        ui.add(egui::Slider::new(&mut s.turbulence, 0.0f32..=3.0).text("turbulence"));
        ui.add(egui::Slider::new(&mut s.detail, 0.5f32..=10.0).text("detail"));
        ui.add(egui::Slider::new(&mut s.embers, 0.0f32..=10.0).text("embers"));
        ui.add(egui::Slider::new(&mut s.sink, 0.0f32..=0.5).text("base sinks below anchor"));
        ui.add(egui::Slider::new(&mut s.pull, 0.0f32..=1.0).text("pull toward camera (m)"));
        ui.horizontal(|ui| {
            ui.label("light colour");
            ui.color_edit_button_rgb(&mut s.light_color);
        });
        ui.add(egui::Slider::new(&mut s.flicker, 0.0f32..=1.0).text("light flicker"));
    });
    ui.horizontal(|ui| {
        if ui.button("Copy molotov settings to console").clicked() {
            info!(
                "molotov: held_translation: Vec3::new({:.3}, {:.3}, {:.3}), held_yaw: {:.1}, \
                 held_pitch: {:.1}, held_roll: {:.1}, held_scale: {:.3}, rag_offset: \
                 Vec3::new({:.3}, {:.3}, {:.3}), rag_width: {:.3}, rag_height: {:.3}, \
                 rag_layers: {}, rag_brightness: {:.2}, trail: {:.3}, max_lean: {:.2}, \
                 world_scale: {:.4}, flame_width: {:.2}, flame_height: {:.2}, \
                 flames_per_spot: {}, fire_brightness: {:.2}, smoke_columns: {}, \
                 smoke_width: {:.2}, smoke_height: {:.2}, smoke_opacity: {:.2}, \
                 core_color: {:?}, mid_color: {:?}, tip_color: {:?}, speed: {:.2}, \
                 turbulence: {:.2}, detail: {:.2}, embers: {:.2}",
                s.held_translation.x, s.held_translation.y, s.held_translation.z, s.held_yaw,
                s.held_pitch, s.held_roll, s.held_scale, s.rag_offset.x, s.rag_offset.y,
                s.rag_offset.z, s.rag_width, s.rag_height, s.rag_layers, s.rag_brightness,
                s.trail, s.max_lean, s.world_scale, s.flame_width, s.flame_height,
                s.flames_per_spot, s.fire_brightness, s.smoke_columns, s.smoke_width,
                s.smoke_height, s.smoke_opacity, s.core_color, s.mid_color, s.tip_color,
                s.speed, s.turbulence, s.detail, s.embers,
            );
        }
        if ui.button("Reset molotov settings").clicked() {
            *s = MolotovSettings::default();
        }
    });
}
