//! `Zombies` bosses (`shared::boss`), on this client. For each one:
//!
//! * **Coming** — orange lightning strikes where it's about to appear, with
//!   its spawn sound, for as long as that sound plays
//!   ([`shared::BossLightning`], [`shared::boss::BOSS_PRE_SPAWN_SECS`]);
//!   then it's there, in an orange flash ([`shared::BossSpawned`]). (The
//!   lightning and flash are the hellhounds', `dogs`.)
//! * **About** — `models/characters/boss.glb`: `idle` standing, `walk` (at
//!   whatever speed keeps its feet planted) on the move, `attack` for both
//!   its smash and its blast, `death` once it's killed — picked from its
//!   replicated pose ([`shared::ZombieAnim`]'s `Boss*` states). It growls
//!   every so often, and its smash and death have their own sounds.
//! * **Blasts** — winding up a blast, a ball of fiery energy swells in its
//!   hands overhead ([`BossOrb`]: a white-hot core in layers of orange glow,
//!   licking flames, orbiting wisps, crackling arcs, sparks drawn in, its own
//!   light); hurled ([`shared::BossBlastLaunched`]), it flies straight on,
//!   roaring (the blast sound, from the ball), shedding embers and smoke,
//!   until it blows up ([`shared::BossBlastExploded`]) — the Bomb Shot
//!   fireball at the boss's size (`vfx::Explosion::boss`), with the blast
//!   hit sound.
//!
//! Every sound is positional and heard by the whole lobby. Look and feel
//! are tuned in the "Boss (Zombies)" debug section ([`BossSettings`]).
//! Everything here is `StateScoped(InGame)`, and a ball whose boss or
//! message never comes is cleared on its own.

use std::time::Duration;

use bevy::animation::RepeatAnimation;
use bevy::audio::{PlaybackMode, SpatialAudioSink, Volume};
use bevy::pbr::NotShadowCaster;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use bevy_egui::egui;
use lightyear::prelude::{Interpolated, LocalId, MessageReceiver, PeerId, TriggerSender};
use shared::boss::{BLAST_CHARGE_START_SECS, BLAST_LIFETIME_SECS, BLAST_RELEASE_SECS, BOSS_HEIGHT};
use shared::{PlayerId, PlayerPose, ZombieAnim};

use crate::dogs::{DogAssets, DogSettings};
use crate::net::{GameClient, RemoteAvatar};
use crate::util::rand01;
use crate::vfx::{smoke_billboard_rotation, ExplosionAssets};
use crate::{AppState, GameSounds, SoundVolumes, WorldModelCamera};

pub(crate) const BOSS_MODEL: &str = "models/characters/boss.glb";
/// `boss.glb` as made: how tall it stands (model units), and how fast (per
/// model unit of size, m/s) `walk` at 1× moves its planted foot.
const MODEL_HEIGHT: f32 = 3.346;
const WALK_MPS_AT_1X: f32 = 1.04;
/// Its hands' bones — the ball forms between them.
const HAND_BONES: [&str; 2] = ["Bip01_L_Hand_020", "Bip01_R_Hand_039"];
/// Most ball particles (embers, sparks, smoke) alive at once.
const MAX_PARTICLES: usize = 500;
/// A ball still waiting on its throw this long (s) past when it should've
/// gone — or flying this long past its lifetime — is cleared.
const ORB_GRACE_SECS: f32 = 1.5;

/// Panel-tunable bosses ("Boss (Zombies)" debug section).
#[derive(Resource, Clone)]
pub(crate) struct BossSettings {
    pub(crate) scale: f32,
    /// Extra turn (degrees) so the model faces the way it walks.
    pub(crate) yaw_offset_deg: f32,
    /// `walk` playback per m/s it's really moving (feet planted at any speed).
    pub(crate) walk_per_mps: f32,
    /// Below this speed (m/s) it stands rather than walking on the spot.
    pub(crate) min_move_speed: f32,
    /// Seconds each clip blends into the next.
    pub(crate) blend_secs: f32,
    /// Seconds between growls (a random gap in this range).
    pub(crate) growl_every: (f32, f32),
    /// Every boss sound fades to silence this far (m) away.
    pub(crate) sound_max_distance: f32,
    /// Its spawn flash's size (× the hellhound's).
    pub(crate) flash_size: f32,

    // --- the energy ball ---
    /// Seconds it takes to swell in from nothing as it's charged.
    pub(crate) charge_fade_secs: f32,
    /// Sizes (m): the white-hot core, its inner glow, the outer halo.
    pub(crate) core_size: f32,
    pub(crate) glow_size: f32,
    pub(crate) halo_size: f32,
    /// Linear colours: the core's, and the fire around it.
    pub(crate) core_color: [f32; 3],
    pub(crate) fire_color: [f32; 3],
    pub(crate) brightness: f32,
    /// Flames licking round it: how many, how big (m), how fast they turn.
    pub(crate) flames: u32,
    pub(crate) flame_size: f32,
    pub(crate) flame_spin: f32,
    /// Wisps orbiting it: how many, how far out (m), how fast (rev/s).
    pub(crate) wisps: u32,
    pub(crate) wisp_orbit: f32,
    pub(crate) wisp_speed: f32,
    /// Crackling arcs off it: how many, how long (m), re-forks a second.
    pub(crate) arcs: u32,
    pub(crate) arc_length: f32,
    pub(crate) arc_hz: f32,
    /// Its light (lumens; range m).
    pub(crate) light: f32,
    pub(crate) light_range: f32,
    /// Sparks drawn into it while it charges (a second).
    pub(crate) charge_sparks: f32,
    /// In flight: embers (a second), how long they last (s), their size (m),
    /// how fast they scatter (m/s); and smoke puffs a second.
    pub(crate) trail_rate: f32,
    pub(crate) trail_life: f32,
    pub(crate) trail_size: f32,
    pub(crate) trail_spread: f32,
    pub(crate) smoke_rate: f32,
}

impl Default for BossSettings {
    fn default() -> Self {
        let scale = BOSS_HEIGHT / MODEL_HEIGHT;
        Self {
            scale,
            yaw_offset_deg: 180.0,
            walk_per_mps: 1.0 / (WALK_MPS_AT_1X * scale),
            min_move_speed: 0.25,
            blend_secs: 0.2,
            growl_every: (6.0, 13.0),
            sound_max_distance: 70.0,
            flash_size: 1.8,

            charge_fade_secs: 0.3,
            core_size: 0.32,
            glow_size: 1.1,
            halo_size: 2.6,
            core_color: [1.0, 0.85, 0.55],
            fire_color: [1.0, 0.38, 0.06],
            brightness: 14.0,
            flames: 8,
            flame_size: 0.75,
            flame_spin: 2.5,
            wisps: 10,
            wisp_orbit: 0.55,
            wisp_speed: 1.6,
            arcs: 5,
            arc_length: 0.75,
            arc_hz: 18.0,
            light: 900_000.0,
            light_range: 14.0,
            charge_sparks: 45.0,
            trail_rate: 90.0,
            trail_life: 0.45,
            trail_size: 0.38,
            trail_spread: 1.2,
            smoke_rate: 18.0,
        }
    }
}

impl BossSettings {
    /// These settings as Rust, to paste over [`Default`].
    fn to_rust(&self) -> String {
        format!("{:#?}", DebugSettings(self))
    }
}

/// Prints [`BossSettings`] as a struct literal.
struct DebugSettings<'a>(&'a BossSettings);

impl std::fmt::Debug for DebugSettings<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.0;
        f.debug_struct("BossSettings")
            .field("scale", &s.scale)
            .field("yaw_offset_deg", &s.yaw_offset_deg)
            .field("walk_per_mps", &s.walk_per_mps)
            .field("min_move_speed", &s.min_move_speed)
            .field("blend_secs", &s.blend_secs)
            .field("growl_every", &s.growl_every)
            .field("sound_max_distance", &s.sound_max_distance)
            .field("flash_size", &s.flash_size)
            .field("charge_fade_secs", &s.charge_fade_secs)
            .field("core_size", &s.core_size)
            .field("glow_size", &s.glow_size)
            .field("halo_size", &s.halo_size)
            .field("core_color", &s.core_color)
            .field("fire_color", &s.fire_color)
            .field("brightness", &s.brightness)
            .field("flames", &s.flames)
            .field("flame_size", &s.flame_size)
            .field("flame_spin", &s.flame_spin)
            .field("wisps", &s.wisps)
            .field("wisp_orbit", &s.wisp_orbit)
            .field("wisp_speed", &s.wisp_speed)
            .field("arcs", &s.arcs)
            .field("arc_length", &s.arc_length)
            .field("arc_hz", &s.arc_hz)
            .field("light", &s.light)
            .field("light_range", &s.light_range)
            .field("charge_sparks", &s.charge_sparks)
            .field("trail_rate", &s.trail_rate)
            .field("trail_life", &s.trail_life)
            .field("trail_size", &s.trail_size)
            .field("trail_spread", &s.trail_spread)
            .field("smoke_rate", &s.smoke_rate)
            .finish()
    }
}

/// The debug section's preview buttons, consumed by [`run_previews`].
#[derive(Resource, Default)]
struct BossPreview {
    lightning: bool,
    orb: bool,
}

/// Shared handles, built once.
#[derive(Resource)]
struct BossAssets {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    attack: AnimationNodeIndex,
    death: AnimationNodeIndex,
    sphere: Handle<Mesh>,
    arc: Handle<Mesh>,
}

/// Tags a boss avatar's `SceneRoot`.
#[derive(Component)]
pub(crate) struct BossVisual;

/// What's inside a boss's scene, once it's in: its `AnimationPlayer`, and
/// its hands.
#[derive(Component)]
struct BossRig {
    player: Entity,
    hands: [Option<Entity>; 2],
}

/// What a boss's model is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum BossState {
    #[default]
    Idle,
    Walk,
    Smash,
    Blast,
    Dead,
}

/// A boss avatar's bookkeeping.
#[derive(Component, Default)]
struct BossMotion {
    prev: Option<Vec3>,
    /// Horizontal velocity (m/s), smoothed.
    vel: Vec3,
    state: BossState,
    /// When it next growls (`Time::elapsed_secs`).
    next_growl: Option<f32>,
}

/// A playing boss sound: its loudness before the distance fade.
#[derive(Component)]
struct BossSound {
    loudness: f32,
}

/// A ball of energy: forming in a boss's hands, or flying.
#[derive(Component)]
struct BossOrb {
    /// The boss avatar winding it up, and its peer (the launch message's).
    avatar: Option<Entity>,
    peer: Option<PeerId>,
    /// Seconds since it started forming.
    age: f32,
    /// How far it's swelled in (0..=1).
    charge: f32,
    /// Hurled: the server's blast id and where it's going.
    flight: Option<Flight>,
    /// Where it's drawn this frame, and the gap between where it was in the
    /// hands and its flight path, eased away over the first moment.
    pos: Vec3,
    offset: Vec3,
    /// Embers / smoke / sparks owed.
    owed: [f32; 3],
    seed: u32,
    next_arc: f32,
    materials: OrbMaterials,
}

struct Flight {
    id: u32,
    dir: Vec3,
    speed: f32,
    age: f32,
}

/// One ball's own materials (so it fades and flickers on its own).
#[derive(Clone)]
struct OrbMaterials {
    core: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    halo: Handle<StandardMaterial>,
    flame: Handle<StandardMaterial>,
    wisp: Handle<StandardMaterial>,
    arc: Handle<StandardMaterial>,
}

/// A part of a [`BossOrb`].
#[derive(Component, Clone, Copy)]
enum OrbPart {
    Core,
    Glow,
    Halo,
    Flame(u32),
    Wisp(u32),
    Arc(u32, u32),
    Light,
}

/// A spark, ember or puff of smoke off a ball.
#[derive(Component)]
struct OrbParticle {
    vel: Vec3,
    age: f32,
    life: f32,
    size: (f32, f32),
    /// Drawn toward this point (a charging ball's sparks), if any.
    into: Option<Vec3>,
    roll: f32,
    material: Handle<StandardMaterial>,
    brightness: f32,
    color: [f32; 3],
    smoke: bool,
}

pub(crate) struct BossPlugin;

impl Plugin for BossPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BossSettings>()
            .init_resource::<BossPreview>()
            .add_systems(Startup, setup_boss_assets)
            .add_systems(
                Update,
                (
                    receive_boss_messages,
                    run_previews,
                    animate_bosses,
                    update_orbs,
                    update_orb_parts,
                    update_orb_particles,
                    fade_boss_sounds,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn setup_boss_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // (`boss.glb`'s clips, in order: idle, walk, attack, death.)
    let (graph, nodes) = AnimationGraph::from_clips(
        (0..4).map(|i| asset_server.load::<AnimationClip>(GltfAssetLabel::Animation(i).from_asset(BOSS_MODEL))),
    );
    commands.insert_resource(BossAssets {
        graph: graphs.add(graph),
        idle: nodes[0],
        walk: nodes[1],
        attack: nodes[2],
        death: nodes[3],
        sphere: meshes.add(Sphere::new(0.5).mesh().ico(3).unwrap()),
        arc: meshes.add(Cylinder::new(0.5, 1.0)),
    });
}

/// Spawn a boss avatar following the `PlayerPose` on `src`.
pub(crate) fn spawn_boss_avatar(commands: &mut Commands, asset_server: &AssetServer, settings: &BossSettings, src: Entity) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RemoteAvatar { src },
            BossVisual,
            BossMotion::default(),
            Transform::from_scale(Vec3::splat(settings.scale.max(0.001))),
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(BOSS_MODEL))),
        ))
        .observe(start_boss_animation);
}

/// Once a boss's scene is in: idle; remember its `AnimationPlayer` and hands.
fn start_boss_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    names: Query<&Name>,
    mut players: Query<&mut AnimationPlayer>,
    assets: Res<BossAssets>,
) {
    let root = trigger.target();
    let mut player_entity = None;
    let mut hands = [None; 2];
    for entity in children.iter_descendants(root) {
        // A skinned mesh is culled against its rest pose, not the animated one.
        commands.entity(entity).insert(NoFrustumCulling);
        if let Ok(name) = names.get(entity) {
            for (i, bone) in HAND_BONES.iter().enumerate() {
                if name.as_str() == *bone {
                    hands[i] = Some(entity);
                }
            }
        }
        if let Ok(mut player) = players.get_mut(entity) {
            let mut transitions = AnimationTransitions::new();
            transitions
                .play(&mut player, assets.idle, Duration::ZERO)
                .set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert((AnimationGraphHandle(assets.graph.clone()), transitions));
            player_entity = Some(entity);
        }
    }
    if let Some(player) = player_entity {
        commands.entity(root).insert(BossRig { player, hands });
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

/// A positional boss sound at `at`, silent until [`fade_boss_sounds`] sets
/// its real volume.
fn boss_sound(clip: Handle<AudioSource>, loudness: f32, at: Vec3, looped: bool) -> impl Bundle {
    let mut playback = crate::positional_playback(Volume::Linear(0.0));
    if looped {
        playback.mode = PlaybackMode::Loop;
    }
    (
        StateScoped(AppState::InGame),
        BossSound { loudness },
        // Volume is ours (distance fade), not the one-shot pass's.
        crate::RemoteSoundEmitter,
        AudioPlayer::new(clip),
        Transform::from_translation(at),
        playback,
    )
}

/// An additive, unlit, glowing material of `color` × `brightness`, wearing
/// `texture` if any.
fn glow_material(color: [f32; 3], brightness: f32, texture: Option<Handle<Image>>) -> StandardMaterial {
    StandardMaterial {
        base_color: LinearRgba::rgb(color[0] * brightness, color[1] * brightness, color[2] * brightness).into(),
        base_color_texture: texture,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// A new ball, forming (or, with `flight`, already hurled) at `pos`.
#[allow(clippy::too_many_arguments)]
fn spawn_orb(
    commands: &mut Commands,
    assets: &BossAssets,
    fx: &ExplosionAssets,
    settings: &BossSettings,
    materials: &mut Assets<StandardMaterial>,
    pos: Vec3,
    avatar: Option<Entity>,
    peer: Option<PeerId>,
    flight: Option<Flight>,
    seed: u32,
) -> Entity {
    let s = settings;
    let mats = OrbMaterials {
        core: materials.add(glow_material(s.core_color, s.brightness * 1.5, None)),
        glow: materials.add(glow_material(s.core_color, s.brightness * 0.35, Some(fx.glow.clone()))),
        halo: materials.add(glow_material(s.fire_color, s.brightness * 0.12, Some(fx.glow.clone()))),
        flame: materials.add(glow_material(s.fire_color, s.brightness * 0.22, Some(fx.fire[0].clone()))),
        wisp: materials.add(glow_material(s.fire_color, s.brightness * 0.5, Some(fx.glow.clone()))),
        arc: materials.add(glow_material(s.core_color, s.brightness * 0.8, None)),
    };
    let charged = flight.is_some();
    let c = s.fire_color;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            BossOrb {
                avatar,
                peer,
                age: 0.0,
                charge: if charged { 1.0 } else { 0.0 },
                flight,
                pos,
                offset: Vec3::ZERO,
                owed: [0.0; 3],
                seed,
                next_arc: 0.0,
                materials: mats.clone(),
            },
            Transform::from_translation(pos),
            Visibility::default(),
        ))
        .with_children(|o| {
            let quad = fx.quad.clone();
            let sprite = |part: OrbPart, mat: &Handle<StandardMaterial>| {
                (
                    part,
                    Mesh3d(quad.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                )
            };
            o.spawn((
                OrbPart::Core,
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(mats.core.clone()),
                Transform::from_scale(Vec3::ZERO),
                NotShadowCaster,
                NoFrustumCulling,
            ));
            o.spawn(sprite(OrbPart::Glow, &mats.glow));
            o.spawn(sprite(OrbPart::Halo, &mats.halo));
            for i in 0..s.flames.min(24) {
                o.spawn(sprite(OrbPart::Flame(i), &mats.flame));
            }
            for i in 0..s.wisps.min(32) {
                o.spawn(sprite(OrbPart::Wisp(i), &mats.wisp));
            }
            for a in 0..s.arcs.min(12) {
                for k in 0..3 {
                    o.spawn((
                        OrbPart::Arc(a, k),
                        Mesh3d(assets.arc.clone()),
                        MeshMaterial3d(mats.arc.clone()),
                        Transform::from_scale(Vec3::ZERO),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                }
            }
            o.spawn((
                OrbPart::Light,
                PointLight {
                    color: Color::srgb(c[0], c[1] * 1.2, c[2] * 2.0),
                    intensity: 0.0,
                    range: s.light_range,
                    shadows_enabled: false,
                    ..default()
                },
            ));
        })
        .id()
}

/// Server → us: a boss coming (lightning), appearing (flash), and its blasts
/// being hurled and blowing up.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn receive_boss_messages(
    (mut lightning, mut spawned, mut launched, mut exploded): (
        Query<&mut MessageReceiver<shared::BossLightning>>,
        Query<&mut MessageReceiver<shared::BossSpawned>>,
        Query<&mut MessageReceiver<shared::BossBlastLaunched>>,
        Query<&mut MessageReceiver<shared::BossBlastExploded>>,
    ),
    (dog_assets, dog_settings): (Option<Res<DogAssets>>, Res<DogSettings>),
    (assets, fx): (Option<Res<BossAssets>>, Option<Res<ExplosionAssets>>),
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    settings: Res<BossSettings>,
    avatars: Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut orbs: Query<(Entity, &mut BossOrb)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut boom: EventWriter<crate::vfx::Explosion>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let (Some(dog_assets), Some(assets), Some(fx), Some(sounds)) = (dog_assets, assets, fx, sounds) else {
        return;
    };
    for mut rx in &mut lightning {
        for msg in rx.receive() {
            let at = Vec3::from_array(msg.at);
            *seq = seq.wrapping_add(1);
            crate::dogs::spawn_lightning(
                &mut commands,
                &dog_assets,
                &dog_settings,
                at,
                *seq,
                shared::boss::BOSS_PRE_SPAWN_SECS,
                true,
            );
            commands.spawn(boss_sound(sounds.boss_spawn.clone(), vols.boss_spawn, at + Vec3::Y, false));
        }
    }
    for mut rx in &mut spawned {
        for msg in rx.receive() {
            let at = Vec3::from_array(msg.at);
            crate::dogs::spawn_flash(
                &mut commands,
                &dog_assets,
                &dog_settings,
                &mut materials,
                at + Vec3::Y * 0.6,
                true,
                settings.flash_size,
            );
        }
    }
    for mut rx in &mut launched {
        for msg in rx.receive() {
            let from = Vec3::from_array(msg.from);
            let flight = Flight {
                id: msg.id,
                dir: Vec3::from_array(msg.dir).normalize_or(Vec3::NEG_Z),
                speed: msg.speed,
                age: 0.0,
            };
            // The ball its boss has been winding up, if we've one: it's
            // thrown from where it is (easing onto the real path).
            let charging = orbs
                .iter_mut()
                .find(|(_, o)| o.flight.is_none() && o.peer == Some(msg.boss));
            let orb = match charging {
                Some((entity, mut orb)) => {
                    orb.offset = orb.pos - from;
                    orb.pos = from;
                    orb.charge = 1.0;
                    orb.flight = Some(flight);
                    entity
                }
                None => {
                    *seq = seq.wrapping_add(1);
                    let avatar = avatars
                        .iter()
                        .find(|(_, a)| peer_of(a.src, &ids, &interp) == Some(msg.boss))
                        .map(|(e, _)| e);
                    spawn_orb(
                        &mut commands,
                        &assets,
                        &fx,
                        &settings,
                        &mut materials,
                        from,
                        avatar,
                        Some(msg.boss),
                        Some(flight),
                        *seq,
                    )
                }
            };
            // Its roar, from the ball as it flies.
            let sound = commands
                .spawn(boss_sound(sounds.boss_blast.clone(), vols.boss_blast, Vec3::ZERO, true))
                .id();
            commands.entity(orb).add_child(sound);
        }
    }
    for mut rx in &mut exploded {
        for msg in rx.receive() {
            let at = Vec3::from_array(msg.at);
            for (entity, orb) in &orbs {
                if orb.flight.as_ref().is_some_and(|f| f.id == msg.id) {
                    commands.entity(entity).try_despawn();
                }
            }
            boom.write(crate::vfx::Explosion {
                feet: at,
                variant: 0,
                phd: false,
                dog: false,
                boss: true,
                frag: false,
            });
        }
    }
}

/// Play the clip each boss's pose calls for (its walk at the speed it's
/// really going), with its sounds: a growl now and then, its smash, its
/// death — and start a ball forming in its hands as it winds up a blast.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn animate_bosses(
    time: Res<Time>,
    settings: Res<BossSettings>,
    (assets, fx): (Option<Res<BossAssets>>, Option<Res<ExplosionAssets>>),
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    poses: Query<&PlayerPose>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut avatars: Query<(Entity, &RemoteAvatar, &Transform, &mut BossMotion, Option<&BossRig>)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    orbs: Query<&BossOrb>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let (Some(assets), Some(fx), Some(sounds)) = (assets, fx, sounds) else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    let s = &*settings;
    let blend = Duration::from_secs_f32(s.blend_secs.max(0.0));

    for (entity, avatar, tf, mut motion, rig) in &mut avatars {
        let Ok(pose) = poses.get(avatar.src) else { continue };
        let raw = match motion.prev {
            Some(prev) => (pose.translation - prev).with_y(0.0) / dt,
            None => Vec3::ZERO,
        };
        motion.prev = Some(pose.translation);
        let k = 1.0 - (-dt / 0.15).exp();
        motion.vel = motion.vel.lerp(raw, k);
        let speed = motion.vel.length();
        let head = tf.translation + Vec3::Y * BOSS_HEIGHT * 0.9;

        let state = if !pose.alive {
            BossState::Dead
        } else {
            match pose.zombie {
                ZombieAnim::BossSmash => BossState::Smash,
                ZombieAnim::BossBlast => BossState::Blast,
                ZombieAnim::BossWalk if speed >= s.min_move_speed => BossState::Walk,
                _ => BossState::Idle,
            }
        };

        // Growls, every so often while it's up.
        if state != BossState::Dead {
            let next = *motion.next_growl.get_or_insert_with(|| {
                *seq = seq.wrapping_add(1);
                now + 1.0 + rand01(seq.wrapping_mul(2_654_435_761)) * s.growl_every.0
            });
            if now >= next {
                *seq = seq.wrapping_add(1);
                let gap = s.growl_every.0 + rand01(seq.wrapping_mul(2_654_435_761)) * (s.growl_every.1 - s.growl_every.0).max(0.0);
                motion.next_growl = Some(now + gap);
                let growl = commands.spawn(boss_sound(sounds.boss_growl.clone(), vols.boss_growl, Vec3::ZERO, false)).id();
                // (Rides along with it: the avatar's scaled, so the offset
                // is in its units.)
                commands.entity(growl).insert(Transform::from_translation(
                    Vec3::Y * BOSS_HEIGHT * 0.85 / s.scale.max(0.001),
                ));
                commands.entity(entity).add_child(growl);
            }
        }

        let Some((mut player, mut transitions)) = rig.and_then(|r| players.get_mut(r.player).ok()) else {
            continue;
        };
        if state != motion.state {
            match state {
                BossState::Idle => {
                    transitions.play(&mut player, assets.idle, blend).set_repeat(RepeatAnimation::Forever);
                }
                BossState::Walk => {
                    transitions.play(&mut player, assets.walk, blend).set_repeat(RepeatAnimation::Forever);
                }
                BossState::Smash | BossState::Blast => {
                    transitions
                        .play(&mut player, assets.attack, Duration::from_secs_f32(0.1))
                        .set_repeat(RepeatAnimation::Never)
                        .replay();
                    if state == BossState::Smash {
                        commands.spawn(boss_sound(sounds.boss_attack.clone(), vols.boss_attack, head, false));
                    } else if let Some(peer) = peer_of(avatar.src, &ids, &interp) {
                        // A ball starts forming as its arms go up.
                        if !orbs.iter().any(|o| o.flight.is_none() && o.peer == Some(peer)) {
                            *seq = seq.wrapping_add(1);
                            spawn_orb(
                                &mut commands,
                                &assets,
                                &fx,
                                s,
                                &mut materials,
                                head,
                                Some(entity),
                                Some(peer),
                                None,
                                *seq,
                            );
                        }
                    }
                }
                BossState::Dead => {
                    transitions
                        .play(&mut player, assets.death, Duration::from_secs_f32(0.15))
                        .set_repeat(RepeatAnimation::Never);
                    commands.spawn(boss_sound(sounds.boss_death.clone(), vols.boss_death, head, false));
                }
            }
            motion.state = state;
        }
        if state == BossState::Walk {
            if let Some(active) = player.animation_mut(assets.walk) {
                active.set_speed((speed * s.walk_per_mps).max(0.05));
            }
        }
    }
}

/// Swell each forming ball in its boss's hands, fly each hurled one on, and
/// shed sparks, embers and smoke; clear any that's lost its boss or message.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_orbs(
    time: Res<Time>,
    settings: Res<BossSettings>,
    fx: Option<Res<ExplosionAssets>>,
    avatars: Query<(&Transform, &RemoteAvatar, Option<&BossRig>), With<BossVisual>>,
    poses: Query<&PlayerPose>,
    bones: Query<&GlobalTransform>,
    mut orbs: Query<(Entity, &mut BossOrb, &mut Transform), Without<BossVisual>>,
    particles: Query<(), With<OrbParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(fx) = fx else { return };
    let dt = time.delta_secs();
    let s = &*settings;
    let mut budget = MAX_PARTICLES.saturating_sub(particles.iter().count());
    let charge_window = BLAST_RELEASE_SECS - BLAST_CHARGE_START_SECS;

    for (entity, mut orb, mut tf) in &mut orbs {
        orb.age += dt;
        let mut rng = orb.seed;
        let mut roll = |k: u32| {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223 ^ k);
            rand01(rng)
        };
        let flying = orb.flight.is_some();
        if let Some(flight) = orb.flight.as_mut() {
            flight.age += dt;
            if flight.age > BLAST_LIFETIME_SECS + ORB_GRACE_SECS {
                commands.entity(entity).try_despawn();
                continue;
            }
            let step = flight.dir * flight.speed * dt;
            orb.pos += step;
            orb.offset *= (-dt / 0.08).exp();
        } else {
            // Forming: in its hands (between them, or over its head), as long
            // as it's still winding up.
            let Some((avatar_tf, avatar, rig)) = orb.avatar.and_then(|a| avatars.get(a).ok()) else {
                commands.entity(entity).try_despawn();
                continue;
            };
            let winding = poses.get(avatar.src).is_ok_and(|p| p.alive && p.zombie == ZombieAnim::BossBlast);
            if (!winding && orb.age > 0.3) || orb.age > charge_window + BLAST_CHARGE_START_SECS + ORB_GRACE_SECS {
                commands.entity(entity).try_despawn();
                continue;
            }
            let hands: Vec<Vec3> = rig
                .map(|r| r.hands.iter().flatten().filter_map(|h| bones.get(*h).ok()).map(|g| g.translation()).collect())
                .unwrap_or_default();
            orb.pos = if hands.is_empty() {
                avatar_tf.translation + Vec3::Y * BOSS_HEIGHT * 0.95
            } else {
                hands.iter().sum::<Vec3>() / hands.len() as f32
            };
            // It only starts forming as the arms go up, then swells in fast.
            let t = orb.age - BLAST_CHARGE_START_SECS;
            let x = (t / s.charge_fade_secs.max(0.01)).clamp(0.0, 1.0);
            orb.charge = x * x * (3.0 - 2.0 * x);
        }
        tf.translation = orb.pos + orb.offset;
        let at = tf.translation;
        let charge = orb.charge;

        // Sparks drawn in while it charges; embers and smoke streaming off
        // it in flight.
        let emit = |rate: f32, owed: &mut f32| {
            *owed += rate * dt;
            let n = owed.floor();
            *owed -= n;
            n as usize
        };
        let mut spawn = |p: OrbParticle, at: Vec3, commands: &mut Commands| {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let material = p.material.clone();
            commands.spawn((
                StateScoped(AppState::InGame),
                p,
                Mesh3d(fx.quad.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(at).with_scale(Vec3::ZERO),
                NotShadowCaster,
                NoFrustumCulling,
            ));
        };
        if !flying {
            if charge > 0.0 {
                let mut owed = orb.owed[0];
                for _ in 0..emit(s.charge_sparks * charge, &mut owed) {
                    let dir = Vec3::new(roll(1) - 0.5, roll(2) - 0.5, roll(3) - 0.5).normalize_or(Vec3::Y);
                    let from = at + dir * (0.9 + roll(4) * 0.8);
                    let material = materials.add(glow_material(s.core_color, s.brightness * 0.6, Some(fx.glow.clone())));
                    spawn(
                        OrbParticle {
                            vel: Vec3::ZERO,
                            age: 0.0,
                            life: 0.22 + roll(5) * 0.15,
                            size: (0.07, 0.03),
                            into: Some(at),
                            roll: 0.0,
                            material,
                            brightness: s.brightness * 0.6,
                            color: s.core_color,
                            smoke: false,
                        },
                        from,
                        &mut commands,
                    );
                }
                orb.owed[0] = owed;
            }
        } else {
            let dir = orb.flight.as_ref().map_or(Vec3::NEG_Z, |f| f.dir);
            let mut owed = orb.owed[1];
            for _ in 0..emit(s.trail_rate, &mut owed) {
                let jitter = Vec3::new(roll(11) - 0.5, roll(12) - 0.5, roll(13) - 0.5) * 2.0;
                let back = -dir * (roll(14) * 0.3);
                let hot = roll(15) < 0.35;
                let color = if hot { s.core_color } else { s.fire_color };
                let brightness = s.brightness * if hot { 0.5 } else { 0.3 };
                let material = materials.add(glow_material(color, brightness, Some(fx.glow.clone())));
                spawn(
                    OrbParticle {
                        vel: jitter * s.trail_spread - dir * 1.5,
                        age: 0.0,
                        life: s.trail_life * (0.6 + roll(16) * 0.8),
                        size: (s.trail_size * (0.6 + roll(17) * 0.6), 0.02),
                        into: None,
                        roll: roll(18) * std::f32::consts::TAU,
                        material,
                        brightness,
                        color,
                        smoke: false,
                    },
                    at + back + jitter * 0.12,
                    &mut commands,
                );
            }
            orb.owed[1] = owed;
            let mut owed = orb.owed[2];
            for _ in 0..emit(s.smoke_rate, &mut owed) {
                let material = materials.add(StandardMaterial {
                    base_color: Color::srgba(0.12, 0.08, 0.06, 0.0),
                    base_color_texture: Some(fx.fire[1].clone()),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                spawn(
                    OrbParticle {
                        vel: Vec3::new(roll(21) - 0.5, roll(22) * 0.6, roll(23) - 0.5) * 0.8,
                        age: 0.0,
                        life: 0.9 + roll(24) * 0.5,
                        size: (0.35, 1.3),
                        into: None,
                        roll: roll(25) * std::f32::consts::TAU,
                        material,
                        brightness: 0.45,
                        color: [0.12, 0.08, 0.06],
                        smoke: true,
                    },
                    at - dir * 0.4,
                    &mut commands,
                );
            }
            orb.owed[2] = owed;
        }
        orb.seed = rng;
    }
}

/// Lay out and animate each ball's parts: the core and its glows facing
/// the camera, flames turning round it, wisps orbiting, arcs crackling off
/// it, its light flickering — all scaled by how far it's charged.
#[allow(clippy::type_complexity)]
fn update_orb_parts(
    time: Res<Time>,
    settings: Res<BossSettings>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut orbs: Query<(&mut BossOrb, &Transform, &Children)>,
    mut parts: Query<(&OrbPart, &mut Transform, Option<&mut PointLight>), Without<BossOrb>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let s = &*settings;
    let t = time.elapsed_secs();
    let cam = camera.into_inner();
    let cam_pos = cam.translation();
    let cam_up = cam.up().as_vec3();
    let cam_right = cam.right().as_vec3();

    for (mut orb, tf, children) in &mut orbs {
        let k = orb.charge;
        let at = tf.translation;
        let seed = orb.seed as f32 * 0.001;
        // A restless pulse, stronger while it's forming.
        let pulse = 1.0 + 0.08 * (t * 23.0 + seed).sin() + 0.05 * (t * 37.0 + seed * 2.0).sin();
        let billboard = |roll: f32| smoke_billboard_rotation(at, cam_pos, cam_up, cam_right, roll);
        let to_cam = (cam_pos - at).normalize_or(Vec3::Z);
        // Re-fork the arcs now and then.
        let refork = t >= orb.next_arc;
        if refork {
            orb.next_arc = t + 1.0 / s.arc_hz.max(1.0);
        }
        let arc_seed = (t * s.arc_hz.max(1.0)).floor() as u32 ^ orb.seed;

        // (The ball's brightness follows its charge.)
        let m = &orb.materials;
        let set = |materials: &mut Assets<StandardMaterial>, h: &Handle<StandardMaterial>, c: [f32; 3], b: f32| {
            if let Some(mat) = materials.get_mut(h) {
                mat.base_color = LinearRgba::rgb(c[0] * b, c[1] * b, c[2] * b).into();
            }
        };
        set(&mut materials, &m.core, s.core_color, s.brightness * 1.5 * k);
        set(&mut materials, &m.glow, s.core_color, s.brightness * 0.35 * k * pulse);
        set(&mut materials, &m.halo, s.fire_color, s.brightness * 0.12 * k);
        set(&mut materials, &m.flame, s.fire_color, s.brightness * 0.22 * k);
        set(&mut materials, &m.wisp, s.fire_color, s.brightness * 0.5 * k);
        set(&mut materials, &m.arc, s.core_color, s.brightness * 0.8 * k);

        for child in children.iter() {
            let Ok((part, mut ptf, light)) = parts.get_mut(child) else { continue };
            match *part {
                OrbPart::Core => {
                    *ptf = Transform::from_scale(Vec3::splat(s.core_size * k * pulse));
                }
                OrbPart::Glow => {
                    *ptf = Transform::from_rotation(billboard(t * 1.3))
                        .with_scale(Vec3::splat(s.glow_size * k * pulse));
                }
                OrbPart::Halo => {
                    *ptf = Transform::from_rotation(billboard(-t * 0.7))
                        .with_scale(Vec3::splat(s.halo_size * k * (0.95 + 0.1 * (t * 9.0 + seed).sin())));
                }
                OrbPart::Flame(i) => {
                    let n = s.flames.max(1) as f32;
                    let phase = i as f32 / n * std::f32::consts::TAU + t * s.flame_spin;
                    // Round the core in the plane facing the camera, licking
                    // outward, each flickering in size.
                    let flicker = 0.7 + 0.3 * (t * 17.0 + i as f32 * 2.3 + seed).sin();
                    let rot = billboard(phase);
                    let out = rot * Vec3::Y;
                    *ptf = Transform::from_translation(out * s.core_size * 0.9 * k)
                        .with_rotation(rot)
                        .with_scale(Vec3::new(0.7, 1.0, 1.0) * s.flame_size * k * flicker);
                }
                OrbPart::Wisp(i) => {
                    let n = s.wisps.max(1) as f32;
                    let a = i as f32 / n * std::f32::consts::TAU + t * s.wisp_speed * std::f32::consts::TAU * (1.0 + (i % 3) as f32 * 0.2);
                    // Each on its own tilted ring.
                    let tilt = Quat::from_rotation_x(i as f32 * 1.7) * Quat::from_rotation_z(i as f32 * 0.9);
                    let p = tilt * Vec3::new(a.cos(), 0.0, a.sin()) * s.wisp_orbit * k;
                    *ptf = Transform::from_translation(p)
                        .with_rotation(billboard(0.0))
                        .with_scale(Vec3::splat(0.12 * k));
                }
                OrbPart::Arc(a, seg) => {
                    if !refork {
                        continue;
                    }
                    // A jagged three-step bolt off the core's surface; some
                    // dark each beat.
                    let r = |j: u32| rand01((arc_seed ^ a.wrapping_mul(0x9E37) ^ j).wrapping_mul(2_654_435_761));
                    if r(99) < 0.35 || k < 0.2 {
                        *ptf = Transform::from_scale(Vec3::ZERO);
                        continue;
                    }
                    let dir = Vec3::new(r(1) - 0.5, r(2) - 0.5, r(3) - 0.5).normalize_or(to_cam);
                    let step = s.arc_length * k / 3.0;
                    let mut p = dir * s.core_size * 0.5 * k;
                    for j in 0..seg {
                        let wob = Vec3::new(r(10 + j) - 0.5, r(20 + j) - 0.5, r(30 + j) - 0.5);
                        p += (dir + wob * 1.2).normalize_or(dir) * step;
                    }
                    let wob = Vec3::new(r(10 + seg) - 0.5, r(20 + seg) - 0.5, r(30 + seg) - 0.5);
                    let q = p + (dir + wob * 1.2).normalize_or(dir) * step;
                    let d = q - p;
                    let len = d.length().max(1e-4);
                    *ptf = Transform {
                        translation: (p + q) * 0.5,
                        rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
                        scale: Vec3::new(0.025, len, 0.025),
                    };
                }
                OrbPart::Light => {
                    if let Some(mut light) = light {
                        let flicker = 0.75 + 0.25 * (t * 31.0 + seed).sin().abs();
                        light.intensity = s.light * k * flicker;
                        light.range = s.light_range;
                    }
                }
            }
        }
    }
}

/// Fly, shrink, fade and billboard every spark, ember and smoke puff.
fn update_orb_particles(
    time: Res<Time>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut particles: Query<(Entity, &mut OrbParticle, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cam = camera.into_inner();
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    for (entity, mut p, mut tf) in &mut particles {
        p.age += dt;
        let x = p.age / p.life.max(0.01);
        if x >= 1.0 {
            materials.remove(&p.material);
            commands.entity(entity).try_despawn();
            continue;
        }
        if let Some(into) = p.into {
            // Sucked into the ball.
            tf.translation = tf.translation.lerp(into, (dt * 9.0).min(1.0));
        } else {
            let drag = (-dt * if p.smoke { 1.5 } else { 3.0 }).exp();
            p.vel *= drag;
            if p.smoke {
                p.vel.y += 0.6 * dt;
            }
            tf.translation += p.vel * dt;
        }
        let size = p.size.0.lerp(p.size.1, if p.smoke { x.sqrt() } else { x });
        tf.scale = Vec3::splat(size.max(1e-4));
        tf.rotation = smoke_billboard_rotation(tf.translation, cam_pos, cam_up, cam_right, p.roll);
        let fade = if p.smoke { (x * 4.0).min(1.0) * (1.0 - x) } else { 1.0 - x * x };
        if let Some(m) = materials.get_mut(&p.material) {
            m.base_color = if p.smoke {
                Color::srgba(p.color[0], p.color[1], p.color[2], p.brightness * fade)
            } else {
                let b = p.brightness * fade;
                LinearRgba::rgb(p.color[0] * b, p.color[1] * b, p.color[2] * b).into()
            };
        }
    }
}

/// Keep every boss sound's loudness matched to how far the listener is from
/// it — a squared fade to silence at `sound_max_distance`.
fn fade_boss_sounds(
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    settings: Res<BossSettings>,
    global_volume: Res<GlobalVolume>,
    mut playing: Query<(&GlobalTransform, &BossSound, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else { return };
    let ear = ear.translation();
    let max = settings.sound_max_distance.max(1.0);
    for (gt, sound, mut sink) in &mut playing {
        let fade = (1.0 - ear.distance(gt.translation()) / max).clamp(0.0, 1.0).powi(2);
        sink.set_volume(Volume::Linear((sound.loudness * fade).max(0.0)) * global_volume.volume);
    }
}

/// The debug section's previews: the orange lightning 10 m ahead, or a ball
/// thrown from in front of us, straight ahead.
#[allow(clippy::too_many_arguments)]
fn run_previews(
    mut preview: ResMut<BossPreview>,
    (dog_assets, dog_settings): (Option<Res<DogAssets>>, Res<DogSettings>),
    (assets, fx): (Option<Res<BossAssets>>, Option<Res<ExplosionAssets>>),
    settings: Res<BossSettings>,
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    if !(preview.lightning || preview.orb) {
        return;
    }
    let (Some(dog_assets), Some(assets), Some(fx), Some(sounds)) = (dog_assets, assets, fx, sounds) else {
        return;
    };
    let cam = camera.into_inner();
    let fwd = cam.forward().as_vec3();
    let flat = fwd.with_y(0.0).normalize_or(Vec3::NEG_Z);
    if std::mem::take(&mut preview.lightning) {
        let at = cam.translation() - Vec3::Y * crate::EYE_HEIGHT + flat * 10.0;
        crate::dogs::spawn_lightning(
            &mut commands,
            &dog_assets,
            &dog_settings,
            at,
            0,
            shared::boss::BOSS_PRE_SPAWN_SECS,
            true,
        );
        commands.spawn(boss_sound(sounds.boss_spawn.clone(), vols.boss_spawn, at + Vec3::Y, false));
    }
    if std::mem::take(&mut preview.orb) {
        // (Slow, and a long way off: something to look at, with no message
        // to end it — it goes on its own.)
        let orb = spawn_orb(
            &mut commands,
            &assets,
            &fx,
            &settings,
            &mut materials,
            cam.translation() + fwd * 3.0,
            None,
            None,
            Some(Flight {
                id: u32::MAX,
                dir: fwd,
                speed: 3.0,
                age: 0.0,
            }),
            7,
        );
        let sound = commands
            .spawn(boss_sound(sounds.boss_blast.clone(), vols.boss_blast, Vec3::ZERO, true))
            .id();
        commands.entity(orb).add_child(sound);
    }
}

/// The "Boss (Zombies)" debug section: strike a boss in, preview its
/// lightning and its blast, and tune its look and sounds.
#[derive(SystemParam)]
pub(crate) struct BossDebug<'w, 's> {
    settings: ResMut<'w, BossSettings>,
    preview: ResMut<'w, BossPreview>,
    local: Query<'w, 's, &'static LocalId, With<GameClient>>,
    lobbies: Query<'w, 's, &'static shared::Lobby>,
    spawn: Query<'w, 's, &'static mut TriggerSender<shared::SpawnBoss>, With<GameClient>>,
}

impl BossDebug<'_, '_> {
    /// Its section in the main debug panel (`debug_ui`).
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let settings = &mut *self.settings;
        let preview = &mut *self.preview;
        let local = &self.local;
        let lobbies = &self.lobbies;
        let spawn = &mut self.spawn;
        let in_zombies = crate::zombies_hud::zombies_game(local, lobbies).is_some();
        ui.label(format!(
            "Comes every {}th round (not dog rounds). Smash within {} m, blasts {}–{} m.",
            shared::boss::BOSS_ROUND_EVERY,
            shared::boss::MELEE_RANGE,
            shared::boss::BLAST_MIN_RANGE,
            shared::boss::BLAST_MAX_RANGE
        ));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(in_zombies, egui::Button::new("Spawn a boss ahead (leader)"))
                .clicked()
            {
                if let Ok(mut s) = spawn.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::SpawnBoss);
                }
            }
            if ui.button("Preview lightning").clicked() {
                preview.lightning = true;
            }
            if ui.button("Preview blast").clicked() {
                preview.orb = true;
            }
        });
        let s = &mut *settings;
        ui.collapsing("Model + walk", |ui| {
            ui.add(egui::Slider::new(&mut s.scale, 0.1f32..=2.0).text("scale"));
            ui.add(egui::Slider::new(&mut s.yaw_offset_deg, -180.0f32..=180.0).text("turn (°)"));
            ui.add(egui::Slider::new(&mut s.walk_per_mps, 0.0f32..=3.0).text("walk speed per m/s"));
            ui.add(egui::Slider::new(&mut s.min_move_speed, 0.0f32..=2.0).text("stands below (m/s)"));
            ui.add(egui::Slider::new(&mut s.blend_secs, 0.0f32..=1.0).text("clip blend (s)"));
            ui.add(egui::Slider::new(&mut s.flash_size, 0.1f32..=5.0).text("spawn flash size (×)"));
        });
        ui.collapsing("Energy ball", |ui| {
            ui.add(egui::Slider::new(&mut s.charge_fade_secs, 0.01f32..=1.0).text("swells in over (s)"));
            ui.add(egui::Slider::new(&mut s.core_size, 0.02f32..=2.0).text("core size (m)"));
            ui.add(egui::Slider::new(&mut s.glow_size, 0.1f32..=5.0).text("glow size (m)"));
            ui.add(egui::Slider::new(&mut s.halo_size, 0.1f32..=8.0).text("halo size (m)"));
            ui.horizontal(|ui| {
                ui.label("core / fire colour");
                ui.color_edit_button_rgb(&mut s.core_color);
                ui.color_edit_button_rgb(&mut s.fire_color);
            });
            ui.add(egui::Slider::new(&mut s.brightness, 0.0f32..=60.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.flames, 0u32..=24).text("flames (new balls)"));
            ui.add(egui::Slider::new(&mut s.flame_size, 0.05f32..=3.0).text("flame size (m)"));
            ui.add(egui::Slider::new(&mut s.flame_spin, 0.0f32..=10.0).text("flame spin"));
            ui.add(egui::Slider::new(&mut s.wisps, 0u32..=32).text("wisps (new balls)"));
            ui.add(egui::Slider::new(&mut s.wisp_orbit, 0.0f32..=3.0).text("wisp orbit (m)"));
            ui.add(egui::Slider::new(&mut s.wisp_speed, 0.0f32..=5.0).text("wisp speed (rev/s)"));
            ui.add(egui::Slider::new(&mut s.arcs, 0u32..=12).text("arcs (new balls)"));
            ui.add(egui::Slider::new(&mut s.arc_length, 0.0f32..=3.0).text("arc length (m)"));
            ui.add(egui::Slider::new(&mut s.arc_hz, 1.0f32..=60.0).text("arc re-forks / s"));
            ui.add(
                egui::Slider::new(&mut s.light, 0.0f32..=20_000_000.0)
                    .logarithmic(true)
                    .text("light (lm)"),
            );
            ui.add(egui::Slider::new(&mut s.light_range, 1.0f32..=60.0).text("light range (m)"));
            ui.add(egui::Slider::new(&mut s.charge_sparks, 0.0f32..=200.0).text("sparks drawn in / s"));
            ui.add(egui::Slider::new(&mut s.trail_rate, 0.0f32..=400.0).text("embers / s"));
            ui.add(egui::Slider::new(&mut s.trail_life, 0.05f32..=3.0).text("ember life (s)"));
            ui.add(egui::Slider::new(&mut s.trail_size, 0.02f32..=2.0).text("ember size (m)"));
            ui.add(egui::Slider::new(&mut s.trail_spread, 0.0f32..=5.0).text("ember scatter (m/s)"));
            ui.add(egui::Slider::new(&mut s.smoke_rate, 0.0f32..=100.0).text("smoke puffs / s"));
        });
        ui.collapsing("Sounds", |ui| {
            ui.add(egui::Slider::new(&mut s.growl_every.0, 1.0f32..=60.0).text("growls every, min (s)"));
            ui.add(egui::Slider::new(&mut s.growl_every.1, 1.0f32..=60.0).text("growls every, max (s)"));
            ui.add(egui::Slider::new(&mut s.sound_max_distance, 5.0f32..=200.0).text("heard within (m)"));
            ui.label("Volumes: Sound volumes. Explosion size: Bomb Shot → boss blast's size.");
        });
        ui.horizontal(|ui| {
            if ui.button("Print boss settings to console").clicked() {
                info!("boss settings:\n{}", s.to_rust());
            }
            if ui.button("Reset").clicked() {
                *s = BossSettings::default();
            }
        });
    }
}
