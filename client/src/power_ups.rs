//! `Zombies` power-ups, client side (the server owns them — see
//! `shared::power_ups` / `server::power_ups`):
//!
//! * every [`PowerUpDrop`] the server replicates gets an avatar: its model
//!   in shiny gold, floating over the spot, spinning and bobbing, wrapped in
//!   a green glow ([`PowerUpGlowMaterial`], `assets/shaders/power_up_glow.wgsl`)
//!   with a green light under it, and its loop sound playing from it
//!   (positional, faded by distance); it blinks once the server says it's
//!   about to go;
//! * [`PowerUpGrabbed`]: everyone hears the announcer line (not positional),
//!   the one who grabbed it also hears the grab sound, and Max Ammo fills
//!   our ammo;
//! * Insta-Kill / Double Points running (`Lobby::active_power_ups`): their
//!   icons with seconds left, in the order they started, above the perk
//!   icons.
//!
//! Everything's tunable from the debug panel ([`PowerUpSettings`]). Avatars
//! go when their drop does, and they, the HUD row and any sound still
//! playing are `StateScoped(InGame)` — nothing carries into the next game.

use bevy::audio::Volume;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use lightyear::prelude::{LocalId, MessageReceiver};
use shared::power_ups::PowerUp;
use shared::{GameMode, Lobby, PowerUpDrop, PowerUpGrabbed};

use crate::net::GameClient;
use crate::zombies_hud::{zombies_game, PERK_ICON_BOTTOM, PERK_ICON_SIZE};
use crate::{
    distance_falloff, killcam, menu, AppState, GameSounds, RemoteSoundEmitter,
    RemoteSoundSettings, SoundVolumes, Weapon, WorldModelCamera, HUD_FONT,
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

/// Each power-up's model, and its native centre (model units) — the avatar
/// spins about it.
fn model(kind: PowerUp) -> (&'static str, Vec3) {
    match kind {
        PowerUp::MaxAmmo => ("models/power_ups/max_ammo.glb", Vec3::new(0.001, 0.302, 0.0)),
        PowerUp::InstaKill => ("models/power_ups/insta_kill.glb", Vec3::new(0.0, 0.037, 0.0)),
        PowerUp::DoublePoints => ("models/power_ups/double_points.glb", Vec3::new(0.009, 0.056, 0.01)),
        PowerUp::Nuke => ("models/power_ups/nuke.glb", Vec3::new(1.0, 0.0, 0.0)),
        PowerUp::BonusPoints => ("models/power_ups/bonus_points.glb", Vec3::new(-0.005, 0.0, 0.0)),
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
    pub(crate) scale: [f32; 5],
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
            // Max Ammo, Insta-Kill, Double Points, Nuke, Bonus Points
            // (`PowerUp::ALL`'s order).
            scale: [1.15, 0.35, 4.0, 0.018, 1.1],
            gold_color: [0.4635839, 0.3274692, 0.055239495],
            gold_metallic: 0.58,
            gold_roughness: 0.15,
            gold_emissive: 1.25,
            glow_color: [0.25, 1.0, 0.3],
            glow_brightness: 0.1,
            glow_size: 1.8,
            glow_pulse_speed: 3.0,
            glow_pulse_amount: 0.65,
            glow_swirl: 0.6,
            glow_swirl_speed: 2.15,
            glow_sparks: 0.3,
            glow_pull: 0.92,
            glow_core: 0.35,
            light_lumens: 30_000.0,
            light_range: 2.2,
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
}

/// The spinning, bobbing part of an avatar (the model under it).
#[derive(Component)]
struct DropSpinner;

/// A model scene; gilded once it's spawned.
#[derive(Component)]
struct DropModel(PowerUp);

#[derive(Component)]
struct DropLight;

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
        .add_systems(OnEnter(AppState::InGame), spawn_timed_row)
        .add_systems(
            Update,
            (
                sync_drop_avatars,
                animate_drop_avatars,
                fade_drop_loops,
                apply_power_up_settings,
                receive_grabs,
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
    avatars: Query<(Entity, &DropAvatar)>,
    settings: Res<PowerUpSettings>,
    have: Option<Res<PowerUpAssets>>,
    sounds: Res<GameSounds>,
    asset_server: Res<AssetServer>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut glow_materials: ResMut<Assets<PowerUpGlowMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    for (e, avatar) in &avatars {
        if drops.get(avatar.src).is_err() {
            commands.entity(e).despawn();
        }
    }
    for (src, drop) in &drops {
        if avatars.iter().any(|(_, a)| a.src == src) {
            continue;
        }
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
                            Mesh3d(quad),
                            MeshMaterial3d(glow),
                            Transform::default(),
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
                // Spawned silent; `fade_drop_loops` sets its volume by
                // distance every frame.
                avatar.spawn((
                    DropLoop,
                    RemoteSoundEmitter,
                    AudioPlayer::new(sounds.power_up_loop.clone()),
                    PlaybackSettings::LOOP
                        .with_spatial(true)
                        .with_spatial_scale(bevy::audio::SpatialScale::new(0.01))
                        .with_volume(Volume::Linear(0.0)),
                    Transform::from_xyz(0.0, 1.0, 0.0),
                ));
            });
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

/// Float, spin, bob — and blink once the server says it's about to go.
fn animate_drop_avatars(
    time: Res<Time>,
    settings: Res<PowerUpSettings>,
    drops: Query<&PowerUpDrop>,
    mut avatars: Query<(&DropAvatar, &Children, &mut Visibility)>,
    mut spinners: Query<&mut Transform, With<DropSpinner>>,
) {
    let t = time.elapsed_secs();
    for (avatar, children, mut vis) in &mut avatars {
        let blinking = drops.get(avatar.src).is_ok_and(|d| d.blinking);
        let on = !blinking || (t * settings.blink_hz.max(0.1) * 2.0).floor() as i64 % 2 == 0;
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

/// Keep each drop's loop matched to its distance from the listener.
fn fade_drop_loops(
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    vols: Res<SoundVolumes>,
    remote: Res<RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut loops: Query<(&GlobalTransform, &mut SpatialAudioSink), With<DropLoop>>,
) {
    let Ok(ear) = listener.single() else { return };
    let ear = ear.translation();
    for (gt, mut sink) in &mut loops {
        let loudness = vols.power_up_loop
            * distance_falloff(ear.distance(gt.translation()), &remote)
            * remote.volume;
        sink.set_volume(Volume::Linear(loudness.max(0.0)) * global_volume.volume);
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
/// for whoever it was, and Max Ammo fills our ammo.
fn receive_grabs(
    mut receivers: Query<&mut MessageReceiver<PowerUpGrabbed>>,
    local: Query<&LocalId, With<GameClient>>,
    sounds: Res<GameSounds>,
    mut weapon: ResMut<Weapon>,
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
            if msg.kind == PowerUp::MaxAmmo {
                weapon.fill_ammo(GameMode::Zombies);
                weapon.throwing_knives = weapon
                    .throwing_knives
                    .max(shared::throwing_knife::starting_knives(GameMode::Zombies));
                // Carrying molotovs: those fill up too.
                if weapon.lethal == crate::Lethal::Molotov {
                    weapon.molotovs = shared::molotov::MAX_MOLOTOVS;
                }
            }
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
    let flicker_off = (time.elapsed_secs() * 6.0).floor() as i64 % 2 == 1;
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
            let alpha = if secs <= 5 && flicker_off { 0.35 } else { 1.0 };
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
