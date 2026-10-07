//! Flash bangs, client side (`Zombies` only — a tactical; the server owns
//! the flight, the fuse, the zombies' stun and the drops: `server::flash_bangs`,
//! `shared::flash_bang`):
//!
//! * **Held** — with flash bangs carried (`Weapon::tactical`), holding the
//!   tactical key brings up the throwing arms holding
//!   `models/weapons/flash_bang_4k.glb` ([`HeldFlashBang`]); releasing
//!   throws it, and — unlike a frag, with no pin out yet — swapping weapons
//!   puts it away again (`weapon.rs`, sent by [`send_throw_requests`]).
//! * **Thrown** — every player sees each [`ThrownFlashBang`] bounce and roll
//!   as `models/weapons/flash_bang_1k.glb` ([`FlashAvatar`]).
//! * **Going off** ([`FlashBangDetonated`]) — a Call of Duty style flash
//!   for everyone: a blinding white core, an anamorphic glare streak, a
//!   burst of white-hot sparks, a hanging cloud of pale smoke and a split
//!   second of very bright light ([`FlashParticle`]), with its bang from
//!   there. Its thrower alone — close enough, with it in sight — gets their
//!   screen whited out for longer the closer (and the more face-on) they
//!   were ([`FlashBlind`], `shared::flash_bang::blind_secs`).
//! * **Dropped** — a zombie's dropped flash bang lies where it fell,
//!   outlined like a dropped molotov ([`DropAvatar`]); walking over it picks
//!   it up (`knife_pickup`, [`receive_pickups`]).
//!
//! Its look in the hand, in the world and going off is tuned in the debug
//! panel's "Flash bang (Zombies)" section ([`flash_section`]), which can also
//! show one in the hand, hold the arms up, preview the flash and the
//! white-out.
//!
//! Every entity here is `StateScoped(InGame)` or a child of one, each avatar
//! goes as soon as its server entity does, the white-out ([`FlashBlind`])
//! and the debug poses are reset on leaving the game, and the flash bang
//! count lives on `Weapon` — nothing carries into the next game.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy_egui::egui;
use bevy_rapier3d::prelude::*;
use lightyear::prelude::*;
use shared::flash_bang::{blind_secs, BLIND_RADIUS};
use shared::{FlashBangDetonated, FlashBangDrop, ThrownFlashBang};

use crate::killcam::ActiveKillCam;
use crate::knife_pickup::OutlineWhenLoaded;
use crate::net::GameClient;
use crate::player::{Player, PlayerHead};
use crate::util::rand01;
use crate::{
    cone_dir, smoke_billboard_rotation, start_throw_knife_model, AppState, ExplosionAssets, GameSounds, Shake,
    ThrowArmsViewModel, ThrowingKnife, Weapon, VIEW_MODEL_RENDER_LAYER,
};

/// The one in hand, and the one everyone sees thrown or dropped.
pub(crate) const HELD_MODEL: &str = "models/weapons/flash_bang_4k.glb";
pub(crate) const WORLD_MODEL: &str = "models/weapons/flash_bang_1k.glb";
/// `flash_bang_*.glb` as made: its middle and height (model units — it's
/// ~0.058 tall, standing up `+Y`).
const MODEL_MIDDLE: Vec3 = Vec3::new(-0.00165, -0.0076, 0.0);
const MODEL_HEIGHT: f32 = 0.0583;
/// How long (m) it is in the Mystery Box.
pub(crate) const BOX_LENGTH: f32 = 0.18;
/// Hard cap on live flash sprites.
const PARTICLE_MAX: usize = 300;

/// The model `height` m tall, its middle on the origin.
fn centered_transform(height: f32) -> Transform {
    let scale = height / MODEL_HEIGHT;
    Transform::from_translation(-MODEL_MIDDLE * scale).with_scale(Vec3::splat(scale))
}

/// The model `height` m tall, lying on its side on the ground.
fn lying_transform(height: f32) -> Transform {
    let mut t = centered_transform(height);
    let turn = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    t.translation = turn * t.translation + Vec3::Y * height * 0.17;
    t.rotation = turn;
    t
}

pub(crate) struct FlashBangPlugin;

impl Plugin for FlashBangPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlashBangSettings>()
            .init_resource::<FlashBlind>()
            .add_systems(Startup, setup_assets)
            .add_systems(OnEnter(AppState::InGame), spawn_blind_overlay)
            .add_systems(
                OnExit(AppState::InGame),
                |mut s: ResMut<FlashBangSettings>, mut blind: ResMut<FlashBlind>| {
                    s.debug_arms_out = false;
                    s.debug_show_in_hand = false;
                    *blind = FlashBlind::default();
                },
            )
            .add_systems(
                Update,
                (
                    spawn_held_flash,
                    update_held_flash,
                    send_throw_requests,
                    sync_flash_avatars,
                    sync_drop_avatars,
                    receive_pickups,
                    (receive_detonations, fire_preview),
                    update_flash_particles,
                    update_flash_lights,
                    update_blind_overlay,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The debug panel's flash bang tuning.
#[derive(Resource, Clone)]
pub(crate) struct FlashBangSettings {
    // --- held in the throwing arms (the arms' local space, like the frag's;
    // the flash bang's middle is where this puts it) ---
    pub(crate) held_translation: Vec3,
    pub(crate) held_yaw: f32,
    pub(crate) held_pitch: f32,
    pub(crate) held_roll: f32,
    /// Model units to arm units (it's ~0.058 model units tall).
    pub(crate) held_scale: f32,
    /// How tall (m) it is thrown...
    pub(crate) world_height: f32,
    /// ...and dropped, waiting to be picked up.
    pub(crate) drop_height: f32,

    // --- going off ---
    /// The blinding core: size (m), how long (s) and how bright (HDR).
    pub(crate) core_size: f32,
    pub(crate) core_time: f32,
    pub(crate) core_brightness: f32,
    /// The softer bloom round it.
    pub(crate) bloom_size: f32,
    pub(crate) bloom_time: f32,
    pub(crate) bloom_brightness: f32,
    /// The horizontal glare streak: length and thickness (m), how long, how
    /// bright.
    pub(crate) glare_length: f32,
    pub(crate) glare_thickness: f32,
    pub(crate) glare_time: f32,
    pub(crate) glare_brightness: f32,
    pub(crate) spark_count: u32,
    pub(crate) spark_speed: f32,
    pub(crate) spark_size: f32,
    pub(crate) spark_life: f32,
    pub(crate) spark_brightness: f32,
    pub(crate) smoke_count: u32,
    pub(crate) smoke_size: f32,
    pub(crate) smoke_speed: f32,
    pub(crate) smoke_life: f32,
    pub(crate) smoke_opacity: f32,
    /// The light: lumens, range (m), how long (s).
    pub(crate) light_intensity: f32,
    pub(crate) light_range: f32,
    pub(crate) light_time: f32,
    /// Camera-shake trauma right on top of it, and how far (m) it reaches.
    pub(crate) shake: f32,
    pub(crate) shake_radius: f32,
    /// Distance (m) at which its bang has faded to silence.
    pub(crate) sound_max_distance: f32,
    /// Of the thrower's white-out, the share spent fully white before it
    /// starts fading.
    pub(crate) blind_hold: f32,

    // --- debug poses (never saved; off on leaving the game) ---
    /// Show a flash bang in the arms' hand whenever they're up.
    pub(crate) debug_show_in_hand: bool,
    /// Hold the arms up as if the key were held, without throwing anything
    /// (shared with the frag's — `weapon.rs`' `ThrowingKnife::dry`).
    pub(crate) debug_arms_out: bool,
    /// Preview buttons, consumed by `fire_preview`.
    preview_flash: bool,
    preview_blind: bool,
}

impl Default for FlashBangSettings {
    fn default() -> Self {
        Self {
            held_translation: Vec3::new(-13.0, -8.0, -1.0),
            held_yaw: 15.0,
            held_pitch: 25.0,
            held_roll: -35.0,
            held_scale: 220.0,
            world_height: 0.13,
            drop_height: 0.13,

            core_size: 5.0,
            core_time: 0.12,
            core_brightness: 40.0,
            bloom_size: 14.0,
            bloom_time: 0.45,
            bloom_brightness: 4.0,
            glare_length: 16.0,
            glare_thickness: 0.35,
            glare_time: 0.3,
            glare_brightness: 12.0,
            spark_count: 40,
            spark_speed: 14.0,
            spark_size: 0.05,
            spark_life: 0.35,
            spark_brightness: 14.0,
            smoke_count: 14,
            smoke_size: 2.6,
            smoke_speed: 2.2,
            smoke_life: 3.5,
            smoke_opacity: 0.4,
            light_intensity: 10_000_000.0,
            light_range: 28.0,
            light_time: 0.2,
            shake: 0.25,
            shake_radius: 18.0,
            sound_max_distance: 140.0,
            blind_hold: 0.55,

            debug_show_in_hand: false,
            debug_arms_out: false,
            preview_flash: false,
            preview_blind: false,
        }
    }
}

impl FlashBangSettings {
    fn held_transform(&self) -> Transform {
        Transform {
            translation: self.held_translation,
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                self.held_yaw.to_radians(),
                self.held_pitch.to_radians(),
                self.held_roll.to_radians(),
            ),
            scale: Vec3::splat(self.held_scale.max(1e-4)),
        }
    }
}

#[derive(Resource)]
struct FlashAssets {
    /// Loaded up front (and kept) so the first one picked up or thrown
    /// doesn't hitch.
    held: Handle<Scene>,
    world: Handle<Scene>,
}

fn setup_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(FlashAssets {
        held: asset_server.load(GltfAssetLabel::Scene(0).from_asset(HELD_MODEL)),
        world: asset_server.load(GltfAssetLabel::Scene(0).from_asset(WORLD_MODEL)),
    });
}

// --- held --------------------------------------------------------------------

/// The flash bang in the throwing arms' hand (its pivot: its middle).
#[derive(Component)]
struct HeldFlashBang;

/// Give each (new) throwing-arms rig its flash bang, on the view-model layer.
fn spawn_held_flash(
    arms: Query<Entity, Added<ThrowArmsViewModel>>,
    settings: Res<FlashBangSettings>,
    assets: Res<FlashAssets>,
    mut commands: Commands,
) {
    for arms in &arms {
        let vm = RenderLayers::layer(VIEW_MODEL_RENDER_LAYER);
        commands
            .spawn((HeldFlashBang, settings.held_transform(), vm.clone(), Visibility::Hidden, ChildOf(arms)))
            .with_children(|pivot| {
                pivot
                    .spawn((SceneRoot(assets.held.clone()), Transform::from_translation(-MODEL_MIDDLE), vm))
                    .observe(start_throw_knife_model);
            });
    }
}

/// Pose the held flash bang from the settings every frame and show it while
/// it's in the hand, or whenever the debug panel says. Hidden during a kill
/// cam.
fn update_held_flash(
    settings: Res<FlashBangSettings>,
    knife: Res<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    mut held: Query<(&mut Transform, &mut Visibility), With<HeldFlashBang>>,
) {
    let shown = (knife.flash_in_hand() || settings.debug_show_in_hand) && killcam.0.is_none();
    for (mut tf, mut vis) in &mut held {
        tf.set_if_neq(settings.held_transform());
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Send the flash bang throw `weapon_system` filed when it left the hand.
fn send_throw_requests(
    mut knife: ResMut<ThrowingKnife>,
    mut sender: Query<&mut TriggerSender<shared::ThrowFlashBang>, With<GameClient>>,
) {
    if !knife.has_flash_request() {
        return;
    }
    let Some((origin, dir)) = knife.take_throw_request() else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::ThrowFlashBang {
            origin: origin.to_array(),
            dir: dir.to_array(),
        });
    }
}

// --- thrown and dropped --------------------------------------------------------------

/// Stands in for one server-owned [`ThrownFlashBang`].
#[derive(Component)]
struct FlashAvatar {
    src: Entity,
}

/// The avatar's model (scaled, its middle on the avatar's origin).
#[derive(Component)]
struct FlashModel;

/// Keep one avatar per flash bang, following the interpolated flight; drop
/// it once the server removes it (it's gone off). Hidden during a kill cam.
#[allow(clippy::too_many_arguments)]
fn sync_flash_avatars(
    flashes: Query<(Entity, &ThrownFlashBang), With<Interpolated>>,
    all: Query<&ThrownFlashBang>,
    mut avatars: Query<(Entity, &FlashAvatar, &mut Transform, &mut Visibility), Without<FlashModel>>,
    mut models: Query<&mut Transform, (With<FlashModel>, Without<FlashAvatar>)>,
    killcam: Res<ActiveKillCam>,
    settings: Res<FlashBangSettings>,
    assets: Res<FlashAssets>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    let mut have = Vec::new();
    for (entity, avatar, mut tf, mut vis) in &mut avatars {
        match all.get(avatar.src) {
            Ok(f) => {
                tf.translation = f.pos;
                tf.rotation = f.rot;
                vis.set_if_neq(wanted);
                have.push(avatar.src);
            }
            Err(_) => commands.entity(entity).try_despawn(),
        }
    }
    for mut tf in &mut models {
        tf.set_if_neq(centered_transform(settings.world_height));
    }
    for (src, f) in &flashes {
        if have.contains(&src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            FlashAvatar { src },
            Transform::from_translation(f.pos).with_rotation(f.rot),
            Visibility::default(),
            children![(FlashModel, SceneRoot(assets.world.clone()), centered_transform(settings.world_height))],
        ));
    }
}

/// Stands in for one dropped [`FlashBangDrop`], outlined like a dropped
/// molotov.
#[derive(Component)]
struct DropAvatar {
    src: Entity,
}

/// A dropped flash bang's model (under its [`DropAvatar`]).
#[derive(Component)]
struct DroppedModel;

/// One avatar per dropped flash bang, lying where it fell; gone with it.
fn sync_drop_avatars(
    drops: Query<(Entity, &FlashBangDrop)>,
    mut avatars: Query<(Entity, &DropAvatar, &mut Visibility)>,
    mut models: Query<&mut Transform, With<DroppedModel>>,
    killcam: Res<ActiveKillCam>,
    settings: Res<FlashBangSettings>,
    assets: Res<FlashAssets>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    for mut tf in &mut models {
        tf.set_if_neq(lying_transform(settings.drop_height));
    }
    let mut have = Vec::new();
    for (entity, avatar, mut vis) in &mut avatars {
        if drops.contains(avatar.src) {
            vis.set_if_neq(wanted);
            have.push(avatar.src);
        } else {
            commands.entity(entity).try_despawn();
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
            Transform::from_translation(d.pos).with_rotation(Quat::from_rotation_y(d.yaw)),
            Visibility::default(),
            children![(DroppedModel, SceneRoot(assets.world.clone()), lying_transform(settings.drop_height))],
        ));
    }
}

/// The server handed us a flash bang we picked up — and the pickup sound
/// plays, just for us.
fn receive_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::FlashBangPickedUp>>,
    mut weapon: ResMut<Weapon>,
    mut pending: ResMut<crate::knife_pickup::PendingPickup>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.add_flash_bang();
            pending.0 = None;
            commands.spawn((AudioPlayer::new(sounds.pick_up_equipment.clone()), PlaybackSettings::DESPAWN));
        }
    }
}

// --- going off -----------------------------------------------------------------------

/// How long our screen's whited out from our own flash bang, and how far in.
#[derive(Resource, Default)]
struct FlashBlind {
    total: f32,
    age: f32,
}

/// The white-out, over the whole screen.
#[derive(Component)]
struct BlindOverlay;

fn spawn_blind_overlay(mut commands: Commands) {
    commands.spawn((
        StateScoped(AppState::InGame),
        BlindOverlay,
        GlobalZIndex(41),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::WHITE.with_alpha(0.0)),
        bevy::ui::FocusPolicy::Pass,
    ));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Core,
    Bloom,
    Glare,
    Spark,
    Smoke,
}

/// One flash sprite, living `lifetime` seconds.
#[derive(Component)]
struct FlashParticle {
    kind: Kind,
    velocity: Vec3,
    gravity: f32,
    drag: f32,
    age: f32,
    lifetime: f32,
    roll: f32,
    scale0: f32,
    scale1: f32,
    peak: f32,
}

/// The flash's light, gone after `lifetime`.
#[derive(Component)]
struct FlashLight {
    age: f32,
    lifetime: f32,
    peak: f32,
}

/// A flash bang went off: the flash and its bang from there for everyone,
/// and — if it was ours, near enough and in sight — our white-out.
#[allow(clippy::too_many_arguments)]
fn receive_detonations(
    mut receivers: Query<&mut MessageReceiver<FlashBangDetonated>>,
    local: Query<&LocalId, With<GameClient>>,
    camera: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    rapier: ReadRapierContext,
    settings: Res<FlashBangSettings>,
    assets: Res<ExplosionAssets>,
    sounds: Res<GameSounds>,
    volumes: Res<crate::SoundVolumes>,
    existing: Query<(), With<FlashParticle>>,
    mut shake: ResMut<Shake>,
    mut blind: ResMut<FlashBlind>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let me = local.iter().next().map(|l| l.0);
    let eye = camera.single().ok().map(|c| (c.translation(), c.forward().as_vec3()));
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let at = Vec3::from_array(msg.at);
            *seq = seq.wrapping_add(1);
            spawn_flash(
                at,
                *seq,
                eye.map(|(e, _)| e),
                &settings,
                &assets,
                &sounds,
                &volumes,
                existing.iter().count(),
                &mut shake,
                &mut materials,
                &mut commands,
            );
            // Ours: whited out if we were close and could see it.
            let Some((eye, forward)) = eye.filter(|_| Some(msg.owner) == me) else {
                continue;
            };
            let to = eye - at;
            let distance = to.length();
            if distance >= BLIND_RADIUS {
                continue;
            }
            let in_sight = distance < 0.3
                || rapier.single().is_ok_and(|r| {
                    r.cast_ray(at, to / distance, distance - 0.3, true, QueryFilter::only_fixed())
                        .is_none()
                });
            if !in_sight {
                continue;
            }
            let secs = blind_secs(distance, forward.dot(-to / distance.max(1e-3)));
            if secs > blind.total - blind.age {
                *blind = FlashBlind { total: secs, age: 0.0 };
            }
        }
    }
}

/// The debug panel's previews: a flash 6 m in front of us (no blinding),
/// and the white-out as if we'd been right on top of one.
#[allow(clippy::too_many_arguments)]
fn fire_preview(
    mut settings: ResMut<FlashBangSettings>,
    player: Single<&Transform, With<Player>>,
    assets: Res<ExplosionAssets>,
    sounds: Res<GameSounds>,
    volumes: Res<crate::SoundVolumes>,
    existing: Query<(), With<FlashParticle>>,
    mut shake: ResMut<Shake>,
    mut blind: ResMut<FlashBlind>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    if std::mem::take(&mut settings.bypass_change_detection().preview_blind) {
        *blind = FlashBlind {
            total: shared::flash_bang::BLIND_MAX_SECS,
            age: 0.0,
        };
    }
    if !std::mem::take(&mut settings.bypass_change_detection().preview_flash) {
        return;
    }
    *seq = seq.wrapping_add(0x9e37);
    let mut fwd = player.rotation * Vec3::NEG_Z;
    fwd.y = 0.0;
    let at = player.translation - Vec3::Y * (crate::player::EYE_HEIGHT - 0.3) + fwd.normalize_or(Vec3::NEG_Z) * 6.0;
    spawn_flash(
        at,
        *seq,
        Some(player.translation),
        &settings,
        &assets,
        &sounds,
        &volumes,
        existing.iter().count(),
        &mut shake,
        &mut materials,
        &mut commands,
    );
}

/// An additive, unlit sprite material, invisible until
/// `update_flash_particles` first reaches it.
fn sprite_material(texture: Handle<Image>, additive: bool) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
        base_color_texture: Some(texture),
        unlit: true,
        alpha_mode: if additive { AlphaMode::Add } else { AlphaMode::Blend },
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// One flash going off at `at`: its sprites, its light, a jolt of camera
/// shake and its bang (fading with our distance, `listener`).
#[allow(clippy::too_many_arguments)]
fn spawn_flash(
    at: Vec3,
    seed: u32,
    listener: Option<Vec3>,
    s: &FlashBangSettings,
    assets: &ExplosionAssets,
    sounds: &GameSounds,
    volumes: &crate::SoundVolumes,
    live: usize,
    shake: &mut Shake,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
) {
    let center = at + Vec3::Y * 0.15;
    let dist = listener.map_or(f32::INFINITY, |l| l.distance(center));

    // The bang, from there.
    let fade = (1.0 - dist / s.sound_max_distance.max(1.0)).clamp(0.0, 1.0);
    let loudness = volumes.flash_bang_detonate * fade * fade;
    if loudness > 0.0 {
        commands.spawn((
            StateScoped(AppState::InGame),
            crate::RemoteSoundEmitter,
            AudioPlayer::new(sounds.flash_bang_detonate.clone()),
            Transform::from_translation(center),
            crate::positional_playback(bevy::audio::Volume::Linear(loudness)),
        ));
    }

    // A small jolt.
    let falloff = (1.0 - dist / s.shake_radius.max(0.1)).clamp(0.0, 1.0);
    shake.trauma = (shake.trauma + s.shake * falloff * falloff).min(1.0);

    // A split second of very bright, cold light.
    commands.spawn((
        StateScoped(AppState::InGame),
        FlashLight {
            age: 0.0,
            lifetime: s.light_time.max(0.02),
            peak: s.light_intensity,
        },
        PointLight {
            color: Color::srgb(0.92, 0.96, 1.0),
            intensity: s.light_intensity,
            range: s.light_range,
            radius: 0.3,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(center + Vec3::Y * 0.3),
    ));

    let mut budget = PARTICLE_MAX.saturating_sub(live);
    let mut spawn = |p: FlashParticle, texture: Handle<Image>, additive: bool, pos: Vec3| {
        if budget == 0 {
            return;
        }
        budget -= 1;
        commands.spawn((
            StateScoped(AppState::InGame),
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(materials.add(sprite_material(texture, additive))),
            Transform::from_translation(pos).with_scale(Vec3::splat(p.scale0.max(1e-4))),
            p,
            NoFrustumCulling,
            NotShadowCaster,
        ));
    };
    let particle = |kind, lifetime: f32, scale0: f32, scale1: f32, peak: f32| FlashParticle {
        kind,
        velocity: Vec3::ZERO,
        gravity: 0.0,
        drag: 0.0,
        age: 0.0,
        lifetime: lifetime.max(0.02),
        roll: 0.0,
        scale0,
        scale1,
        peak,
    };

    // The blinding core, the bloom round it and the glare across it.
    spawn(particle(Kind::Core, s.core_time, s.core_size * 0.5, s.core_size, s.core_brightness), assets.glow.clone(), true, center);
    spawn(particle(Kind::Bloom, s.bloom_time, s.bloom_size * 0.4, s.bloom_size, s.bloom_brightness), assets.glow.clone(), true, center);
    spawn(particle(Kind::Glare, s.glare_time, s.glare_length * 0.6, s.glare_length, s.glare_brightness), assets.glow.clone(), true, center);

    // White-hot sparks, fast and short-lived.
    for i in 0..s.spark_count {
        let r = seed.wrapping_mul(2_654_435_761).wrapping_add(i.wrapping_mul(40_503)).wrapping_add(0xf1a5);
        let dir = cone_dir(Vec3::Y, 150f32.to_radians(), r);
        let speed = s.spark_speed * (0.35 + 0.65 * rand01(r ^ 0x9e37));
        let size = s.spark_size * (0.6 + 0.8 * rand01(r ^ 0x2c));
        let mut p = particle(Kind::Spark, s.spark_life * (0.5 + 0.8 * rand01(r ^ 0x1234)), size, 0.0, s.spark_brightness);
        p.velocity = dir * speed;
        p.gravity = 9.8;
        p.drag = 2.0;
        spawn(p, assets.glow.clone(), true, center);
    }

    // A pale cloud that hangs where it went off.
    for i in 0..s.smoke_count {
        let r = seed.wrapping_mul(83_492_791).wrapping_add(i.wrapping_mul(1_597_334_677)).wrapping_add(0x5a0e);
        let dir = cone_dir(Vec3::Y, 80f32.to_radians(), r);
        let speed = s.smoke_speed * (0.3 + 0.7 * rand01(r ^ 0x9e37));
        let size = s.smoke_size * (0.7 + 0.6 * rand01(r ^ 0x2c));
        let mut p = particle(Kind::Smoke, s.smoke_life * (0.6 + 0.8 * rand01(r ^ 0x1234)), size * 0.35, size, s.smoke_opacity);
        p.velocity = dir * speed;
        p.gravity = -0.35;
        p.drag = 1.8;
        p.roll = rand01(r ^ 0x77) * std::f32::consts::TAU;
        spawn(p, assets.smoke.clone(), false, center + dir * 0.3);
    }
}

/// Integrate, billboard, grow and colour every flash sprite; despawn
/// (freeing its material) at the end of its life.
#[allow(clippy::type_complexity)]
fn update_flash_particles(
    time: Res<Time>,
    settings: Res<FlashBangSettings>,
    player: Single<&Transform, (With<Player>, Without<FlashParticle>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<FlashParticle>)>,
    mut particles: Query<(Entity, &mut Transform, &mut FlashParticle, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cam_pos = player.translation;
    let cam_rot = player.rotation * head.rotation;
    let cam_up = cam_rot * Vec3::Y;
    let cam_right = cam_rot * Vec3::X;
    for (entity, mut tf, mut p, material) in &mut particles {
        p.age += dt;
        if p.age >= p.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        let (gravity, drag) = (p.gravity, p.drag);
        p.velocity.y -= gravity * dt;
        p.velocity *= (1.0 - drag * dt).max(0.0);
        tf.translation += p.velocity * dt;

        let f = (p.age / p.lifetime).clamp(0.0, 1.0);
        let grow = 1.0 - (1.0 - f).powi(3);
        let scale = (p.scale0 + (p.scale1 - p.scale0) * grow).max(1e-4);
        let color = match p.kind {
            Kind::Core | Kind::Bloom => {
                tf.scale = Vec3::splat(scale);
                let b = p.peak * (1.0 - f).powi(2);
                LinearRgba::new(b * 0.95, b * 0.97, b, 1.0)
            }
            Kind::Glare => {
                // Long and thin, screen-horizontal.
                tf.scale = Vec3::new(scale, settings.glare_thickness.max(1e-3), 1.0);
                let b = p.peak * (1.0 - f).powf(1.5);
                LinearRgba::new(b * 0.8, b * 0.9, b, 1.0)
            }
            Kind::Spark => {
                let len = p.scale0 + p.velocity.length() * 0.05;
                tf.scale = Vec3::new(p.scale0.max(1e-4), len.max(1e-4), 1.0);
                let b = p.peak * (1.0 - f);
                LinearRgba::new(b, b, b * 0.95, 1.0)
            }
            Kind::Smoke => {
                tf.scale = Vec3::splat(scale);
                let a = (f / 0.08).min(1.0) * (1.0 - f).powf(1.4);
                LinearRgba::from(Color::srgba(0.82, 0.83, 0.85, p.peak * a))
            }
        };
        tf.rotation = if p.kind == Kind::Spark {
            // Stretched along its flight, as seen from the camera.
            let to_cam = (cam_pos - tf.translation).normalize_or(Vec3::Y);
            let along = p.velocity - to_cam * p.velocity.dot(to_cam);
            if along.length_squared() > 1e-6 {
                let up = along.normalize();
                Quat::from_mat3(&Mat3::from_cols(up.cross(to_cam), up, to_cam))
            } else {
                smoke_billboard_rotation(tf.translation, cam_pos, cam_up, cam_right, 0.0)
            }
        } else {
            smoke_billboard_rotation(tf.translation, cam_pos, cam_up, cam_right, p.roll)
        };
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::LinearRgba(color);
        }
    }
}

fn update_flash_lights(
    time: Res<Time>,
    mut lights: Query<(Entity, &mut FlashLight, &mut PointLight)>,
    mut commands: Commands,
) {
    for (entity, mut l, mut light) in &mut lights {
        l.age += time.delta_secs();
        if l.age >= l.lifetime {
            commands.entity(entity).despawn();
            continue;
        }
        let f = l.age / l.lifetime;
        light.intensity = l.peak * (1.0 - f).powi(2);
    }
}

/// Our white-out: fully white for the first [`FlashBangSettings::blind_hold`]
/// of it, then fading back.
fn update_blind_overlay(
    time: Res<Time>,
    settings: Res<FlashBangSettings>,
    mut blind: ResMut<FlashBlind>,
    mut overlay: Single<&mut BackgroundColor, With<BlindOverlay>>,
) {
    let alpha = if blind.total > 0.0 && blind.age < blind.total {
        blind.age += time.delta_secs();
        let f = (blind.age / blind.total).clamp(0.0, 1.0);
        let hold = settings.blind_hold.clamp(0.0, 0.95);
        if f < hold {
            1.0
        } else {
            (1.0 - (f - hold) / (1.0 - hold)).powf(1.6)
        }
    } else {
        0.0
    };
    let c = Color::WHITE.with_alpha(alpha);
    if overlay.0 != c {
        overlay.0 = c;
    }
}

// --- debug panel -------------------------------------------------------------------

/// The debug panel's "Flash bang (Zombies)" section: give yourself some, pose
/// the arms and one in them, its look in the hand and the world, and its
/// flash — with previews of the flash and the white-out.
pub(crate) fn flash_section(ui: &mut egui::Ui, s: &mut FlashBangSettings, weapon: &mut Weapon) {
    ui.horizontal(|ui| {
        if ui.button("Give me a flash bang").clicked() {
            weapon.add_flash_bang();
        }
        ui.label(format!("carrying {}", weapon.flash_bangs));
    });
    ui.checkbox(&mut s.debug_show_in_hand, "Show a flash bang in the hand");
    ui.checkbox(&mut s.debug_arms_out, "Hold the throwing arms up (as if the tactical key were held)");
    ui.label(format!(
        "Goes off {:.1} s after it's thrown. Stuns zombies within {:.0} m for {:.0} s; blinds only its thrower, within {:.0} m.",
        shared::flash_bang::FUSE_SECS,
        shared::flash_bang::STUN_RADIUS,
        shared::flash_bang::STUN_SECS,
        BLIND_RADIUS,
    ));
    ui.collapsing("Held flash bang (4k model)", |ui| {
        ui.label(
            "Positioned in the throwing arms' local space, like the frag: one unit = the arms' scale in \
             metres (~cm by default). The position is the flash bang's middle. Tick both boxes above to see it.",
        );
        ui.add(egui::Slider::new(&mut s.held_translation.x, -100.0f32..=100.0).text("x"));
        ui.add(egui::Slider::new(&mut s.held_translation.y, -100.0f32..=100.0).text("y"));
        ui.add(egui::Slider::new(&mut s.held_translation.z, -100.0f32..=100.0).text("z"));
        ui.add(egui::Slider::new(&mut s.held_yaw, -180.0f32..=180.0).text("yaw (°)"));
        ui.add(egui::Slider::new(&mut s.held_pitch, -180.0f32..=180.0).text("pitch (°)"));
        ui.add(egui::Slider::new(&mut s.held_roll, -180.0f32..=180.0).text("roll (°)"));
        ui.add(egui::Slider::new(&mut s.held_scale, 10.0f32..=1000.0).text("scale").logarithmic(true));
    });
    ui.collapsing("Thrown / dropped (1k model)", |ui| {
        ui.add(egui::Slider::new(&mut s.world_height, 0.03f32..=0.5).text("thrown height (m)"));
        ui.add(egui::Slider::new(&mut s.drop_height, 0.03f32..=0.5).text("dropped height (m)"));
    });
    ui.collapsing("Going off", |ui| {
        ui.horizontal(|ui| {
            if ui.button("Preview the flash").clicked() {
                s.preview_flash = true;
            }
            if ui.button("Preview the white-out").clicked() {
                s.preview_blind = true;
            }
        });
        ui.label("Core");
        ui.add(egui::Slider::new(&mut s.core_size, 0.5f32..=20.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.core_time, 0.02f32..=1.0).text("time (s)"));
        ui.add(egui::Slider::new(&mut s.core_brightness, 1.0f32..=200.0).text("brightness").logarithmic(true));
        ui.label("Bloom");
        ui.add(egui::Slider::new(&mut s.bloom_size, 1.0f32..=40.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.bloom_time, 0.05f32..=2.0).text("time (s)"));
        ui.add(egui::Slider::new(&mut s.bloom_brightness, 0.1f32..=50.0).text("brightness").logarithmic(true));
        ui.label("Glare streak");
        ui.add(egui::Slider::new(&mut s.glare_length, 1.0f32..=50.0).text("length (m)"));
        ui.add(egui::Slider::new(&mut s.glare_thickness, 0.05f32..=3.0).text("thickness (m)"));
        ui.add(egui::Slider::new(&mut s.glare_time, 0.05f32..=2.0).text("time (s)"));
        ui.add(egui::Slider::new(&mut s.glare_brightness, 0.1f32..=100.0).text("brightness").logarithmic(true));
        ui.label("Sparks");
        ui.add(egui::Slider::new(&mut s.spark_count, 0u32..=150).text("count"));
        ui.add(egui::Slider::new(&mut s.spark_speed, 1.0f32..=40.0).text("speed"));
        ui.add(egui::Slider::new(&mut s.spark_size, 0.01f32..=0.3).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.spark_life, 0.05f32..=2.0).text("life (s)"));
        ui.add(egui::Slider::new(&mut s.spark_brightness, 0.5f32..=60.0).text("brightness").logarithmic(true));
        ui.label("Smoke");
        ui.add(egui::Slider::new(&mut s.smoke_count, 0u32..=60).text("count"));
        ui.add(egui::Slider::new(&mut s.smoke_size, 0.2f32..=10.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.smoke_speed, 0.0f32..=10.0).text("speed"));
        ui.add(egui::Slider::new(&mut s.smoke_life, 0.2f32..=10.0).text("life (s)"));
        ui.add(egui::Slider::new(&mut s.smoke_opacity, 0.0f32..=1.0).text("opacity"));
        ui.label("Light, shake, sound, white-out");
        ui.add(egui::Slider::new(&mut s.light_intensity, 0.0f32..=50_000_000.0).text("light (lm)"));
        ui.add(egui::Slider::new(&mut s.light_range, 1.0f32..=80.0).text("light range (m)"));
        ui.add(egui::Slider::new(&mut s.light_time, 0.02f32..=1.5).text("light time (s)"));
        ui.add(egui::Slider::new(&mut s.shake, 0.0f32..=1.0).text("shake"));
        ui.add(egui::Slider::new(&mut s.shake_radius, 1.0f32..=60.0).text("shake radius (m)"));
        ui.add(egui::Slider::new(&mut s.sound_max_distance, 10.0f32..=300.0).text("sound fades out by (m)"));
        ui.add(egui::Slider::new(&mut s.blind_hold, 0.0f32..=0.95).text("white-out: share held fully white"));
    });
    if ui.button("Copy settings to console").clicked() {
        info!(
            "flash bang: held_translation: Vec3::new({:.2}, {:.2}, {:.2}), held_yaw: {:.1}, held_pitch: {:.1}, \
             held_roll: {:.1}, held_scale: {:.2}, world_height: {:.3}, drop_height: {:.3}, core: {:.2} m {:.2} s x{:.1}, \
             bloom: {:.2} m {:.2} s x{:.1}, glare: {:.2}x{:.2} m {:.2} s x{:.1}, sparks: {} @ {:.1} {:.3} m {:.2} s x{:.1}, \
             smoke: {} {:.2} m @ {:.1} {:.2} s a{:.2}, light: {:.0} lm {:.1} m {:.2} s, shake {:.2} / {:.1} m, \
             sound {:.0} m, blind_hold {:.2}",
            s.held_translation.x, s.held_translation.y, s.held_translation.z, s.held_yaw, s.held_pitch, s.held_roll,
            s.held_scale, s.world_height, s.drop_height, s.core_size, s.core_time, s.core_brightness, s.bloom_size,
            s.bloom_time, s.bloom_brightness, s.glare_length, s.glare_thickness, s.glare_time, s.glare_brightness,
            s.spark_count, s.spark_speed, s.spark_size, s.spark_life, s.spark_brightness, s.smoke_count, s.smoke_size,
            s.smoke_speed, s.smoke_life, s.smoke_opacity, s.light_intensity, s.light_range, s.light_time, s.shake,
            s.shake_radius, s.sound_max_distance, s.blind_hold,
        );
    }
    if ui.button("Reset flash bang").clicked() {
        let poses = (s.debug_show_in_hand, s.debug_arms_out);
        *s = FlashBangSettings::default();
        (s.debug_show_in_hand, s.debug_arms_out) = poses;
    }
}
