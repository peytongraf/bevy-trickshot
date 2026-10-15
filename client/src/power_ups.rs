//! `Zombies` power-ups, client side (the server owns them — see
//! `shared::power_ups` / `server::power_ups`):
//!
//! * every [`PowerUpDrop`] the server replicates gets an avatar: its model
//!   in shiny gold, floating over the spot, spinning and bobbing, wrapped in
//!   a green glow ([`PowerUpGlowMaterial`], `assets/shaders/power_up_glow.wgsl`)
//!   with a green light under it, and its loop sound playing from it
//!   (positional, faded by distance) after the spawn sound; it blinks once
//!   the server says it's about to go, slowly then quicker, ticking each
//!   blink, and ends with the timer end if it runs out;
//! * [`PowerUpGrabbed`]: everyone hears the announcer line (not positional),
//!   the one who grabbed it also hears the grab sound; Max Ammo plays its
//!   refill and fills our ammo, a Nuke goes off once the line's said — its
//!   flash sound, a blast where it lay and the screen whiting out
//!   ([`NukeFlashes`]) — a Carpenter hammers away while it's at work, at
//!   least [`CARPENTER_MIN_SECS`] ([`CarpenterHammering`]);
//! * Double Points and Insta-Kill tick down with their HUD icons for
//!   everyone, ending with the timer end ([`timed_countdowns`]);
//! * Insta-Kill / Double Points running (`Lobby::active_power_ups`): their
//!   icons with seconds left, in the order they started, above the perk
//!   icons.
//!
//! Everything's tunable from the debug panel ([`PowerUpSettings`]). Avatars
//! go when their drop does, and they, the HUD row and any sound still
//! playing are `StateScoped(InGame)` — nothing carries into the next game.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use lightyear::prelude::{LocalId, MessageReceiver};
use shared::power_ups::{PowerUp, BLINK_SECS, NUKE_FLASH_DELAY_SECS};
use shared::{GameMode, Lobby, PowerUpDrop, PowerUpGrabbed};

use crate::net::GameClient;
use crate::zombies_hud::{zombies_game, PERK_ICON_BOTTOM, PERK_ICON_SIZE};
use crate::{
    killcam, menu, AppState, GameSounds,
    Weapon, HUD_FONT,
};

const SHADER_ASSET_PATH: &str = "shaders/power_up_glow.wgsl";

use params::GlowParams;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct (as in `shroom_xray`).
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Mirrors `GlowParams` in the shader.
    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct GlowParams {
        pub(crate) color: Vec4,
        pub(crate) size: f32,
        pub(crate) pulse_speed: f32,
        pub(crate) pulse_amount: f32,
        pub(crate) swirl: f32,
        pub(crate) swirl_speed: f32,
        pub(crate) sparks: f32,
        pub(crate) pull: f32,
        pub(crate) core: f32,
        pub(crate) depth_pull: f32,
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct PowerUpGlowMaterial {
    #[uniform(0)]
    params: GlowParams,
}

impl Material for PowerUpGlowMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// A billboard: both faces, and it never writes depth (it's a glow, not
    /// a surface) — anything in front of the drop still hides it (the
    /// shader depth-tests it at the drop, not where `pull` puts it).
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

/// Each power-up's model, and its native centre (model units) — the avatar
/// spins about it.
fn model(kind: PowerUp) -> (&'static str, Vec3) {
    match kind {
        PowerUp::MaxAmmo => ("models/props/power_ups/max_ammo.glb", Vec3::new(0.001, 0.302, 0.0)),
        PowerUp::InstaKill => ("models/props/power_ups/insta_kill.glb", Vec3::new(0.0, 0.037, 0.0)),
        PowerUp::DoublePoints => ("models/props/power_ups/double_points.glb", Vec3::new(0.009, 0.056, 0.01)),
        PowerUp::Nuke => ("models/props/power_ups/nuke.glb", Vec3::new(1.0, 0.0, 0.0)),
        PowerUp::BonusPoints => ("models/props/power_ups/bonus_points.glb", Vec3::new(-0.005, 0.0, 0.0)),
        PowerUp::Carpenter => ("models/props/power_ups/carpenter.glb", Vec3::new(0.0, 0.13, -0.005)),
    }
}

/// The HUD icon of a timed power-up.
fn icon_path(kind: PowerUp) -> Option<&'static str> {
    match kind {
        PowerUp::InstaKill => Some("textures/icons/power_ups/insta_kill.png"),
        PowerUp::DoublePoints => Some("textures/icons/power_ups/double_points.png"),
        _ => None,
    }
}

/// Everything tunable about how a drop looks (the debug panel's
/// "Power-ups (Zombies)" section).
#[derive(Resource, Clone, PartialEq)]
pub(crate) struct PowerUpSettings {
    /// How high (m) the model's middle floats over the ground.
    pub(crate) height: f32,
    /// How far (m) it bobs up and down either way, and how fast (cycles/s).
    pub(crate) bob_height: f32,
    pub(crate) bob_speed: f32,
    /// Turn rate (degrees/s).
    pub(crate) spin_deg: f32,
    /// Each model's scale — they're modelled at wildly different sizes, so
    /// these bring each to about 0.7 m across.
    pub(crate) scale: [f32; PowerUp::ALL.len()],
    /// The gold: sRGB colour, metalness, roughness and how much it glows by
    /// itself (so it still reads as gold on a dark map).
    pub(crate) gold_color: [f32; 3],
    pub(crate) gold_metallic: f32,
    pub(crate) gold_roughness: f32,
    pub(crate) gold_emissive: f32,
    /// The glow: sRGB colour and brightness, width (m), pulse, swirl, sparks,
    /// how far it's pulled toward the camera (m) and its core's size.
    pub(crate) glow_color: [f32; 3],
    pub(crate) glow_brightness: f32,
    pub(crate) glow_size: f32,
    pub(crate) glow_pulse_speed: f32,
    pub(crate) glow_pulse_amount: f32,
    pub(crate) glow_swirl: f32,
    pub(crate) glow_swirl_speed: f32,
    pub(crate) glow_sparks: f32,
    pub(crate) glow_pull: f32,
    pub(crate) glow_core: f32,
    /// Where the glow is depth-tested, on top of the automatic spot just
    /// behind the whole model (m toward the camera; 0 = that spot, so all of
    /// the model covers the glow — positive starts letting it over the
    /// model's back).
    pub(crate) glow_depth_pull: f32,
    /// The green light under it: lumens and range (m).
    pub(crate) light_lumens: f32,
    pub(crate) light_range: f32,
    /// Blinks per second once it's about to go.
    pub(crate) blink_hz: f32,
}

impl Default for PowerUpSettings {
    fn default() -> Self {
        Self {
            height: 1.3,
            bob_height: 0.13,
            bob_speed: 0.5,
            spin_deg: 60.0,
            // Max Ammo, Insta-Kill, Double Points, Nuke, Bonus Points,
            // Carpenter (`PowerUp::ALL`'s order).
            scale: [1.15, 0.35, 4.0, 0.024, 1.1, 2.6],
            gold_color: [0.3633843, 0.273065, 0.09241041],
            gold_metallic: 0.87,
            gold_roughness: 0.0,
            gold_emissive: 0.0,
            glow_color: [0.011099039, 0.26902723, 0.028294351],
            glow_brightness: 1.0,
            glow_size: 2.2,
            glow_pulse_speed: 4.3,
            glow_pulse_amount: 0.65,
            glow_swirl: 0.8,
            glow_swirl_speed: 2.15,
            glow_sparks: 0.0,
            glow_pull: 0.0,
            glow_core: 0.26,
            glow_depth_pull: -0.3,
            light_lumens: 10_000.0,
            light_range: 4.7,
            blink_hz: 4.0,
        }
    }
}

impl PowerUpSettings {
    pub(crate) fn scale_mut(&mut self, kind: PowerUp) -> &mut f32 {
        let i = PowerUp::ALL.iter().position(|p| *p == kind).unwrap_or(0);
        &mut self.scale[i]
    }

    fn scale(&self, kind: PowerUp) -> f32 {
        let i = PowerUp::ALL.iter().position(|p| *p == kind).unwrap_or(0);
        self.scale[i].max(1e-4)
    }

    fn glow(&self) -> GlowParams {
        let [r, g, b] = self.glow_color;
        let lin = Color::srgb(r, g, b).to_linear();
        GlowParams {
            color: Vec4::new(lin.red, lin.green, lin.blue, self.glow_brightness),
            size: self.glow_size,
            pulse_speed: self.glow_pulse_speed,
            pulse_amount: self.glow_pulse_amount,
            swirl: self.glow_swirl,
            swirl_speed: self.glow_swirl_speed,
            sparks: self.glow_sparks,
            pull: self.glow_pull,
            core: self.glow_core,
            depth_pull: self.glow_depth_pull,
        }
    }

    fn gold(&self, m: &mut StandardMaterial) {
        let [r, g, b] = self.gold_color;
        let c = Color::srgb(r, g, b);
        m.base_color = c;
        m.base_color_texture = None;
        m.metallic = self.gold_metallic;
        m.perceptual_roughness = self.gold_roughness;
        m.emissive = c.to_linear() * self.gold_emissive;
    }

    fn light_color(&self) -> Color {
        let [r, g, b] = self.glow_color;
        Color::srgb(r, g, b)
    }
}

/// The shared assets every avatar uses (built on first use).
#[derive(Resource)]
struct PowerUpAssets {
    gold: Handle<StandardMaterial>,
    glow: Handle<PowerUpGlowMaterial>,
    quad: Handle<Mesh>,
}

/// Stands in for one replicated [`PowerUpDrop`].
#[derive(Component)]
struct DropAvatar {
    src: Entity,
    /// The drop's own phase, so several don't bob in step.
    phase: f32,
    /// Seconds it's been blinking (not counting pauses) — near
    /// [`BLINK_SECS`] when it goes, it timed out rather than being grabbed.
    blink_age: f32,
    /// Blinks it's been through (the fraction is the one under way: shown
    /// for the first half, hidden for the second) — see [`blink_hz`].
    blink_phase: f32,
    /// Shown last frame (to tick as it blinks back on).
    lit: bool,
}

/// Blinks a second `blink_age` seconds into a drop's blinking: slow for the
/// first half of [`BLINK_SECS`], then at the full `hz`, Call of Duty style.
fn blink_hz(hz: f32, blink_age: f32) -> f32 {
    let hz = hz.max(0.1);
    if blink_age < BLINK_SECS * 0.5 {
        hz * 0.5
    } else {
        hz
    }
}

/// The timed power-ups' HUD icons flicker off on this, in their last few
/// seconds — Double Points' countdown ticks with it.
fn hud_flicker_off(t: f32) -> bool {
    (t * 6.0).floor() as i64 % 2 == 1
}

/// Double Points' countdown ticks once a second from this many seconds left
/// (as its number counts down), then with its icon's flicker over the last
/// [`HUD_FLICKER_SECS`].
const DOUBLE_POINTS_TICK_SECS: u16 = 10;
const HUD_FLICKER_SECS: u16 = 5;

/// A Carpenter's hammering goes on at least this long (s), however quick
/// the boards are.
const CARPENTER_MIN_SECS: f32 = 3.0;

/// The hammering, while a Carpenter's at work: when it (last) started.
#[derive(Component)]
struct CarpenterHammering {
    started: f32,
}

/// Seconds (not counting pauses) till each grabbed Nuke's flash
/// ([`NUKE_FLASH_DELAY_SECS`]), and where its drop was.
#[derive(Resource, Default)]
struct NukeFlashes(Vec<(f32, Vec3)>);

/// A Nuke's white-out over the whole screen, Call of Duty style: up fast to
/// [`NUKE_WHITE_PEAK`], held a moment, then faded away.
#[derive(Component)]
struct NukeWhiteout {
    age: f32,
}

const NUKE_WHITE_PEAK: f32 = 0.9;
const NUKE_WHITE_IN_SECS: f32 = 0.08;
const NUKE_WHITE_HOLD_SECS: f32 = 0.2;
const NUKE_WHITE_OUT_SECS: f32 = 1.2;

/// The spinning, bobbing part of an avatar (the model under it).
#[derive(Component)]
struct DropSpinner;

/// A model scene; gilded once it's spawned.
#[derive(Component)]
struct DropModel(PowerUp);

#[derive(Component)]
struct DropLight;

/// A drop's glow quad. Its transform's scale carries the model's bounding
/// radius (m, measured by [`measure_drop_models`]) to the shader, which
/// depth-tests the glow that far behind the model's middle — so every part
/// of the model, however it's turned, draws over the glow. (The shader sizes
/// the quad itself from `glow_size`, never from this scale.)
#[derive(Component)]
struct DropGlow;

/// [`DropGlow`]'s radius until the model's loaded and measured.
const FALLBACK_MODEL_RADIUS: f32 = 0.5;

/// The loop playing from a drop.
#[derive(Component)]
struct DropLoop;

/// One slot in the timed power-ups row: the `index`th running one.
#[derive(Component)]
struct TimedSlot {
    index: usize,
    shown: Option<PowerUp>,
}

#[derive(Component)]
struct TimedSlotIcon(usize);

#[derive(Component)]
struct TimedSlotText(usize);

const TIMED_ICON_SIZE: f32 = 64.0;

pub(crate) struct PowerUpsPlugin;

impl Plugin for PowerUpsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PowerUpGlowMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .init_resource::<PowerUpSettings>()
        .init_resource::<NukeFlashes>()
        .add_systems(OnEnter(AppState::InGame), spawn_timed_row)
        .add_systems(OnExit(AppState::InGame), |mut n: ResMut<NukeFlashes>| n.0.clear())
        .add_systems(
            Update,
            (
                sync_drop_avatars,
                animate_drop_avatars,
                measure_drop_models,
                apply_power_up_settings,
                receive_grabs,
                nuke_flashes,
                fade_nuke_whiteouts,
                carpenter_hammering,
                timed_countdowns,
                update_timed_row,
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

fn assets(
    commands: &mut Commands,
    have: &Option<Res<PowerUpAssets>>,
    settings: &PowerUpSettings,
    std_materials: &mut Assets<StandardMaterial>,
    glow_materials: &mut Assets<PowerUpGlowMaterial>,
    meshes: &mut Assets<Mesh>,
) -> (Handle<StandardMaterial>, Handle<PowerUpGlowMaterial>, Handle<Mesh>) {
    if let Some(a) = have {
        return (a.gold.clone(), a.glow.clone(), a.quad.clone());
    }
    let mut gold = StandardMaterial::default();
    settings.gold(&mut gold);
    let a = PowerUpAssets {
        gold: std_materials.add(gold),
        glow: glow_materials.add(PowerUpGlowMaterial {
            params: settings.glow(),
        }),
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
    };
    let out = (a.gold.clone(), a.glow.clone(), a.quad.clone());
    commands.insert_resource(a);
    out
}

/// An avatar for every drop that hasn't one; drop the avatars whose drop is
/// gone (grabbed, timed out, or the game ended).
#[allow(clippy::too_many_arguments)]
fn sync_drop_avatars(
    drops: Query<(Entity, &PowerUpDrop)>,
    avatars: Query<(Entity, &DropAvatar, &Transform)>,
    settings: Res<PowerUpSettings>,
    have: Option<Res<PowerUpAssets>>,
    sounds: Res<GameSounds>,
    asset_server: Res<AssetServer>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut glow_materials: ResMut<Assets<PowerUpGlowMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    for (e, avatar, tf) in &avatars {
        if drops.get(avatar.src).is_err() {
            commands.entity(e).despawn();
            // Gone at the end of its blinking: it timed out.
            if avatar.blink_age >= BLINK_SECS - 0.4 {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    AudioPlayer::new(sounds.power_up_timer_end.clone()),
                    Transform::from_translation(tf.translation + Vec3::Y * settings.height),
                    crate::positional_playback(Volume::Linear(1.0)),
                ));
            }
        }
    }
    for (src, drop) in &drops {
        if avatars.iter().any(|(_, a, _)| a.src == src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.power_up_spawn.clone()),
            Transform::from_translation(drop.pos + Vec3::Y * settings.height),
            crate::positional_playback(Volume::Linear(1.0)),
        ));
        let (_, glow, quad) = assets(
            &mut commands,
            &have,
            &settings,
            &mut std_materials,
            &mut glow_materials,
            &mut meshes,
        );
        let (path, centre) = model(drop.kind);
        let scale = settings.scale(drop.kind);
        commands
            .spawn((
                StateScoped(AppState::InGame),
                DropAvatar {
                    src,
                    phase: (src.index() as f32 * 1.618).fract() * std::f32::consts::TAU,
                    blink_age: 0.0,
                    blink_phase: 0.0,
                    lit: true,
                },
                Transform::from_translation(drop.pos),
                Visibility::default(),
            ))
            .with_children(|avatar| {
                avatar
                    .spawn((DropSpinner, Transform::default(), Visibility::default()))
                    .with_children(|spinner| {
                        spinner
                            .spawn((
                                DropModel(drop.kind),
                                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))),
                                Transform::from_translation(-centre * scale)
                                    .with_scale(Vec3::splat(scale)),
                            ))
                            .observe(gild_model);
                        spinner.spawn((
                            DropGlow,
                            Mesh3d(quad),
                            MeshMaterial3d(glow),
                            Transform::from_scale(Vec3::splat(FALLBACK_MODEL_RADIUS)),
                            NoFrustumCulling,
                            NotShadowCaster,
                        ));
                        spinner.spawn((
                            DropLight,
                            PointLight {
                                color: settings.light_color(),
                                intensity: settings.light_lumens,
                                range: settings.light_range,
                                shadows_enabled: false,
                                ..default()
                            },
                            Transform::default(),
                        ));
                    });
                // (Faded by distance by its profile in the mix.)
                avatar.spawn((
                    DropLoop,
                    AudioPlayer::new(sounds.power_up_loop.clone()),
                    PlaybackSettings::LOOP.with_spatial(true),
                    Transform::from_xyz(0.0, 1.0, 0.0),
                ));
            });
    }
}

/// Keep each glow's [`DropGlow`] radius at its model's bounding radius about
/// the spin centre — measured off the model's meshes as they are (so a scale
/// change in the panel follows; the spin doesn't change it).
fn measure_drop_models(
    spinners: Query<(&GlobalTransform, &Children), With<DropSpinner>>,
    models: Query<(), With<DropModel>>,
    children: Query<&Children>,
    meshes: Query<(&bevy::render::primitives::Aabb, &GlobalTransform)>,
    mut glows: Query<&mut Transform, With<DropGlow>>,
) {
    for (spinner_gt, kids) in &spinners {
        let centre = spinner_gt.translation();
        let mut radius: Option<f32> = None;
        for model in kids.iter().filter(|&k| models.contains(k)) {
            for e in children.iter_descendants(model) {
                let Ok((aabb, gt)) = meshes.get(e) else { continue };
                let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                for i in 0..8 {
                    let corner = c + h * Vec3::new(
                        if i & 1 == 0 { -1.0 } else { 1.0 },
                        if i & 2 == 0 { -1.0 } else { 1.0 },
                        if i & 4 == 0 { -1.0 } else { 1.0 },
                    );
                    let d = gt.transform_point(corner).distance(centre);
                    radius = Some(radius.map_or(d, |r: f32| r.max(d)));
                }
            }
        }
        let Some(radius) = radius else { continue };
        for glow in kids.iter() {
            if let Ok(mut tf) = glows.get_mut(glow) {
                let want = Vec3::splat(radius.max(0.01));
                if tf.scale != want {
                    tf.scale = want;
                }
            }
        }
    }
}

/// Swap every material in a freshly spawned model for the shared gold.
fn gild_model(
    trigger: Trigger<SceneInstanceReady>,
    assets: Option<Res<PowerUpAssets>>,
    children: Query<&Children>,
    mut mats: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    for e in children.iter_descendants(trigger.target()) {
        if let Ok(mut m) = mats.get_mut(e) {
            m.0 = assets.gold.clone();
            commands.entity(e).insert(NotShadowCaster);
        }
    }
}

/// Float, spin, bob — and blink once the server says it's about to go,
/// slowly then quicker ([`blink_hz`]), ticking from it each time it comes
/// back on.
#[allow(clippy::type_complexity)]
fn animate_drop_avatars(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    settings: Res<PowerUpSettings>,
    sounds: Res<GameSounds>,
    drops: Query<&PowerUpDrop>,
    mut avatars: Query<(&mut DropAvatar, &Transform, &Children, &mut Visibility), Without<DropSpinner>>,
    mut spinners: Query<&mut Transform, With<DropSpinner>>,
    mut commands: Commands,
) {
    let t = time.elapsed_secs();
    let dt = if paused.0 { 0.0 } else { time.delta_secs() };
    for (mut avatar, tf, children, mut vis) in &mut avatars {
        let blinking = drops.get(avatar.src).is_ok_and(|d| d.blinking);
        if blinking {
            avatar.blink_age += dt;
            avatar.blink_phase += dt * blink_hz(settings.blink_hz, avatar.blink_age);
        }
        let on = !blinking || avatar.blink_phase.fract() < 0.5;
        if blinking && on && !avatar.lit {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.power_up_timer_tick.clone()),
                Transform::from_translation(tf.translation + Vec3::Y * settings.height),
                crate::positional_playback(Volume::Linear(1.0)),
            ));
        }
        avatar.lit = on;
        vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
        for child in children.iter() {
            if let Ok(mut tf) = spinners.get_mut(child) {
                let bob = (t * settings.bob_speed * std::f32::consts::TAU + avatar.phase).sin()
                    * settings.bob_height;
                tf.translation = Vec3::Y * (settings.height + bob);
                tf.rotation = Quat::from_rotation_y(t * settings.spin_deg.to_radians() + avatar.phase);
            }
        }
    }
}

/// Push panel changes onto the shared gold / glow and every drop's light and
/// model scale.
#[allow(clippy::too_many_arguments)]
fn apply_power_up_settings(
    settings: Res<PowerUpSettings>,
    assets: Option<Res<PowerUpAssets>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut glow_materials: ResMut<Assets<PowerUpGlowMaterial>>,
    mut lights: Query<&mut PointLight, With<DropLight>>,
    mut models: Query<(&DropModel, &mut Transform)>,
) {
    if !settings.is_changed() {
        return;
    }
    if let Some(a) = assets {
        if let Some(m) = std_materials.get_mut(&a.gold) {
            settings.gold(m);
        }
        let want = settings.glow();
        if glow_materials.get(&a.glow).is_some_and(|m| m.params != want) {
            if let Some(m) = glow_materials.get_mut(&a.glow) {
                m.params = want;
            }
        }
    }
    for mut light in &mut lights {
        light.color = settings.light_color();
        light.intensity = settings.light_lumens;
        light.range = settings.light_range;
    }
    for (m, mut tf) in &mut models {
        let scale = settings.scale(m.0);
        tf.translation = -model(m.0).1 * scale;
        tf.scale = Vec3::splat(scale);
    }
}

/// Someone grabbed a power-up: its announcer for everyone, the grab sound
/// for whoever it was, and Max Ammo fills our ammo — both guns, mags and
/// all.
fn receive_grabs(
    mut receivers: Query<&mut MessageReceiver<PowerUpGrabbed>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    sounds: Res<GameSounds>,
    time: Res<Time>,
    mut weapon: ResMut<Weapon>,
    mut nukes: ResMut<NukeFlashes>,
    mut hammering: Query<&mut CarpenterHammering>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let announcer = match msg.kind {
                PowerUp::MaxAmmo => &sounds.power_up_max_ammo,
                PowerUp::InstaKill => &sounds.power_up_insta_kill,
                PowerUp::DoublePoints => &sounds.power_up_double_points,
                PowerUp::Nuke => &sounds.power_up_nuke,
                PowerUp::BonusPoints => &sounds.power_up_bonus_points,
                PowerUp::Carpenter => &sounds.power_up_carpenter,
            };
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(announcer.clone()),
                PlaybackSettings::DESPAWN,
            ));
            if Some(msg.by) == me {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    AudioPlayer::new(sounds.power_up_grab.clone()),
                    PlaybackSettings::DESPAWN,
                ));
            }
            match msg.kind {
                PowerUp::MaxAmmo => {
                    commands.spawn((
                        StateScoped(AppState::InGame),
                        AudioPlayer::new(sounds.max_ammo_refill.clone()),
                        PlaybackSettings::DESPAWN,
                    ));
                }
                PowerUp::Nuke => nukes.0.push((NUKE_FLASH_DELAY_SECS, msg.pos)),
                PowerUp::Carpenter => {
                    // (Another while one's at work: the hammering goes on.)
                    let now = time.elapsed_secs();
                    if let Ok(mut h) = hammering.single_mut() {
                        h.started = now;
                    } else {
                        commands.spawn((
                            StateScoped(AppState::InGame),
                            CarpenterHammering { started: now },
                            AudioPlayer::new(sounds.carpenter_loop.clone()),
                            PlaybackSettings::LOOP,
                        ));
                    }
                }
                _ => {}
            }
            if msg.kind == PowerUp::MaxAmmo {
                weapon.max_ammo(crate::pap::my_pap_levels(&local, &lobbies));
                // Only the lethal actually carried fills up.
                match weapon.lethal {
                    crate::Lethal::ThrowingKnife => {
                        weapon.throwing_knives = weapon
                            .throwing_knives
                            .max(shared::throwing_knife::starting_knives(GameMode::Zombies));
                    }
                    crate::Lethal::Molotov => weapon.molotovs = shared::molotov::MAX_MOLOTOVS,
                    crate::Lethal::MonkeyBomb => weapon.monkey_bombs = shared::monkey_bomb::MAX_MONKEYS,
                    crate::Lethal::Frag => weapon.frags = shared::frag::MAX_FRAGS,
                    crate::Lethal::FlashBang => {}
                }
                // ...and the tactical, if one's carried.
                if let Some(tactical) = weapon.tactical {
                    weapon.fill_lethal(tactical);
                }
            }
        }
    }
}

/// Each grabbed Nuke going off, once the announcer's said it — when its
/// first zombie catches fire (`server::power_ups::fire_nukes`): its flash
/// sound, a big blast where the drop was (`vfx::Explosion`, just a look)
/// and the screen whiting out.
fn nuke_flashes(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    sounds: Res<GameSounds>,
    mut nukes: ResMut<NukeFlashes>,
    mut boom: EventWriter<crate::vfx::Explosion>,
    mut commands: Commands,
) {
    if nukes.0.is_empty() || paused.0 {
        return;
    }
    let dt = time.delta_secs();
    nukes.0.retain_mut(|(left, at)| {
        *left -= dt;
        if *left > 0.0 {
            return true;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.nuke_flash.clone()),
            PlaybackSettings::DESPAWN,
        ));
        boom.write(crate::vfx::Explosion {
            feet: *at,
            variant: 0,
            phd: false,
            dog: false,
            boss: false,
            frag: false,
            nuke: true,
        });
        commands.spawn((
            StateScoped(AppState::InGame),
            NukeWhiteout { age: 0.0 },
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::WHITE.with_alpha(0.0)),
            GlobalZIndex(40),
            Pickable::IGNORE,
        ));
        false
    });
}

/// Fade each Nuke white-out up, hold it and fade it away.
fn fade_nuke_whiteouts(
    time: Res<Time>,
    mut whiteouts: Query<(Entity, &mut NukeWhiteout, &mut BackgroundColor)>,
    mut commands: Commands,
) {
    for (e, mut w, mut bg) in &mut whiteouts {
        w.age += time.delta_secs();
        let a = w.age;
        let alpha = if a < NUKE_WHITE_IN_SECS {
            a / NUKE_WHITE_IN_SECS
        } else if a < NUKE_WHITE_IN_SECS + NUKE_WHITE_HOLD_SECS {
            1.0
        } else {
            1.0 - (a - NUKE_WHITE_IN_SECS - NUKE_WHITE_HOLD_SECS) / NUKE_WHITE_OUT_SECS
        };
        if alpha <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        bg.0 = Color::WHITE.with_alpha(alpha * NUKE_WHITE_PEAK);
    }
}

/// Stop the hammering once the Carpenter's done (`Lobby::carpenter`) — but
/// not before [`CARPENTER_MIN_SECS`] — with its finish. Out of the game:
/// just stop.
fn carpenter_hammering(
    time: Res<Time>,
    sounds: Res<GameSounds>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    hammering: Query<(Entity, &CarpenterHammering)>,
    mut commands: Commands,
) {
    let Ok((e, h)) = hammering.single() else { return };
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        commands.entity(e).despawn();
        return;
    };
    if lobby.carpenter || time.elapsed_secs() - h.started < CARPENTER_MIN_SECS {
        return;
    }
    commands.entity(e).despawn();
    commands.spawn((
        StateScoped(AppState::InGame),
        AudioPlayer::new(sounds.carpenter_end.clone()),
        PlaybackSettings::DESPAWN,
    ));
}

/// Each timed power-up (Double Points, Insta-Kill) running out, for
/// everyone, in step with its HUD icon: a tick each second its number counts
/// down from [`DOUBLE_POINTS_TICK_SECS`], then one each time the icon
/// flickers back on over the last [`HUD_FLICKER_SECS`] — and the end as it
/// runs out (not when it's grabbed again, nor when the game's over). Both
/// use Double Points' countdown sounds.
#[allow(clippy::too_many_arguments)]
fn timed_countdowns(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    sounds: Res<GameSounds>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut last: Local<Vec<(PowerUp, u16)>>,
    mut was_off: Local<bool>,
    mut commands: Commands,
) {
    let lobby = zombies_game(&local, &lobbies);
    let running: Vec<(PowerUp, u16)> = lobby.map_or_else(Vec::new, |l| l.active_power_ups.clone());
    let off = hud_flicker_off(time.elapsed_secs());
    let flicked_on = *was_off && !off;
    *was_off = off;
    let before = std::mem::replace(&mut *last, running.clone());
    let play = |clip: &Handle<AudioSource>, commands: &mut Commands| {
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(clip.clone()),
            PlaybackSettings::DESPAWN,
        ));
    };
    for kind in PowerUp::ALL.into_iter().filter(|p| p.timed()) {
        let find = |list: &[(PowerUp, u16)]| list.iter().find(|(p, _)| *p == kind).map(|&(_, s)| s);
        match (find(&before), find(&running)) {
            // Ran out (the server drops it after its last second).
            (Some(b), None) if b <= 1 && lobby.is_some() => play(&sounds.double_points_end, &mut commands),
            (b, Some(s)) if !paused.0 => {
                let counted_down = b.is_some_and(|b| s < b);
                if s <= HUD_FLICKER_SECS {
                    if flicked_on {
                        play(&sounds.double_points_tick, &mut commands);
                    }
                } else if s <= DOUBLE_POINTS_TICK_SECS && counted_down {
                    play(&sounds.double_points_tick, &mut commands);
                }
            }
            _ => {}
        }
    }
}

// --- the timed power-ups row -----------------------------------------------

fn spawn_timed_row(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                // Just above the perk icons.
                bottom: Val::Px(PERK_ICON_BOTTOM + PERK_ICON_SIZE + 12.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(18.0),
                ..default()
            },
        ))
        .with_children(|row| {
            // One slot per timed power-up there is; unused ones take no room.
            for index in 0..2 {
                row.spawn((
                    TimedSlot { index, shown: None },
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        display: Display::None,
                        ..default()
                    },
                ))
                .with_children(|slot| {
                    slot.spawn((
                        TimedSlotIcon(index),
                        ImageNode::default(),
                        Node {
                            width: Val::Px(TIMED_ICON_SIZE),
                            height: Val::Px(TIMED_ICON_SIZE),
                            ..default()
                        },
                    ));
                    slot.spawn((
                        TimedSlotText(index),
                        Text::new(""),
                        TextFont {
                            font: font.clone(),
                            font_size: 26.0,
                            ..default()
                        },
                        TextColor::WHITE,
                        TextShadow::default(),
                    ));
                });
            }
        });
}

/// Show the running timed power-ups, first-started leftmost, with seconds
/// left (the icon flickers for the last few).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_timed_row(
    time: Res<Time>,
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut slots: Query<(&mut TimedSlot, &mut Node)>,
    mut icons: Query<(&TimedSlotIcon, &mut ImageNode)>,
    mut texts: Query<(&TimedSlotText, &mut Text)>,
) {
    let running: &[(PowerUp, u16)] = zombies_game(&local, &lobbies)
        .map_or(&[], |l| l.active_power_ups.as_slice());
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    let flicker_off = hud_flicker_off(time.elapsed_secs());
    for (mut slot, mut node) in &mut slots {
        let entry = running
            .get(slot.index)
            .copied()
            .filter(|(p, _)| hud_up && icon_path(*p).is_some());
        let display = if entry.is_some() { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        let Some((kind, secs)) = entry else {
            slot.shown = None;
            continue;
        };
        let new_kind = slot.shown != Some(kind);
        slot.shown = Some(kind);
        for (icon, mut image) in &mut icons {
            if icon.0 != slot.index {
                continue;
            }
            if new_kind {
                if let Some(path) = icon_path(kind) {
                    image.image = asset_server.load(path);
                }
            }
            let alpha = if secs <= HUD_FLICKER_SECS && flicker_off { 0.35 } else { 1.0 };
            if image.color.alpha() != alpha {
                image.color = Color::WHITE.with_alpha(alpha);
            }
        }
        for (text, mut t) in &mut texts {
            if text.0 == slot.index {
                let want = secs.to_string();
                if t.0 != want {
                    t.0 = want;
                }
            }
        }
    }
}
