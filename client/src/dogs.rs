//! Dog rounds' hellhounds (`shared::dogs`), on this client: every fifth
//! `Zombies` round, the server sends only dogs. For each one:
//!
//! * lightning strikes from the ground to the sky where it's about to
//!   appear, with the pre-spawn sound, for as long as that sound plays
//!   ([`shared::DogLightning`], [`shared::dogs::DOG_PRE_SPAWN_SECS`]);
//! * then it's just there, in a flash of light, with its spawn sound
//!   ([`shared::DogSpawned`]);
//! * it runs at you (`models/characters/dog.glb`'s one clip, `run1`, looped
//!   at whatever speed keeps its feet planted — playback ∝ its real speed),
//!   barking the whole time, with fire burning along its back
//!   (the molotov's flames, `molotov::FireMaterial`) that never goes out;
//! * and the moment it dies — blown up on a player or killed — its body is
//!   gone and an explosion goes off right where it was, with the dog
//!   explosion sound ([`shared::DogExploded`], `vfx::Explosion::dog`).
//!
//! Every sound is positional and heard by the whole lobby. Look and feel are
//! tuned in the debug panel's "Dogs (Zombies)" section ([`DogSettings`]). Everything
//! here is `StateScoped(InGame)`; a gone dog's source entity is marked
//! [`DogGone`] so no new avatar is made for it before the server's despawn
//! arrives.

use bevy::audio::{PlaybackMode, SpatialAudioSink, Volume};
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use bevy_egui::egui;
use lightyear::prelude::{Interpolated, MessageReceiver, PeerId};
use shared::{PlayerId, PlayerPose};

use crate::molotov::{flicker, seeded_scale, FireMaterial, MolotovAssets, MolotovSettings};
use crate::net::RemoteAvatar;
use crate::player::Player;
use crate::util::rand01;
use crate::{AppState, GameSounds, SoundVolumes, WorldModelCamera};

pub(crate) const DOG_MODEL: &str = "models/characters/dog.glb";
/// `run1`'s one cycle sits at the very end of its timeline: keys only from
/// here to its end (s).
const RUN_START: f32 = 25.208_334;
const RUN_END: f32 = 25.875;
/// The bone the back fire rides on.
const BACK_BONE: &str = "spine mid_35";
/// A lightning bolt's segments: the main channel's, and each branch's.
const BOLT_SEGMENTS: usize = 22;
const BRANCHES: usize = 3;
const BRANCH_SEGMENTS: usize = 6;
/// Lightning / the spawn flash's colour (linear): a cold electric blue-white
/// for a hellhound...
const STRIKE_COLOR: [f32; 3] = [0.62, 0.76, 1.0];
/// ...and a fiery orange for a boss (`crate::boss`).
pub(crate) const BOSS_STRIKE_COLOR: [f32; 3] = [1.0, 0.45, 0.08];

/// Panel-tunable hellhounds ("Dogs (Zombies)" debug-panel section).
#[derive(Resource, Clone)]
pub(crate) struct DogSettings {
    pub(crate) scale: f32,
    /// Extra turn (degrees) so the model faces the way it runs.
    pub(crate) yaw_offset_deg: f32,
    /// `run1` playback speed per m/s it's really moving — dial in at any
    /// speed until the feet stop sliding, and it holds at every speed.
    pub(crate) run_per_mps: f32,
    /// Below this speed (m/s) the run holds still rather than treading in
    /// place.
    pub(crate) min_move_speed: f32,

    // --- the fire on its back ---
    /// Above the back bone (m).
    pub(crate) fire_height: f32,
    /// Flames, spread this far (m) along the back and across it.
    pub(crate) fire_flames: u32,
    pub(crate) fire_length: f32,
    pub(crate) fire_width: f32,
    /// Shift (m) of the whole fire forward (+) / back (−) along the dog.
    pub(crate) fire_forward: f32,
    /// Each flame's size (m).
    pub(crate) flame_width: f32,
    pub(crate) flame_height: f32,
    pub(crate) fire_brightness: f32,
    /// How far the flames stream back as it runs (× the molotov's trail).
    pub(crate) fire_trail: f32,
    /// Its light (lumens; range m).
    pub(crate) fire_light: f32,
    pub(crate) fire_light_range: f32,

    /// Every dog sound fades to silence this far (m) away.
    pub(crate) sound_max_distance: f32,

    // --- the lightning before one appears ---
    pub(crate) lightning_height: f32,
    pub(crate) lightning_width: f32,
    /// How far (m) each step of the bolt wanders sideways.
    pub(crate) lightning_jitter: f32,
    /// Times a second the bolt re-forks.
    pub(crate) lightning_flicker_hz: f32,
    /// Chance (0..1) the bolt's lit at each re-fork (the rest it's dark).
    pub(crate) lightning_on_chance: f32,
    pub(crate) lightning_brightness: f32,
    pub(crate) lightning_light: f32,
    pub(crate) lightning_light_range: f32,

    // --- the flash it appears in ---
    pub(crate) flash_size: f32,
    pub(crate) flash_secs: f32,
    pub(crate) flash_brightness: f32,
    pub(crate) flash_light: f32,
    pub(crate) flash_light_range: f32,
}

impl Default for DogSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            yaw_offset_deg: 180.0,
            run_per_mps: 0.2,
            min_move_speed: 0.3,

            fire_height: 0.0,
            fire_flames: 12,
            fire_length: 1.35,
            fire_width: 0.27,
            fire_forward: 0.37,
            flame_width: 0.38,
            flame_height: 0.6,
            fire_brightness: 1.05,
            fire_trail: 1.45,
            fire_light: 80_000.0,
            fire_light_range: 5.0,

            sound_max_distance: 50.0,

            lightning_height: 80.0,
            lightning_width: 0.16,
            lightning_jitter: 1.4,
            lightning_flicker_hz: 12.0,
            lightning_on_chance: 0.75,
            lightning_brightness: 30.0,
            lightning_light: 4_000_000.0,
            lightning_light_range: 30.0,

            flash_size: 3.0,
            flash_secs: 0.45,
            flash_brightness: 20.0,
            flash_light: 3_000_000.0,
            flash_light_range: 15.0,
        }
    }
}

/// The nearest dog, for the debug panel: its speed (m/s) and run playback
/// rate.
#[derive(Resource, Default)]
pub(crate) struct DogReadout {
    nearest: Option<(f32, f32)>,
}

/// The debug panel's preview buttons, consumed by [`run_previews`].
#[derive(Resource, Default)]
pub(crate) struct DogPreview {
    lightning: bool,
    flash: bool,
    explosion: bool,
}

/// Shared handles, built once. (`crate::boss` strikes its lightning with
/// them too.)
#[derive(Resource)]
pub(crate) struct DogAssets {
    graph: Handle<AnimationGraph>,
    run: AnimationNodeIndex,
    bolt_mesh: Handle<Mesh>,
    bolt_material: Handle<StandardMaterial>,
    /// A boss's orange bolts.
    boss_bolt_material: Handle<StandardMaterial>,
    flash_mesh: Handle<Mesh>,
}

/// Tags a dog avatar's `SceneRoot`.
#[derive(Component)]
pub(crate) struct DogVisual;

/// On a dog's source (pose) entity once its avatar's gone (it exploded):
/// don't make it another.
#[derive(Component)]
pub(crate) struct DogGone;

/// What's inside a dog's scene, once it's in: its `AnimationPlayer`, and the
/// bone its back fire rides.
#[derive(Component)]
struct DogRig {
    player: Entity,
    back: Option<Entity>,
}

/// A dog avatar's bookkeeping.
#[derive(Component, Default)]
struct DogMotion {
    prev: Option<Vec3>,
    /// Horizontal velocity (m/s), smoothed.
    vel: Vec3,
    /// Its fire's been lit.
    fire: bool,
    /// When its bark starts (staggered, so a pack doesn't bark in unison);
    /// `None` once it has.
    bark_at: Option<f32>,
}

/// A dog's back fire, following its avatar.
#[derive(Component)]
struct DogFire {
    avatar: Entity,
    material: Handle<FireMaterial>,
}

/// One flame of a [`DogFire`]: where along / across the back, its size.
#[derive(Component)]
struct DogFlame {
    along: f32,
    across: f32,
    size: Vec2,
    seed: f32,
}

#[derive(Component)]
struct DogFireLight {
    seed: f32,
}

/// A playing dog sound: its loudness before the distance fade.
#[derive(Component)]
struct DogSound {
    loudness: f32,
}

/// Lightning striking where a dog's about to appear.
#[derive(Component)]
struct Lightning {
    age: f32,
    secs: f32,
    next_fork: f32,
    seq: u32,
}

/// One segment of a [`Lightning`] bolt (main channel first, then branches).
#[derive(Component)]
struct BoltSegment(usize);

#[derive(Component)]
struct LightningLight;

/// The flash a dog appears in.
#[derive(Component)]
struct SpawnFlash {
    age: f32,
    material: Handle<StandardMaterial>,
    color: [f32; 3],
    /// × [`DogSettings::flash_size`].
    size: f32,
}

#[derive(Component)]
struct SpawnFlashLight;

pub(crate) struct DogsPlugin;

impl Plugin for DogsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DogSettings>()
            .init_resource::<DogReadout>()
            .init_resource::<DogPreview>()
            .add_systems(Startup, setup_dog_assets)
            .add_systems(
                Update,
                (
                    receive_dog_messages,
                    run_previews,
                    animate_dogs,
                    light_dog_fires,
                    update_dog_fires,
                    update_lightning,
                    update_flashes,
                    fade_dog_sounds,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn setup_dog_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (graph, run) = AnimationGraph::from_clip(asset_server.load(GltfAssetLabel::Animation(0).from_asset(DOG_MODEL)));
    let settings = DogSettings::default();
    commands.insert_resource(DogAssets {
        graph: graphs.add(graph),
        run,
        bolt_mesh: meshes.add(Cylinder::new(0.5, 1.0)),
        bolt_material: materials.add(bolt_material(STRIKE_COLOR, settings.lightning_brightness)),
        boss_bolt_material: materials.add(bolt_material(BOSS_STRIKE_COLOR, settings.lightning_brightness)),
        flash_mesh: meshes.add(Sphere::new(1.0)),
    });
}

fn strike(color: [f32; 3], brightness: f32) -> LinearRgba {
    LinearRgba::rgb(color[0] * brightness, color[1] * brightness, color[2] * brightness)
}

fn bolt_material(color: [f32; 3], brightness: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: strike(color, brightness).into(),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    }
}

/// Spawn a hellhound avatar following the `PlayerPose` on `src`.
pub(crate) fn spawn_dog_avatar(commands: &mut Commands, asset_server: &AssetServer, settings: &DogSettings, src: Entity) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RemoteAvatar { src },
            DogVisual,
            DogMotion::default(),
            Transform::from_scale(Vec3::splat(settings.scale.max(0.001))),
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(DOG_MODEL))),
        ))
        .observe(start_dog_animation);
}

/// Once a dog's scene is in: run, from the start of the cycle; remember its
/// `AnimationPlayer` and back bone.
fn start_dog_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    names: Query<&Name>,
    mut players: Query<&mut AnimationPlayer>,
    assets: Res<DogAssets>,
) {
    let root = trigger.target();
    let mut player_entity = None;
    let mut back = None;
    for entity in children.iter_descendants(root) {
        // A skinned mesh is culled against its rest pose, not the animated one.
        commands.entity(entity).insert(NoFrustumCulling);
        if names.get(entity).is_ok_and(|n| n.as_str() == BACK_BONE) {
            back = Some(entity);
        }
        if let Ok(mut player) = players.get_mut(entity) {
            player.play(assets.run).repeat().seek_to(RUN_START);
            commands.entity(entity).insert(AnimationGraphHandle(assets.graph.clone()));
            player_entity = Some(entity);
        }
    }
    if let Some(player) = player_entity {
        commands.entity(root).insert(DogRig { player, back });
    }
}

/// The peer a pose entity belongs to (its own `PlayerId`, or its confirmed
/// entity's).
fn peer_of(src: Entity, ids: &Query<&PlayerId>, interp: &Query<&Interpolated>) -> Option<PeerId> {
    ids.get(src)
        .ok()
        .or_else(|| interp.get(src).ok().and_then(|i| ids.get(i.confirmed_entity).ok()))
        .map(|id| id.0)
}

/// A positional dog sound at `at`, silent until [`fade_dog_sounds`] sets its
/// real volume.
fn dog_sound(clip: Handle<AudioSource>, loudness: f32, at: Vec3, looped: bool) -> impl Bundle {
    let mut playback = crate::positional_playback(Volume::Linear(0.0));
    if looped {
        playback.mode = PlaybackMode::Loop;
    }
    (
        StateScoped(AppState::InGame),
        DogSound { loudness },
        // Volume is ours (distance fade), not the one-shot pass's.
        crate::RemoteSoundEmitter,
        AudioPlayer::new(clip),
        Transform::from_translation(at),
        playback,
    )
}

/// Lightning striking at `at` for `secs` — a hellhound's blue-white, or a
/// boss's orange (`boss`).
pub(crate) fn spawn_lightning(
    commands: &mut Commands,
    assets: &DogAssets,
    settings: &DogSettings,
    at: Vec3,
    seq: u32,
    secs: f32,
    boss: bool,
) {
    let c = if boss { BOSS_STRIKE_COLOR } else { STRIKE_COLOR };
    let material = if boss { &assets.boss_bolt_material } else { &assets.bolt_material };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            Lightning {
                age: 0.0,
                secs,
                next_fork: 0.0,
                seq,
            },
            Transform::from_translation(at),
            Visibility::default(),
        ))
        .with_children(|l| {
            for i in 0..BOLT_SEGMENTS + BRANCHES * BRANCH_SEGMENTS {
                l.spawn((
                    BoltSegment(i),
                    Mesh3d(assets.bolt_mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            }
            l.spawn((
                LightningLight,
                PointLight {
                    color: Color::srgb(c[0], c[1], c[2]),
                    intensity: 0.0,
                    range: settings.lightning_light_range,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, 3.0, 0.0),
            ));
        });
}

/// The flash something appears in at `at` — a hellhound's blue-white, or a
/// boss's orange (`boss`, `size` × the usual).
pub(crate) fn spawn_flash(
    commands: &mut Commands,
    assets: &DogAssets,
    settings: &DogSettings,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
    boss: bool,
    size: f32,
) {
    let c = if boss { BOSS_STRIKE_COLOR } else { STRIKE_COLOR };
    let material = materials.add(StandardMaterial {
        base_color: strike(c, settings.flash_brightness).into(),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    commands
        .spawn((
            StateScoped(AppState::InGame),
            SpawnFlash {
                age: 0.0,
                material: material.clone(),
                color: c,
                size,
            },
            Mesh3d(assets.flash_mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(at + Vec3::Y * 0.6).with_scale(Vec3::splat(0.01)),
            NotShadowCaster,
        ))
        .with_child((
            SpawnFlashLight,
            PointLight {
                color: Color::srgb(c[0], c[1], c[2]),
                intensity: settings.flash_light,
                range: settings.flash_light_range,
                shadows_enabled: false,
                ..default()
            },
        ));
}

/// Server → us: lightning before a dog, a dog appearing, a dog exploding.
#[allow(clippy::too_many_arguments)]
fn receive_dog_messages(
    mut lightning: Query<&mut MessageReceiver<shared::DogLightning>>,
    mut spawned: Query<&mut MessageReceiver<shared::DogSpawned>>,
    mut exploded: Query<&mut MessageReceiver<shared::DogExploded>>,
    assets: Option<Res<DogAssets>>,
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    settings: Res<DogSettings>,
    avatars: Query<(Entity, &RemoteAvatar, &Transform), With<DogVisual>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut boom: EventWriter<crate::vfx::Explosion>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let (Some(assets), Some(sounds)) = (assets, sounds) else { return };
    for mut rx in &mut lightning {
        for msg in rx.receive() {
            let at = Vec3::from_array(msg.at);
            *seq = seq.wrapping_add(1);
            spawn_lightning(&mut commands, &assets, &settings, at, *seq, shared::dogs::DOG_PRE_SPAWN_SECS, false);
            commands.spawn(dog_sound(sounds.dog_pre_spawn.clone(), vols.dog_pre_spawn, at + Vec3::Y, false));
        }
    }
    for mut rx in &mut spawned {
        for msg in rx.receive() {
            let at = Vec3::from_array(msg.at);
            spawn_flash(&mut commands, &assets, &settings, &mut materials, at, false, 1.0);
            commands.spawn(dog_sound(sounds.dog_spawn.clone(), vols.dog_spawn, at + Vec3::Y * 0.5, false));
        }
    }
    for mut rx in &mut exploded {
        for msg in rx.receive() {
            // Its body goes, and the blast's where we see it.
            let mut feet = Vec3::from_array(msg.at);
            for (avatar, remote, tf) in &avatars {
                if peer_of(remote.src, &ids, &interp) == Some(msg.dog) {
                    feet = tf.translation;
                    commands.entity(avatar).try_despawn();
                    if let Ok(mut src) = commands.get_entity(remote.src) {
                        src.insert(DogGone);
                    }
                }
            }
            boom.write(crate::vfx::Explosion {
                feet,
                variant: 0,
                phd: false,
                dog: true,
                boss: false,
                frag: false,
            });
        }
    }
}

/// Run each dog's legs at the speed it's really going (looping the one
/// cycle at the end of `run1`), and clear any that's died.
#[allow(clippy::type_complexity)]
fn animate_dogs(
    time: Res<Time>,
    settings: Res<DogSettings>,
    assets: Option<Res<DogAssets>>,
    poses: Query<&PlayerPose>,
    me: Query<&Transform, (With<Player>, Without<DogVisual>)>,
    mut avatars: Query<(Entity, &RemoteAvatar, &mut DogMotion, Option<&DogRig>)>,
    mut players: Query<&mut AnimationPlayer>,
    mut readout: ResMut<DogReadout>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let me = me.single().ok().map(|t| t.translation);
    let mut nearest: Option<(f32, f32, f32)> = None;
    let cycle = RUN_END - RUN_START;

    for (entity, avatar, mut motion, rig) in &mut avatars {
        let Ok(pose) = poses.get(avatar.src) else { continue };
        if !pose.alive {
            // (Its explosion's the server's message; this is just in case.)
            commands.entity(entity).try_despawn();
            if let Ok(mut src) = commands.get_entity(avatar.src) {
                src.insert(DogGone);
            }
            continue;
        }
        let raw = match motion.prev {
            Some(prev) => (pose.translation - prev).with_y(0.0) / dt,
            None => Vec3::ZERO,
        };
        motion.prev = Some(pose.translation);
        let k = 1.0 - (-dt / 0.12).exp();
        motion.vel = motion.vel.lerp(raw, k);
        let speed = motion.vel.length();
        let rate = if speed < settings.min_move_speed {
            0.0
        } else {
            speed * settings.run_per_mps
        };

        if let Some(mut player) = rig.and_then(|r| players.get_mut(r.player).ok()) {
            if let Some(active) = player.animation_mut(assets.run) {
                active.set_speed(rate);
                // Keep the playhead inside the cycle — and step back a cycle
                // before it runs off the end of the clip, so it never wraps
                // to the empty start.
                let t = active.seek_time();
                if !(RUN_START..=RUN_END).contains(&t) {
                    active.seek_to(RUN_START + (t - RUN_START).rem_euclid(cycle));
                } else if t + rate * dt * 1.5 >= RUN_END {
                    active.seek_to(t - cycle);
                }
            }
        }

        if let Some(me) = me {
            let d = me.distance(pose.translation);
            if nearest.is_none_or(|(n, ..)| d < n) {
                nearest = Some((d, speed, rate));
            }
        }
    }
    readout.nearest = nearest.map(|(_, s, r)| (s, r));
}

/// A new dog: light the fire on its back, and (a moment later) start its bark.
fn light_dog_fires(
    time: Res<Time>,
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    settings: Res<DogSettings>,
    molotov: Res<MolotovSettings>,
    molotov_assets: Option<Res<MolotovAssets>>,
    mut avatars: Query<(Entity, &mut DogMotion), With<DogVisual>>,
    mut fire_materials: ResMut<Assets<FireMaterial>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let (Some(sounds), Some(molotov_assets)) = (sounds, molotov_assets) else { return };
    let now = time.elapsed_secs();
    for (avatar, mut motion) in &mut avatars {
        if !motion.fire {
            motion.fire = true;
            *seq = seq.wrapping_add(1);
            let key = seq.wrapping_mul(2_654_435_761);
            let hash = |i: u32, k: u32| rand01(key ^ i.wrapping_mul(7919) ^ k.wrapping_mul(0x9E37_79B9));
            motion.bark_at = Some(now + 0.3 + 2.0 * hash(0, 9));
            let material = fire_materials.add(FireMaterial::flame(&molotov, settings.fire_brightness, Vec3::ZERO));
            let flames = settings.fire_flames.max(1);
            commands
                .spawn((
                    StateScoped(AppState::InGame),
                    DogFire {
                        avatar,
                        material: material.clone(),
                    },
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .with_children(|fire| {
                    for i in 0..flames {
                        let k = 0.75 + 0.25 * hash(i, 3);
                        fire.spawn((
                            DogFlame {
                                along: ((i as f32 + 0.25 + 0.5 * hash(i, 1)) / flames as f32 - 0.5),
                                across: hash(i, 2) - 0.5,
                                size: Vec2::new(k, (0.7 + 0.3 * hash(i, 4)) * k),
                                seed: hash(i, 5),
                            },
                            Mesh3d(molotov_assets.quad.clone()),
                            MeshMaterial3d(material.clone()),
                            Transform::from_scale(Vec3::splat(0.001)),
                            NotShadowCaster,
                            NoFrustumCulling,
                        ));
                    }
                    fire.spawn((
                        DogFireLight { seed: hash(0, 6) * 10.0 },
                        PointLight {
                            color: molotov.light_color(),
                            intensity: 0.0,
                            range: settings.fire_light_range,
                            shadows_enabled: false,
                            ..default()
                        },
                        Transform::from_xyz(0.0, 0.3, 0.0),
                    ));
                });
        }
        if motion.bark_at.is_some_and(|at| now >= at) {
            motion.bark_at = None;
            let bark = commands
                .spawn(dog_sound(sounds.dog_bark.clone(), vols.dog_bark, Vec3::ZERO, true))
                .id();
            commands.entity(avatar).add_child(bark);
        }
    }
}

/// Keep each back fire on its dog's back (riding the spine as it runs),
/// streaming back the faster it goes; put it out when the dog's gone.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_dog_fires(
    time: Res<Time>,
    settings: Res<DogSettings>,
    molotov: Res<MolotovSettings>,
    avatars: Query<(&GlobalTransform, &DogMotion, Option<&DogRig>), With<DogVisual>>,
    bones: Query<&GlobalTransform, Without<DogVisual>>,
    mut fires: Query<(Entity, &DogFire, &mut Transform, &Children, &mut Visibility)>,
    mut flames: Query<(&DogFlame, &mut Transform), Without<DogFire>>,
    mut lights: Query<(&DogFireLight, &mut PointLight)>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut commands: Commands,
) {
    let t = time.elapsed_secs();
    for (entity, fire, mut tf, children, mut vis) in &mut fires {
        let Ok((avatar_gt, motion, rig)) = avatars.get(fire.avatar) else {
            materials.remove(&fire.material);
            commands.entity(entity).try_despawn();
            continue;
        };
        let rot = avatar_gt.rotation();
        // The model faces +Z.
        let along = rot * Vec3::Z;
        let across = rot * Vec3::X;
        let back = rig
            .and_then(|r| r.back)
            .and_then(|b| bones.get(b).ok())
            .map(|b| b.translation())
            .unwrap_or_else(|| avatar_gt.translation() + Vec3::Y * settings.scale);
        tf.translation = back + Vec3::Y * settings.fire_height + along * settings.fire_forward;
        vis.set_if_neq(Visibility::Inherited);
        if let Some(m) = materials.get_mut(&fire.material) {
            *m = FireMaterial::flame(
                &molotov,
                settings.fire_brightness,
                molotov.lean(motion.vel) * settings.fire_trail,
            );
        }
        for child in children.iter() {
            if let Ok((flame, mut ftf)) = flames.get_mut(child) {
                ftf.translation =
                    along * flame.along * settings.fire_length + across * flame.across * settings.fire_width;
                ftf.scale = seeded_scale(
                    Vec2::new(flame.size.x * settings.flame_width, flame.size.y * settings.flame_height),
                    flame.seed,
                );
            } else if let Ok((fl, mut light)) = lights.get_mut(child) {
                light.intensity = settings.fire_light * flicker(t, fl.seed, molotov.flicker);
                light.range = settings.fire_light_range;
            }
        }
    }
}

/// Fork and flicker each lightning bolt, and take it down once the dog's due.
#[allow(clippy::type_complexity)]
fn update_lightning(
    time: Res<Time>,
    settings: Res<DogSettings>,
    assets: Option<Res<DogAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bolts: Query<(Entity, &mut Lightning, &Children)>,
    mut segments: Query<(&BoltSegment, &mut Transform), Without<Lightning>>,
    mut lights: Query<&mut PointLight, With<LightningLight>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    if let Some(assets) = &assets {
        if settings.is_changed() {
            if let Some(m) = materials.get_mut(&assets.bolt_material) {
                m.base_color = strike(STRIKE_COLOR, settings.lightning_brightness).into();
            }
            if let Some(m) = materials.get_mut(&assets.boss_bolt_material) {
                m.base_color = strike(BOSS_STRIKE_COLOR, settings.lightning_brightness).into();
            }
        }
    }
    let s = &*settings;
    for (entity, mut bolt, children) in &mut bolts {
        bolt.age += dt;
        if bolt.age >= bolt.secs {
            commands.entity(entity).try_despawn();
            continue;
        }
        if bolt.age < bolt.next_fork {
            continue;
        }
        bolt.seq = bolt.seq.wrapping_add(1);
        let base = bolt.seq.wrapping_mul(2_654_435_761);
        let roll = |k: u32| rand01(base ^ k.wrapping_mul(0x9E37_79B9));
        bolt.next_fork = bolt.age + (0.5 + roll(1)) / s.lightning_flicker_hz.max(0.5);
        // Lit, or a dark beat between flashes (always lit at the very end,
        // as the dog lands).
        let lit = roll(2) < s.lightning_on_chance || bolt.secs - bolt.age < 0.25;

        // The main channel: from the ground straight up, wandering a little
        // each step — more toward the sky.
        let step = s.lightning_height / BOLT_SEGMENTS as f32;
        let mut points = vec![Vec3::ZERO];
        for i in 1..=BOLT_SEGMENTS {
            let prev = points[i - 1];
            let wander = s.lightning_jitter * (0.4 + 0.6 * i as f32 / BOLT_SEGMENTS as f32);
            let off = Vec3::new(roll(10 + i as u32) - 0.5, 0.0, roll(40 + i as u32) - 0.5) * 2.0 * wander;
            points.push(Vec3::new(prev.x * 0.85, step * i as f32, prev.z * 0.85) + off);
        }
        // Branches off it, forking outward and up.
        let mut branches: Vec<Vec<Vec3>> = Vec::new();
        for b in 0..BRANCHES {
            let from = 3 + (roll(80 + b as u32) * (BOLT_SEGMENTS - 6) as f32) as usize;
            let a = roll(90 + b as u32) * std::f32::consts::TAU;
            let out = Vec3::new(a.cos(), 0.0, a.sin());
            let mut p = vec![points[from]];
            for j in 1..=BRANCH_SEGMENTS {
                let prev = p[j - 1];
                let jit = Vec3::new(roll(100 + (b * 10 + j) as u32) - 0.5, 0.0, roll(200 + (b * 10 + j) as u32) - 0.5)
                    * s.lightning_jitter;
                p.push(prev + out * step * 0.6 + Vec3::Y * step * 0.5 + jit);
            }
            branches.push(p);
        }
        let segment = |a: Vec3, b: Vec3, width: f32| {
            let d = b - a;
            let len = d.length().max(1e-4);
            Transform {
                translation: (a + b) * 0.5,
                rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
                scale: Vec3::new(width, len, width),
            }
        };
        for child in children.iter() {
            if let Ok((seg, mut tf)) = segments.get_mut(child) {
                let i = seg.0;
                *tf = if !lit {
                    Transform::from_scale(Vec3::ZERO)
                } else if i < BOLT_SEGMENTS {
                    segment(points[i], points[i + 1], s.lightning_width)
                } else {
                    let b = (i - BOLT_SEGMENTS) / BRANCH_SEGMENTS;
                    let j = (i - BOLT_SEGMENTS) % BRANCH_SEGMENTS;
                    // (Some forks are dark this beat.)
                    if roll(300 + b as u32) < 0.35 {
                        Transform::from_scale(Vec3::ZERO)
                    } else {
                        segment(branches[b][j], branches[b][j + 1], s.lightning_width * 0.5)
                    }
                };
            } else if let Ok(mut light) = lights.get_mut(child) {
                light.intensity = if lit {
                    s.lightning_light * (0.6 + 0.4 * roll(3))
                } else {
                    s.lightning_light * 0.05
                };
                light.range = s.lightning_light_range;
            }
        }
    }
}

/// Swell and fade each spawn flash.
fn update_flashes(
    time: Res<Time>,
    settings: Res<DogSettings>,
    mut flashes: Query<(Entity, &mut SpawnFlash, &mut Transform, &Children)>,
    mut lights: Query<&mut PointLight, With<SpawnFlashLight>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let s = &*settings;
    for (entity, mut flash, mut tf, children) in &mut flashes {
        flash.age += time.delta_secs();
        let k = flash.age / s.flash_secs.max(0.01);
        if k >= 1.0 {
            materials.remove(&flash.material);
            commands.entity(entity).try_despawn();
            continue;
        }
        let fade = (1.0 - k) * (1.0 - k);
        tf.scale = Vec3::splat(s.flash_size * flash.size * (0.3 + 0.7 * k.sqrt()) * 0.5);
        if let Some(m) = materials.get_mut(&flash.material) {
            m.base_color = strike(flash.color, s.flash_brightness * fade).into();
        }
        for child in children.iter() {
            if let Ok(mut light) = lights.get_mut(child) {
                light.intensity = s.flash_light * fade;
                light.range = s.flash_light_range;
            }
        }
    }
}

/// Keep every dog sound's loudness matched to how far the listener is from
/// it — a squared fade to silence at `sound_max_distance`.
fn fade_dog_sounds(
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    settings: Res<DogSettings>,
    global_volume: Res<GlobalVolume>,
    mut playing: Query<(&GlobalTransform, &DogSound, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else { return };
    let ear = ear.translation();
    let max = settings.sound_max_distance.max(1.0);
    for (gt, sound, mut sink) in &mut playing {
        let fade = (1.0 - ear.distance(gt.translation()) / max).clamp(0.0, 1.0).powi(2);
        sink.set_volume(Volume::Linear((sound.loudness * fade).max(0.0)) * global_volume.volume);
    }
}

/// The debug panel's previews, 8 m in front of us on our ground.
#[allow(clippy::too_many_arguments)]
fn run_previews(
    mut preview: ResMut<DogPreview>,
    assets: Option<Res<DogAssets>>,
    settings: Res<DogSettings>,
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    player: Single<&Transform, With<Player>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut boom: EventWriter<crate::vfx::Explosion>,
    mut commands: Commands,
) {
    if !(preview.lightning || preview.flash || preview.explosion) {
        return;
    }
    let (Some(assets), Some(sounds)) = (assets, sounds) else { return };
    let fwd = (player.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let at = player.translation - Vec3::Y * crate::EYE_HEIGHT + fwd * 8.0;
    if std::mem::take(&mut preview.lightning) {
        spawn_lightning(&mut commands, &assets, &settings, at, 0, shared::dogs::DOG_PRE_SPAWN_SECS, false);
        commands.spawn(dog_sound(sounds.dog_pre_spawn.clone(), vols.dog_pre_spawn, at + Vec3::Y, false));
    }
    if std::mem::take(&mut preview.flash) {
        spawn_flash(&mut commands, &assets, &settings, &mut materials, at, false, 1.0);
        commands.spawn(dog_sound(sounds.dog_spawn.clone(), vols.dog_spawn, at + Vec3::Y * 0.5, false));
    }
    if std::mem::take(&mut preview.explosion) {
        boom.write(crate::vfx::Explosion {
            feet: at,
            variant: 0,
            phd: false,
            dog: true,
            boss: false,
            frag: false,
        });
    }
}

/// The debug panel's "Dogs (Zombies)" section (`debug_ui`).
pub(crate) fn dogs_section(ui: &mut egui::Ui, s: &mut DogSettings, preview: &mut DogPreview, readout: &DogReadout) {
    match readout.nearest {
        Some((speed, rate)) => ui.label(format!("nearest dog: {speed:.2} m/s → run at {rate:.2}×")),
        None => ui.label("no dogs right now (they come every 5th round)"),
    };
    ui.horizontal(|ui| {
        if ui.button("Preview lightning").clicked() {
            preview.lightning = true;
        }
        if ui.button("Preview spawn flash").clicked() {
            preview.flash = true;
        }
        if ui.button("Preview explosion").clicked() {
            preview.explosion = true;
        }
    });
    ui.collapsing("Model + run", |ui| {
        ui.add(egui::Slider::new(&mut s.scale, 0.1f32..=2.0).text("scale"));
        ui.add(egui::Slider::new(&mut s.yaw_offset_deg, -180.0f32..=180.0).text("turn (°)"));
        ui.add(
            egui::Slider::new(&mut s.run_per_mps, 0.0f32..=2.0)
                .text("run speed per m/s (feet planted at any speed)"),
        );
        ui.add(egui::Slider::new(&mut s.min_move_speed, 0.0f32..=3.0).text("still below (m/s)"));
    });
    ui.collapsing("Back fire", |ui| {
        ui.add(egui::Slider::new(&mut s.fire_height, -0.5f32..=1.0).text("above the back (m)"));
        ui.add(egui::Slider::new(&mut s.fire_forward, -1.0f32..=1.0).text("forward (m)"));
        ui.add(egui::Slider::new(&mut s.fire_flames, 1u32..=16).text("flames (new dogs)"));
        ui.add(egui::Slider::new(&mut s.fire_length, 0.0f32..=2.0).text("along the back (m)"));
        ui.add(egui::Slider::new(&mut s.fire_width, 0.0f32..=1.0).text("across (m)"));
        ui.add(egui::Slider::new(&mut s.flame_width, 0.05f32..=2.0).text("flame width (m)"));
        ui.add(egui::Slider::new(&mut s.flame_height, 0.05f32..=2.0).text("flame height (m)"));
        ui.add(egui::Slider::new(&mut s.fire_brightness, 0.0f32..=3.0).text("brightness"));
        ui.add(egui::Slider::new(&mut s.fire_trail, 0.0f32..=3.0).text("streams back"));
        ui.add(
            egui::Slider::new(&mut s.fire_light, 0.0f32..=1_000_000.0)
                .logarithmic(true)
                .text("light (lm)"),
        );
        ui.add(egui::Slider::new(&mut s.fire_light_range, 0.5f32..=20.0).text("light range (m)"));
    });
    ui.collapsing("Lightning", |ui| {
        ui.add(egui::Slider::new(&mut s.lightning_height, 5.0f32..=200.0).text("height (m)"));
        ui.add(egui::Slider::new(&mut s.lightning_width, 0.01f32..=1.0).text("width (m)"));
        ui.add(egui::Slider::new(&mut s.lightning_jitter, 0.0f32..=5.0).text("jaggedness (m)"));
        ui.add(egui::Slider::new(&mut s.lightning_flicker_hz, 1.0f32..=40.0).text("re-forks / s"));
        ui.add(egui::Slider::new(&mut s.lightning_on_chance, 0.0f32..=1.0).text("lit share"));
        ui.add(egui::Slider::new(&mut s.lightning_brightness, 0.0f32..=100.0).text("brightness"));
        ui.add(
            egui::Slider::new(&mut s.lightning_light, 0.0f32..=40_000_000.0)
                .logarithmic(true)
                .text("light (lm)"),
        );
        ui.add(egui::Slider::new(&mut s.lightning_light_range, 1.0f32..=100.0).text("light range (m)"));
    });
    ui.collapsing("Spawn flash", |ui| {
        ui.add(egui::Slider::new(&mut s.flash_size, 0.1f32..=10.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.flash_secs, 0.05f32..=2.0).text("time (s)"));
        ui.add(egui::Slider::new(&mut s.flash_brightness, 0.0f32..=100.0).text("brightness"));
        ui.add(
            egui::Slider::new(&mut s.flash_light, 0.0f32..=40_000_000.0)
                .logarithmic(true)
                .text("light (lm)"),
        );
        ui.add(egui::Slider::new(&mut s.flash_light_range, 1.0f32..=60.0).text("light range (m)"));
    });
    ui.add(egui::Slider::new(&mut s.sound_max_distance, 5.0f32..=150.0).text("sounds heard within (m)"));
    ui.label("Explosion size: \"Zombies perks\" → \"Bomb Shot\" → hellhound's size. Volumes: Sound volumes.");
    ui.horizontal(|ui| {
        if ui.button("Copy dog settings to console").clicked() {
            info!(
                "dogs: scale {:.3}, yaw_offset_deg {:.1}, run_per_mps {:.3}, min_move_speed {:.2}, \
                 fire_height {:.2}, fire_forward {:.2}, fire_flames {}, fire_length {:.2}, fire_width {:.2}, \
                 flame_width {:.2}, flame_height {:.2}, fire_brightness {:.2}, fire_trail {:.2}, \
                 fire_light {:.0}, fire_light_range {:.1}, lightning_height {:.1}, lightning_width {:.3}, \
                 lightning_jitter {:.2}, lightning_flicker_hz {:.1}, lightning_on_chance {:.2}, \
                 lightning_brightness {:.1}, lightning_light {:.0}, lightning_light_range {:.1}, \
                 flash_size {:.2}, flash_secs {:.2}, flash_brightness {:.1}, flash_light {:.0}, \
                 flash_light_range {:.1}, sound_max_distance {:.1}",
                s.scale, s.yaw_offset_deg, s.run_per_mps, s.min_move_speed, s.fire_height, s.fire_forward,
                s.fire_flames, s.fire_length, s.fire_width, s.flame_width, s.flame_height, s.fire_brightness,
                s.fire_trail, s.fire_light, s.fire_light_range, s.lightning_height, s.lightning_width,
                s.lightning_jitter, s.lightning_flicker_hz, s.lightning_on_chance, s.lightning_brightness,
                s.lightning_light, s.lightning_light_range, s.flash_size, s.flash_secs, s.flash_brightness,
                s.flash_light, s.flash_light_range, s.sound_max_distance,
            );
        }
        if ui.button("Reset").clicked() {
            *s = DogSettings::default();
        }
    });
}
