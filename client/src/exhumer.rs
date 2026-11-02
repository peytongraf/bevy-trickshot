//! The Exhumer on this client — Der Tempel's wonder weapon (`shared::exhumer`;
//! the server's `exhumer` runs it). Two triggers:
//!
//! * **Aim (held) — its beam** ([`beam_input`]: the aim button with it in
//!   hand — no aim-down-sight for it, `weapons::ads`; sent with our input as
//!   `PlayerInput::beaming`). Every player's beam is drawn ([`draw_beams`]:
//!   ours from the gun, theirs from their `PlayerPose::beaming`) out to its
//!   reach or the first wall, glowing motes drifting back along it into the
//!   gun, its hum looping. The zombies it has hold of
//!   ([`shared::exhumer::ExhumerDrain`]) rise into the air as they're
//!   drained, turning and tipping back, a soul-light growing in each chest
//!   ([`Drained`]) — until one's soul is torn out
//!   ([`shared::exhumer::ExhumerSouls`]): a wisp streaks from its chest
//!   back into the gun and the empty body's thrown limp out of the air (a
//!   ragdoll, `ragdoll`).
//! * **Fire — let them out** (`weapons::weapon` asks; sent here as
//!   [`shared::exhumer::ReleaseSouls`]): a wisp after each zombie the
//!   server sent one for ([`shared::exhumer::ExhumerSwarm`]), following it.
//!
//! Its souls are its only ammo: the server's count (`Weapon::souls`), shown
//! as its mag while it's in hand ([`sync_soul_mag`]) — they brighten its
//! camo (`pap`).
//!
//! Its sounds are placeholders at their final paths
//! (`audio/zombies/exhumer/`). Everything here is `StateScoped(InGame)` and
//! gone once played out (the souls and the beam are `Weapon`'s, reset with
//! it every game). Tuned in the debug panel's "Easter egg (Der Tempel)" →
//! "The Exhumer".

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use bevy_egui::egui;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::{Interpolated, LocalId, MessageReceiver, PeerId, TriggerSender};
use shared::exhumer::{beam_dir, ExhumerDrain, ExhumerSouls, ExhumerSwarm, ReleaseSouls, BEAM_RANGE, LIFT_HEIGHT, SWARM_SPEED};
use shared::{PlayerId, PlayerPose};

use crate::audio_engine::prelude::*;
use crate::keybinds::KeyBindings;
use crate::net::{GameClient, RemoteAvatar};
use crate::vfx::{smoke_billboard_rotation, ExplosionAssets};
use crate::weapons::Weapon;
use crate::{killcam, menu, AppState, WorldModelCamera, EYE_HEIGHT};

pub(crate) struct ExhumerPlugin;

impl Plugin for ExhumerPlugin {
    fn build(&self, app: &mut App) {
        let can_beam = || menu::weapon_active.and(killcam::no_killcam);
        app.init_resource::<ExhumerFxSettings>()
            .init_resource::<ExhumerDebug>()
            .add_systems(Startup, load_sounds)
            .add_systems(
                Update,
                (
                    (beam_input.run_if(can_beam()), beam_off.run_if(not(can_beam()))),
                    sync_soul_mag,
                    send_release,
                    debug_give,
                    (receive_drains, receive_souls, receive_swarms),
                    lift_avatars.after(crate::net::follow_remote_avatars),
                    run_glows,
                    draw_beams,
                    fly_motes,
                    fly_wisps,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            // (Not left humming once out of a game.)
            .add_systems(OnExit(AppState::InGame), |mut weapon: ResMut<Weapon>| weapon.beaming = false);
    }
}

/// Panel-tunable look ("The Exhumer").
#[derive(Resource, Clone, Debug)]
pub(crate) struct ExhumerFxSettings {
    /// The souls' colour (linear), a wisp's size (m) and glow, and how many
    /// glows trail behind it, how far apart (s).
    pub(crate) color: [f32; 3],
    pub(crate) wisp_size: f32,
    pub(crate) wisp_glow: f32,
    pub(crate) trail: u32,
    pub(crate) trail_lag: f32,
    /// How long (s) a torn-out soul takes back to the gun, and how high (m)
    /// its arc bows.
    pub(crate) return_secs: f32,
    pub(crate) arc: f32,
    /// The beam: its width (m), how bright, how much it flickers (0..1), and
    /// its light (lumens); the motes drawn back along it — how many a
    /// second, how fast (m/s), how big (m).
    pub(crate) beam_width: f32,
    pub(crate) beam_glow: f32,
    pub(crate) beam_flicker: f32,
    pub(crate) beam_light: f32,
    pub(crate) motes_per_sec: f32,
    pub(crate) mote_speed: f32,
    pub(crate) mote_size: f32,
    /// A drained body: how fast (rad/s) it turns, how far (°) it tips back;
    /// the glow in its chest (lumens, at its brightest).
    pub(crate) lift_spin: f32,
    pub(crate) lift_tilt_deg: f32,
    pub(crate) chest_light: f32,
    /// The sounds' volume (linear, before the mix), and the beam's hum's.
    pub(crate) volume: f32,
    pub(crate) beam_volume: f32,
}

impl Default for ExhumerFxSettings {
    fn default() -> Self {
        Self {
            color: [0.55, 1.0, 0.65],
            wisp_size: 0.45,
            wisp_glow: 4.0,
            trail: 6,
            trail_lag: 0.035,
            return_secs: 0.75,
            arc: 1.6,
            beam_width: 0.09,
            beam_glow: 3.0,
            beam_flicker: 0.35,
            beam_light: 80_000.0,
            motes_per_sec: 40.0,
            mote_speed: 14.0,
            mote_size: 0.16,
            lift_spin: 0.9,
            lift_tilt_deg: 22.0,
            chest_light: 120_000.0,
            volume: 1.4,
            beam_volume: 0.9,
        }
    }
}

#[derive(Resource)]
struct ExhumerSounds {
    grab: Handle<AudioSource>,
    tear: Handle<AudioSource>,
    release: Handle<AudioSource>,
    beam: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(ExhumerSounds {
        grab: asset_server.load("audio/zombies/exhumer/lift.mp3"),
        tear: asset_server.load("audio/zombies/exhumer/soul_tear.mp3"),
        release: asset_server.load("audio/zombies/exhumer/release.mp3"),
        beam: asset_server.load("audio/zombies/exhumer/beam.mp3"),
    });
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn sound_at(commands: &mut Commands, clip: &Handle<AudioSource>, at: Vec3, volume: f32) {
    commands.spawn((
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip.clone()),
        Transform::from_translation(at),
        crate::positional_playback(Volume::Linear(volume)),
    ));
}

// --- our triggers -------------------------------------------------------------------------

/// The aim button held with the Exhumer in hand (and our hands free, the
/// window ours): its beam's on.
#[allow(clippy::too_many_arguments)]
fn beam_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    hands: crate::weapons::Hands,
    mut weapon: ResMut<Weapon>,
) {
    let on = weapon.exhumer_in_hand()
        && weapon.busy.is_none()
        && hands.free()
        && window.cursor_options.grab_mode != CursorGrabMode::None
        && binds.aim.pressed(&keys, &mouse);
    if weapon.beaming != on {
        weapon.beaming = on;
    }
}

/// No beam while the weapon's not ours to use (a menu, dead, down, a kill
/// cam, a soul up in the pyramid...).
fn beam_off(mut weapon: ResMut<Weapon>) {
    if weapon.beaming {
        weapon.beaming = false;
    }
}

/// While the Exhumer's in hand its mag shows its souls — its only ammo —
/// with nothing in reserve.
fn sync_soul_mag(mut weapon: ResMut<Weapon>) {
    if weapon.primary != shared::weapon::WeaponId::Exhumer {
        return;
    }
    if weapon.mag != weapon.souls || weapon.reserve != 0 {
        weapon.mag = weapon.souls;
        weapon.reserve = 0;
    }
}

/// Our ask to let the souls out (`weapons::weapon`): to the server.
fn send_release(mut weapon: ResMut<Weapon>, mut tx: Query<&mut TriggerSender<ReleaseSouls>, With<GameClient>>) {
    if !std::mem::take(&mut weapon.release_request) {
        return;
    }
    if let Ok(mut tx) = tx.single_mut() {
        tx.trigger::<shared::LobbyChannel>(ReleaseSouls);
    }
}

// --- drained ------------------------------------------------------------------------------

/// On a zombie's avatar a beam has hold of: how far it's drained (0..=1,
/// the server's), how far it's shown risen (eased toward that), and when
/// (`Time::elapsed_secs`) the server last said — let go, it sinks back.
#[derive(Component)]
struct Drained {
    target: f32,
    shown: f32,
    seen: f32,
}

/// The soul-light in a drained zombie's chest.
#[derive(Component)]
struct ChestGlow {
    avatar: Entity,
}

/// The avatar of the bot `peer`, if it's here — and whether a beam's
/// already got it.
fn avatar_of(
    peer: PeerId,
    avatars: &Query<(Entity, &RemoteAvatar, Has<Drained>)>,
    ids: &Query<&PlayerId>,
    interp: &Query<&Interpolated>,
) -> Option<(Entity, bool)> {
    avatars
        .iter()
        .find(|(_, a, _)| crate::boss::peer_of(a.src, ids, interp) == Some(peer))
        .map(|(e, _, drained)| (e, drained))
}

#[allow(clippy::too_many_arguments)]
fn receive_drains(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    sounds: Res<ExhumerSounds>,
    mut receivers: Query<&mut MessageReceiver<ExhumerDrain>>,
    avatars: Query<(Entity, &RemoteAvatar, Has<Drained>)>,
    mut drained: Query<&mut Drained>,
    transforms: Query<&Transform, With<RemoteAvatar>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            for (zombie, share) in msg.zombies {
                let Some((avatar, already)) = avatar_of(zombie, &avatars, &ids, &interp) else { continue };
                if already {
                    if let Ok(mut d) = drained.get_mut(avatar) {
                        d.target = d.target.max(share);
                        d.seen = now;
                    }
                    continue;
                }
                // Taken hold of: it starts to rise, a glow in its chest.
                commands.entity(avatar).insert(Drained { target: share, shown: 0.0, seen: now });
                let c = settings.color;
                commands.spawn((
                    StateScoped(AppState::InGame),
                    ChestGlow { avatar },
                    PointLight {
                        color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
                        intensity: 0.0,
                        range: 4.0,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::default(),
                ));
                if let Ok(tf) = transforms.get(avatar) {
                    sound_at(&mut commands, &sounds.grab, tf.translation + Vec3::Y, settings.volume);
                }
            }
        }
    }
}

/// Every drained body, over where its pose puts it: risen as far as it's
/// drained, turning and tipping back — sinking back down once let go (a
/// ragdoll goes its own way).
#[allow(clippy::type_complexity)]
fn lift_avatars(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    mut drained: Query<(Entity, &mut Drained, &mut Transform), (With<RemoteAvatar>, Without<crate::ragdoll::Ragdoll>)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    for (e, mut d, mut tf) in &mut drained {
        // (Let go a moment: back down.)
        if now - d.seen > 0.45 {
            d.target = 0.0;
        }
        let step = dt * 3.0;
        d.shown += (d.target - d.shown).clamp(-step, step);
        if d.target == 0.0 && d.shown <= 0.0 {
            commands.entity(e).remove::<Drained>();
            continue;
        }
        let up = ease(d.shown);
        tf.translation.y += LIFT_HEIGHT * up + (now * 5.0).sin() * 0.05 * up;
        let spin = settings.lift_spin * now * up;
        let tilt = settings.lift_tilt_deg.to_radians() * up;
        tf.rotation = Quat::from_rotation_y(spin) * tf.rotation * Quat::from_rotation_x(-tilt);
    }
}

/// The glow in each drained chest, as bright as it's drained.
fn run_glows(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    avatars: Query<(&Transform, Option<&Drained>), (With<RemoteAvatar>, Without<ChestGlow>)>,
    mut glows: Query<(Entity, &ChestGlow, &mut Transform, &mut PointLight)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (e, glow, mut tf, mut light) in &mut glows {
        let Some((body, Some(d))) = avatars.get(glow.avatar).ok() else {
            commands.entity(e).despawn();
            continue;
        };
        tf.translation = body.translation + Vec3::Y * 1.25;
        let flicker = 0.8 + 0.2 * (now * 27.0 + e.index() as f32).sin();
        light.intensity = settings.chest_light * d.shown.powi(2) * flicker;
    }
}

// --- the beams ----------------------------------------------------------------------------

/// A beam being drawn: whose (`None`: ours), where it starts, the way it
/// goes and how far (m) — this frame — and motes owed (fractional).
#[derive(Component)]
struct BeamVisual {
    peer: Option<PeerId>,
    from: Vec3,
    dir: Vec3,
    len: f32,
    owed: f32,
}

/// A beam's glowing core, and its light under it.
#[derive(Component)]
struct BeamCore;

/// A mote drawn back along a beam: how far along it (m from the gun).
#[derive(Component)]
struct Mote {
    beam: Entity,
    along: f32,
    spin: f32,
    seed: f32,
}

#[derive(Resource)]
struct BeamAssets {
    core: Handle<Mesh>,
    core_material: Handle<StandardMaterial>,
    mote_material: Handle<StandardMaterial>,
}

/// Each beam that's on, drawn from its gun out to its reach (or the first
/// wall), with its light and hum — gone when it's let go.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw_beams(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    weapon: Res<Weapon>,
    sounds: Res<ExhumerSounds>,
    fx: Option<Res<ExplosionAssets>>,
    assets: Option<Res<BeamAssets>>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    (local, poses): (Query<&LocalId, With<GameClient>>, Query<(&PlayerId, &PlayerPose), With<Interpolated>>),
    rapier: ReadRapierContext,
    mut beams: Query<(Entity, &mut BeamVisual, &mut Transform, &Children), Without<BeamCore>>,
    mut cores: Query<(&mut Transform, Option<&mut PointLight>), (With<BeamCore>, Without<BeamVisual>)>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    mut commands: Commands,
) {
    let s = &*settings;
    let Some(fx) = fx else { return };
    let Some(assets) = assets else {
        let [r, g, b] = s.color;
        commands.insert_resource(BeamAssets {
            core: meshes.add(Cylinder::new(1.0, 1.0)),
            core_material: materials.add(StandardMaterial {
                base_color: Color::srgba(r, g, b, 0.5),
                emissive: LinearRgba::rgb(r * s.beam_glow, g * s.beam_glow, b * s.beam_glow),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                ..default()
            }),
            mote_material: materials.add(StandardMaterial {
                base_color: LinearRgba::rgb(r * 3.0, g * 3.0, b * 3.0).into(),
                base_color_texture: Some(fx.glow.clone()),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                double_sided: true,
                cull_mode: None,
                ..default()
            }),
        });
        return;
    };
    let me = local.iter().next().map(|l| l.0);
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    // Every beam that's on now: ours from the gun, theirs from where theirs
    // would be.
    let mut on: Vec<(Option<PeerId>, Vec3, Vec3)> = Vec::new();
    if weapon.beaming {
        let from = cam.translation() + cam.right() * 0.14 - cam.up() * 0.13 + cam.forward() * 0.45;
        on.push((None, from, cam.forward().as_vec3()));
    }
    for (id, pose) in &poses {
        if pose.beaming && Some(id.0) != me {
            let dir = beam_dir(pose.yaw, pose.pitch);
            let right = Quat::from_rotation_y(pose.yaw) * Vec3::X;
            on.push((Some(id.0), pose.translation - Vec3::Y * 0.25 + right * 0.2 + dir * 0.5, dir));
        }
    }
    for (e, beam, ..) in &beams {
        if !on.iter().any(|(p, ..)| *p == beam.peer) {
            commands.entity(e).despawn();
        }
    }
    let ctx = rapier.single().ok();
    let flicker = 1.0 - s.beam_flicker * (0.5 + 0.5 * (time.elapsed_secs() * 31.0).sin());
    for (peer, from, dir) in on {
        let len = ctx
            .as_ref()
            .and_then(|r| r.cast_ray(from, dir, BEAM_RANGE, true, QueryFilter::only_fixed()))
            .map_or(BEAM_RANGE, |(_, d)| d.max(0.1));
        let Some((_, mut beam, mut root, children)) = beams.iter_mut().find(|(_, b, ..)| b.peer == peer) else {
            // A new one: its core (and light), its hum — ours not
            // positional, theirs from where it starts.
            let mut hum = match peer {
                None => PlaybackSettings::LOOP,
                Some(_) => crate::positional_playback(Volume::Linear(s.beam_volume)),
            };
            hum.mode = crate::audio_engine::PlaybackMode::Loop;
            hum.volume = Volume::Linear(s.beam_volume);
            let c = s.color;
            commands
                .spawn((
                    StateScoped(AppState::InGame),
                    BeamVisual { peer, from, dir, len, owed: 0.0 },
                    Transform::from_translation(from),
                    Visibility::default(),
                ))
                .with_children(|b| {
                    b.spawn((
                        BeamCore,
                        Mesh3d(assets.core.clone()),
                        MeshMaterial3d(assets.core_material.clone()),
                        Transform::from_scale(Vec3::ZERO),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                    b.spawn((
                        BeamCore,
                        PointLight {
                            color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
                            intensity: s.beam_light,
                            range: 6.0,
                            shadows_enabled: false,
                            ..default()
                        },
                        Transform::default(),
                    ));
                    b.spawn((AudioPlayer::new(sounds.beam.clone()), hum, Transform::default()));
                });
            continue;
        };
        beam.from = from;
        beam.dir = dir;
        beam.len = len;
        beam.owed += s.motes_per_sec * time.delta_secs();
        root.translation = from;
        // (A cylinder stands along its Y: stretched along the beam, its
        // middle half way out; the light half way out too.)
        let turn = Quat::from_rotation_arc(Vec3::Y, dir);
        for child in children.iter() {
            let Ok((mut tf, light)) = cores.get_mut(child) else { continue };
            match light {
                Some(mut light) => {
                    tf.translation = dir * len * 0.5;
                    light.intensity = s.beam_light * flicker;
                }
                None => {
                    let w = s.beam_width * flicker;
                    *tf = Transform::from_translation(dir * len * 0.5)
                        .with_rotation(turn)
                        .with_scale(Vec3::new(w, len, w));
                }
            }
        }
    }
    let mut spawn: Vec<(Entity, f32)> = Vec::new();
    for (e, mut beam, ..) in &mut beams {
        while beam.owed >= 1.0 {
            beam.owed -= 1.0;
            spawn.push((e, beam.len));
        }
    }
    for (i, (beam, len)) in spawn.into_iter().enumerate() {
        let seed = crate::util::rand01((time.elapsed_secs() * 1000.0) as u32 ^ (i as u32).wrapping_mul(2_654_435_761));
        commands.spawn((
            StateScoped(AppState::InGame),
            Mote { beam, along: len * (0.6 + 0.4 * seed), spin: seed * std::f32::consts::TAU, seed },
            Mesh3d(fx.quad.clone()),
            MeshMaterial3d(assets.mote_material.clone()),
            Transform::from_scale(Vec3::ZERO),
            NotShadowCaster,
            NoFrustumCulling,
        ));
    }
}

/// The motes on each beam, drawn back along it into the gun, spiralling in
/// — gone once there (or with their beam).
fn fly_motes(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    beams: Query<&BeamVisual>,
    mut motes: Query<(Entity, &mut Mote, &mut Transform)>,
    mut commands: Commands,
) {
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    let s = &*settings;
    let dt = time.delta_secs();
    for (e, mut mote, mut tf) in &mut motes {
        let Ok(beam) = beams.get(mote.beam) else {
            commands.entity(e).despawn();
            continue;
        };
        mote.along -= s.mote_speed * (0.7 + 0.6 * mote.seed) * dt;
        mote.spin += dt * 9.0;
        if mote.along <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let side = beam.dir.any_orthonormal_vector();
        let up = beam.dir.cross(side);
        let r = 0.25 * (mote.along / beam.len.max(0.1)).min(1.0);
        let at = beam.from + beam.dir * mote.along.min(beam.len) + (side * mote.spin.cos() + up * mote.spin.sin()) * r;
        let size = s.mote_size * (0.6 + 0.4 * mote.seed);
        *tf = Transform::from_translation(at)
            .with_rotation(smoke_billboard_rotation(at, cam_pos, cam_up, cam_right, mote.spin))
            .with_scale(Vec3::new(size, size, 1.0));
    }
}

// --- souls in flight --------------------------------------------------------------------

/// Where a wisp's going.
#[derive(Clone, Copy)]
enum WispTo {
    /// Back into this player's Exhumer.
    Shooter(PeerId),
    /// Into this zombie (let out after it).
    Zombie(PeerId),
}

/// A soul in flight: from `from` to its target over `secs`, bowing to one
/// side (`side`); `last` is where its target was last seen.
#[derive(Component)]
struct Wisp {
    from: Vec3,
    to: WispTo,
    last: Vec3,
    side: Vec3,
    age: f32,
    secs: f32,
}

/// One glow of a wisp — its head (`0`) or a trailing one.
#[derive(Component)]
struct WispPart(u32);

#[allow(clippy::too_many_arguments)]
fn spawn_wisp(
    commands: &mut Commands,
    fx: &ExplosionAssets,
    material: &Handle<StandardMaterial>,
    s: &ExhumerFxSettings,
    from: Vec3,
    to: WispTo,
    last: Vec3,
    secs: f32,
    seed: u32,
) {
    let r = |k: u32| crate::util::rand01(seed.wrapping_mul(2_654_435_761).wrapping_add(k)) * 2.0 - 1.0;
    let side = Vec3::new(r(1), r(2) * 0.4, r(3)).normalize_or(Vec3::X);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            Wisp { from, to, last, side, age: 0.0, secs: secs.max(0.05) },
            Transform::from_translation(from),
            Visibility::default(),
        ))
        .with_children(|w| {
            for i in 0..=s.trail {
                w.spawn((
                    WispPart(i),
                    Mesh3d(fx.quad.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            }
        });
}

/// The wisps' glow, made the first time it's needed.
fn wisp_material(
    material: &mut Option<Handle<StandardMaterial>>,
    fx: &ExplosionAssets,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: LinearRgba::rgb(0.0, 0.0, 0.0).into(),
                base_color_texture: Some(fx.glow.clone()),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .clone()
}

/// Souls torn out: each streaks back into its shooter's Exhumer — and ours
/// holds as many as the server says.
#[allow(clippy::too_many_arguments)]
fn receive_souls(
    settings: Res<ExhumerFxSettings>,
    sounds: Res<ExhumerSounds>,
    fx: Option<Res<ExplosionAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
    mut receivers: Query<&mut MessageReceiver<ExhumerSouls>>,
    local: Query<&LocalId, With<GameClient>>,
    mut weapon: ResMut<Weapon>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if Some(msg.shooter) == me && weapon.souls != msg.souls {
                weapon.souls = msg.souls;
            }
            if let Some(first) = msg.from.first() {
                sound_at(&mut commands, &sounds.tear, Vec3::from_array(*first), settings.volume);
            }
            let Some(fx) = fx.as_deref() else { continue };
            let mat = wisp_material(&mut material, fx, &mut materials);
            for from in &msg.from {
                *seq = seq.wrapping_add(1);
                let from = Vec3::from_array(*from);
                let to = WispTo::Shooter(msg.shooter);
                spawn_wisp(&mut commands, fx, &mat, &settings, from, to, from, settings.return_secs, *seq);
            }
        }
    }
}

/// The souls let out: one after each zombie the server sent one for.
#[allow(clippy::too_many_arguments)]
fn receive_swarms(
    settings: Res<ExhumerFxSettings>,
    sounds: Res<ExhumerSounds>,
    fx: Option<Res<ExplosionAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
    mut receivers: Query<&mut MessageReceiver<ExhumerSwarm>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let origin = Vec3::from_array(msg.origin);
            sound_at(&mut commands, &sounds.release, origin, settings.volume);
            let Some(fx) = fx.as_deref() else { continue };
            let mat = wisp_material(&mut material, fx, &mut materials);
            for (zombie, at) in &msg.targets {
                *seq = seq.wrapping_add(1);
                let at = Vec3::from_array(*at);
                let secs = origin.distance(at) / SWARM_SPEED + 0.2;
                spawn_wisp(&mut commands, fx, &mat, &settings, origin, WispTo::Zombie(*zombie), at, secs, *seq);
            }
        }
    }
}

/// Every wisp on its way: arcing from where it left to where its target is
/// now, its tail behind it — gone once it's there.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fly_wisps(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    local: Query<&LocalId, With<GameClient>>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    poses: Query<(&PlayerId, &PlayerPose), With<Interpolated>>,
    mut wisps: Query<(Entity, &mut Wisp, &mut Transform, &Children), Without<WispPart>>,
    mut parts: Query<(&WispPart, &mut Transform, &MeshMaterial3d<StandardMaterial>), Without<Wisp>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    let me = local.iter().next().map(|l| l.0);
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    let s = &*settings;
    let dt = time.delta_secs();
    let pose_of = |peer: PeerId| poses.iter().find(|(id, _)| id.0 == peer).map(|(_, p)| p.translation);
    let [r, g, b] = s.color;
    let glow: Color = LinearRgba::rgb(r * s.wisp_glow, g * s.wisp_glow, b * s.wisp_glow).into();
    for (e, mut wisp, mut root, children) in &mut wisps {
        wisp.age += dt;
        // Where its target is now.
        let target = match wisp.to {
            WispTo::Shooter(p) if Some(p) == me => Some(cam_pos + cam.forward() * 0.6 - cam_up * 0.25),
            WispTo::Shooter(p) => pose_of(p).map(|eye| eye - Vec3::Y * 0.35),
            WispTo::Zombie(p) => pose_of(p).map(|eye| eye - Vec3::Y * (EYE_HEIGHT - 1.2)),
        };
        if let Some(t) = target {
            wisp.last = t;
        }
        if wisp.age >= wisp.secs {
            commands.entity(e).despawn();
            continue;
        }
        let (from, to, side, secs) = (wisp.from, wisp.last, wisp.side, wisp.secs);
        let at = |age: f32| {
            let u = ease(age / secs);
            let mid = from.lerp(to, 0.5) + Vec3::Y * s.arc + side * s.arc * 0.6;
            from.lerp(mid, u).lerp(mid.lerp(to, u), u)
        };
        let head = at(wisp.age);
        root.translation = head;
        for child in children.iter() {
            let Ok((part, mut tf, material)) = parts.get_mut(child) else { continue };
            let age = wisp.age - part.0 as f32 * s.trail_lag;
            if age < 0.0 {
                tf.scale = Vec3::ZERO;
                continue;
            }
            let pos = at(age);
            let k = 1.0 - part.0 as f32 / (s.trail as f32 + 1.0);
            let size = (s.wisp_size * k * (0.7 + 0.3 * (1.0 - wisp.age / secs))).max(1.0e-4);
            *tf = Transform::from_translation(pos - head)
                .with_rotation(smoke_billboard_rotation(pos, cam_pos, cam_up, cam_right, age * 3.0))
                .with_scale(Vec3::new(size, size, 1.0));
            // (Only touched when it's off — so it's not marked changed every
            // frame.)
            if materials.get(&material.0).is_some_and(|m| m.base_color != glow) {
                if let Some(m) = materials.get_mut(&material.0) {
                    m.base_color = glow;
                }
            }
        }
    }
}

// --- the debug panel --------------------------------------------------------------------

/// Between the debug panel's "The Exhumer" section and the game: what it
/// shows of ours, and its ask to be given one ([`debug_give`]). (The panel's
/// system already holds `Weapon` and a `GiveWeapon` sender elsewhere.)
#[derive(Resource, Default)]
pub(crate) struct ExhumerDebug {
    give: bool,
    souls: u32,
    beaming: bool,
    carried: bool,
}

/// The panel's ask to be given the Exhumer (in place of the gun in hand,
/// like a wall buy, free) — and what it shows, kept up to date.
fn debug_give(
    mut debug: ResMut<ExhumerDebug>,
    weapon: Res<Weapon>,
    mut tx: Query<&mut TriggerSender<shared::GiveWeapon>, With<GameClient>>,
) {
    let carried = weapon.carries(shared::weapon::SlotWeapon::Gun(shared::weapon::WeaponId::Exhumer));
    if (debug.souls, debug.beaming, debug.carried) != (weapon.souls, weapon.beaming, carried) {
        (debug.souls, debug.beaming, debug.carried) = (weapon.souls, weapon.beaming, carried);
    }
    if !std::mem::take(&mut debug.give) || carried {
        return;
    }
    let (mag, reserve) = weapon.held_ammo();
    if let Ok(mut tx) = tx.single_mut() {
        tx.trigger::<shared::LobbyChannel>(shared::GiveWeapon {
            weapon: shared::weapon::WeaponId::Exhumer,
            slot: weapon.held as u8,
            mag,
            reserve,
        });
    }
}

/// The debug panel's controls for it.
pub(crate) fn exhumer_section(ui: &mut egui::Ui, s: &mut ExhumerFxSettings, debug: &mut ExhumerDebug, in_zombies: bool) {
    ui.label(format!(
        "souls: {}/{}  ·  beam {}",
        debug.souls,
        shared::exhumer::MAX_SOULS,
        if debug.beaming { "on" } else { "off" }
    ));
    if ui
        .add_enabled(in_zombies && !debug.carried, egui::Button::new("Give me the Exhumer (in place of the gun in hand)"))
        .clicked()
    {
        debug.give = true;
    }
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.color);
        ui.label("souls' colour (the beam's from the next game)");
    });
    ui.add(egui::Slider::new(&mut s.beam_width, 0.01f32..=0.5).text("beam width (m)"));
    ui.add(egui::Slider::new(&mut s.beam_flicker, 0.0f32..=1.0).text("beam flicker"));
    ui.add(egui::Slider::new(&mut s.beam_light, 0.0f32..=1_000_000.0).text("beam light (lm)"));
    ui.add(egui::Slider::new(&mut s.motes_per_sec, 0.0f32..=200.0).text("motes drawn in (/s)"));
    ui.add(egui::Slider::new(&mut s.mote_speed, 1.0f32..=60.0).text("mote speed (m/s)"));
    ui.add(egui::Slider::new(&mut s.mote_size, 0.02f32..=1.0).text("mote size (m)"));
    ui.add(egui::Slider::new(&mut s.wisp_size, 0.05f32..=2.0).text("soul wisp size (m)"));
    ui.add(egui::Slider::new(&mut s.wisp_glow, 0.0f32..=20.0).text("soul wisp glow"));
    ui.add(egui::Slider::new(&mut s.trail, 0..=20).text("wisp tail glows (next ones)"));
    ui.add(egui::Slider::new(&mut s.trail_lag, 0.005f32..=0.2).text("wisp tail spacing (s)"));
    ui.add(egui::Slider::new(&mut s.return_secs, 0.1f32..=3.0).text("soul back to the gun (s)"));
    ui.add(egui::Slider::new(&mut s.arc, 0.0f32..=6.0).text("soul's arc (m)"));
    ui.add(egui::Slider::new(&mut s.lift_spin, 0.0f32..=6.0).text("drained body turns (rad/s)"));
    ui.add(egui::Slider::new(&mut s.lift_tilt_deg, 0.0f32..=90.0).text("drained body tips back (°)"));
    ui.add(egui::Slider::new(&mut s.chest_light, 0.0f32..=1_000_000.0).text("glow in its chest (lm)"));
    ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=3.0).text("sounds' volume"));
    ui.add(egui::Slider::new(&mut s.beam_volume, 0.0f32..=3.0).text("beam hum volume (next time it's on)"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = ExhumerFxSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
