//! The Hierophant wakes ([`EggStep::Awakening`] — its timings are
//! `shared::easter_egg`'s `WAKE_*`, the server moving on to the fight once
//! it's done):
//!
//! 1. the sarcophagus' lid blasts off — tumbling up and away to land beside
//!    it ([`lid_pose`]) — with a bang, a flash, a jolt of the camera and
//!    every light stuttering;
//! 2. his spirit pours down out of the dark into the body lying there — a
//!    spiral of glowing wisps, a green glow growing over it, the body
//!    twitching as they go in;
//! 3. stillness: the glow cut, the lights steady, nothing;
//! 4. he snaps upright, eyes lit, screaming — a hard hit, the camera
//!    shaken hard (less the further off we are), the lights stuttering
//!    again.
//!
//! The body's a placeholder of plain boxes ([`WakeFx`]) — robed, gaunt,
//! lying in the sarcophagus — and stays standing there once the fight's on
//! until the Hierophant himself is in, standing where it stood (`fight`). The lid lies
//! where it landed from then on, back on the sarcophagus once the egg's
//! reset. Its sounds are placeholders at their final paths
//! (`audio/zombies/easter_egg/awakening/`, `hierophant/wake.mp3`).
//!
//! The clock ([`EggWake`]), the camera's shake ([`EggCameraShake`]) and
//! which sounds have played are reset on leaving the game; the body,
//! wisps, lights and sounds are `StateScoped(InGame)`, and gone (or back)
//! as soon as the egg isn't there. Tuned in the debug panel's "Easter egg
//! (Der Tempel)" → "He wakes".

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::easter_egg::{
    EggStep, AWAKENING_SECS, WAKE_LID_SECS, WAKE_SNAP_AT, WAKE_SPIRIT_FROM, WAKE_SPIRIT_TO,
};
use shared::Lobby;

use crate::audio_engine::prelude::*;
use crate::net::GameClient;
use crate::{AppState, Player};

use super::{egg_game, ramp};

pub(super) struct WakePlugin;

impl Plugin for WakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggWake>()
            .init_resource::<EggWakeSettings>()
            .init_resource::<EggCameraShake>()
            .add_systems(Startup, (make_assets, load_sounds))
            .add_systems(OnExit(AppState::InGame), |mut wake: ResMut<EggWake>, mut shake: ResMut<EggCameraShake>| {
                *wake = default();
                *shake = default();
            })
            .add_systems(
                Update,
                (track_wake, sync_wake_fx, animate_wake, animate_lid, wake_cues, wake_shake)
                    .chain()
                    .run_if(in_state(AppState::InGame))
                    .before(crate::camera_shake),
            );
    }
}

/// How far (s) into the wake-up we are — `None` before it; once he's past
/// it (the fight and on) as it ended. Held while the game's paused.
#[derive(Resource, Default)]
pub(crate) struct EggWake {
    t: Option<f32>,
    /// Which of its moments ([`Cue`]) have had their sound and jolt, this
    /// wake-up.
    cued: u8,
}

/// The camera's shake from the wake-up right now — `weapons::camera_shake`
/// layers it in, under everything else.
#[derive(Resource)]
pub(crate) struct EggCameraShake(pub(crate) Transform);

impl Default for EggCameraShake {
    fn default() -> Self {
        Self(Transform::IDENTITY)
    }
}

/// Panel-tunable wake-up ("He wakes").
#[derive(Resource, Clone, Debug)]
pub(crate) struct EggWakeSettings {
    /// The sounds' volumes (linear, before the mix).
    pub(crate) blast_volume: f32,
    pub(crate) spirit_volume: f32,
    pub(crate) scream_volume: f32,
    /// The hard hit as he snaps up — not positional: it's a sting.
    pub(crate) hit_volume: f32,
    /// How hard the camera shakes as the lid blasts off, and as he snaps up
    /// (0..1 of the most)...
    pub(crate) lid_shake: f32,
    pub(crate) snap_shake: f32,
    /// ...the most: how far (m) it jolts and how far (°) it tilts...
    pub(crate) shake_pos_m: f32,
    pub(crate) shake_rot_deg: f32,
    /// ...how fast (Hz)...
    pub(crate) shake_hz: f32,
    /// ...and how it fades off with distance: full within this far (m) of
    /// the sarcophagus, down to `shake_far_min` of it this far off.
    pub(crate) shake_full_within: f32,
    pub(crate) shake_far: f32,
    pub(crate) shake_far_min: f32,
    /// How high (m) the lid flies and how far out to the side it lands.
    pub(crate) lid_height: f32,
    pub(crate) lid_throw: f32,
    /// The glow over the body as his spirit pours in, and the flashes as
    /// the lid goes and he snaps up (lumens).
    pub(crate) spirit_light: f32,
    pub(crate) flash_light: f32,
}

impl Default for EggWakeSettings {
    fn default() -> Self {
        Self {
            blast_volume: 1.5,
            spirit_volume: 1.5,
            scream_volume: 2.5,
            hit_volume: 1.0,
            lid_shake: 0.35,
            snap_shake: 1.0,
            shake_pos_m: 0.05,
            shake_rot_deg: 3.0,
            shake_hz: 22.0,
            shake_full_within: 12.0,
            shake_far: 45.0,
            shake_far_min: 0.3,
            lid_height: 2.6,
            lid_throw: 2.4,
            spirit_light: 150_000.0,
            flash_light: 600_000.0,
        }
    }
}

// --- the clock ------------------------------------------------------------------------

fn track_wake(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut wake: ResMut<EggWake>,
) {
    let step = egg_game(&local, &lobbies).map(|l| l.easter_egg.step);
    match step {
        Some(EggStep::Awakening) => {
            let t = wake.t.get_or_insert(0.0);
            if !paused.0 {
                *t += time.delta_secs();
            }
        }
        // (Past it: as it ended — but no sounds.)
        Some(s) if s > EggStep::Awakening => {
            if wake.t.is_none() {
                wake.cued = u8::MAX;
            }
            wake.t = Some(AWAKENING_SECS);
        }
        _ => {
            if wake.t.is_some() || wake.cued != 0 {
                *wake = default();
            }
        }
    }
}

// --- the lid --------------------------------------------------------------------------

/// The lid's motion `t` s into the wake-up, in the sarcophagus' own frame,
/// to put on top of where it sits ([`super::LidRest`]): blasted up and out
/// to its +x side, tumbling, then lying on the ground there, tipped.
pub(super) fn lid_pose(t: f32, rest: Vec3, settings: &EggWakeSettings) -> Transform {
    let s = ramp(t, 0.0, WAKE_LID_SECS);
    // (Where it ends up: flat on the ground beside it, turned a bit.)
    let landed = Vec3::new(0.55 + settings.lid_throw, 0.075, 0.25);
    let start = rest;
    let across = start.lerp(landed, s);
    let arc = settings.lid_height * 4.0 * s * (1.0 - s);
    // (Tumbling over once as it goes, landing turned a little.)
    let spin = Quat::from_euler(EulerRot::YXZ, 0.45 * s, 0.6 * (s * PI).sin(), -TAU * s);
    Transform::from_translation(across + Vec3::Y * arc).with_rotation(spin)
}

// --- the body, wisps and lights -------------------------------------------------------

#[derive(Resource)]
struct WakeAssets {
    robe: (Handle<Mesh>, Handle<StandardMaterial>),
    head: (Handle<Mesh>, Handle<StandardMaterial>),
    arm: Handle<Mesh>,
    skin: Handle<StandardMaterial>,
    eye: (Handle<Mesh>, Handle<StandardMaterial>),
    wisp: (Handle<Mesh>, Handle<StandardMaterial>),
}

fn make_assets(mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, mut commands: Commands) {
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.38, 0.3),
        perceptual_roughness: 0.85,
        ..default()
    });
    commands.insert_resource(WakeAssets {
        robe: (
            meshes.add(Cuboid::new(0.45, 1.5, 0.28)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.12, 0.08, 0.06),
                perceptual_roughness: 0.95,
                ..default()
            }),
        ),
        head: (meshes.add(Sphere::new(0.13)), skin.clone()),
        arm: meshes.add(Cuboid::new(0.08, 0.8, 0.08)),
        skin,
        eye: (
            meshes.add(Sphere::new(0.025)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.4, 1.0, 0.45),
                emissive: LinearRgba::rgb(4.0, 30.0, 6.0),
                unlit: true,
                ..default()
            }),
        ),
        wisp: (
            meshes.add(Sphere::new(0.09)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.75, 1.0, 0.8),
                emissive: LinearRgba::rgb(3.0, 14.0, 5.0),
                unlit: true,
                ..default()
            }),
        ),
    });
}

/// How many wisps his spirit comes down as, and how long (s) each takes.
const WISPS: usize = 18;
const WISP_TRAVEL: f32 = 1.3;
/// How high (m) over the body they come from.
const WISP_HEIGHT: f32 = 8.0;

/// The wake-up's placeholder body, wisps and lights, under one root in the
/// sarcophagus' own frame.
#[derive(Component)]
struct WakeFx;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum WakePart {
    /// The body, pivoting at its feet — lying until he snaps up.
    Body,
    Eyes,
    Wisp(usize),
    SpiritLight,
    Flash,
}

/// The body's feet (the pivot it snaps up on), lying along -z from there
/// on top of the sarcophagus' chest (`super::make_placeholders`) — where
/// the fight's Hierophant starts, too.
const FEET: Vec3 = shared::easter_egg::HIEROPHANT_FEET;
/// Lying, face up.
fn lying() -> Quat {
    Quat::from_rotation_x(-FRAC_PI_2)
}

/// The body's there while he's waking, and into the fight until he's in
/// (`EasterEgg::hierophant`), on a map with the sarcophagus placed.
fn sync_wake_fx(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    assets: Res<WakeAssets>,
    roots: Query<Entity, With<WakeFx>>,
    mut root_at: Query<&mut Transform, With<WakeFx>>,
    mut commands: Commands,
) {
    let at = egg_game(&local, &lobbies)
        .filter(|l| match l.easter_egg.step {
            EggStep::Awakening => true,
            EggStep::BossFight => l.easter_egg.hierophant.is_none(),
            _ => false,
        })
        .and_then(|l| shared::level::layout(l.map).sarcophagus);
    let Some(at) = at else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    if !roots.is_empty() {
        for mut t in &mut root_at {
            t.set_if_neq(crate::util::placed(at));
        }
        return;
    }
    let a = &*assets;
    commands
        .spawn((StateScoped(AppState::InGame), WakeFx, crate::util::placed(at), Visibility::default()))
        .with_children(|fx| {
            fx.spawn((WakePart::Body, Transform::from_translation(FEET).with_rotation(lying()), Visibility::default()))
                .with_children(|body| {
                    body.spawn((Mesh3d(a.robe.0.clone()), MeshMaterial3d(a.robe.1.clone()), Transform::from_xyz(0.0, 0.75, 0.0)));
                    body.spawn((Mesh3d(a.head.0.clone()), MeshMaterial3d(a.head.1.clone()), Transform::from_xyz(0.0, 1.64, 0.0)));
                    for side in [-1.0, 1.0] {
                        body.spawn((
                            Mesh3d(a.arm.clone()),
                            MeshMaterial3d(a.skin.clone()),
                            Transform::from_xyz(0.29 * side, 1.02, 0.0),
                        ));
                    }
                    body.spawn((WakePart::Eyes, Transform::IDENTITY, Visibility::Hidden)).with_children(|eyes| {
                        for side in [-1.0, 1.0] {
                            eyes.spawn((
                                Mesh3d(a.eye.0.clone()),
                                MeshMaterial3d(a.eye.1.clone()),
                                Transform::from_xyz(0.045 * side, 1.67, 0.115),
                                NotShadowCaster,
                            ));
                        }
                    });
                });
            for i in 0..WISPS {
                fx.spawn((
                    WakePart::Wisp(i),
                    Mesh3d(a.wisp.0.clone()),
                    MeshMaterial3d(a.wisp.1.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    NotShadowCaster,
                ));
            }
            for part in [WakePart::SpiritLight, WakePart::Flash] {
                fx.spawn((
                    part,
                    PointLight {
                        color: Color::srgb(0.45, 1.0, 0.5),
                        intensity: 0.0,
                        range: 14.0,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::default(),
                ));
            }
        });
}

/// When (s into the wake-up) wisp `i` starts down.
fn wisp_start(i: usize) -> f32 {
    let span = (WAKE_SPIRIT_TO - WAKE_SPIRIT_FROM - WISP_TRAVEL).max(0.1);
    WAKE_SPIRIT_FROM + span * i as f32 / (WISPS - 1).max(1) as f32
}

/// How far (0..1) he's snapped up, `t` s into the wake-up — fast, and
/// overshooting a touch before he settles.
fn snap(t: f32) -> f32 {
    let x = ramp(t, WAKE_SNAP_AT, WAKE_SNAP_AT + 0.17);
    let c = 2.2;
    1.0 + (c + 1.0) * (x - 1.0).powi(3) + c * (x - 1.0).powi(2)
}

/// Play the wake-up out on the body, wisps and lights by its clock.
#[allow(clippy::type_complexity)]
fn animate_wake(
    time: Res<Time>,
    wake: Res<EggWake>,
    settings: Res<EggWakeSettings>,
    mut parts: Query<(&WakePart, &mut Transform, Option<&mut Visibility>), Without<PointLight>>,
    // (Apart from `parts` by the light.)
    mut lights: Query<(&WakePart, &mut PointLight, &mut Transform)>,
) {
    let Some(t) = wake.t else { return };
    let now = time.elapsed_secs();
    // The body's chest, lying.
    let chest = FEET + lying() * Vec3::Y * 1.0;
    for (part, mut transform, vis) in &mut parts {
        match *part {
            WakePart::Body => {
                // A twitch now and then as the wisps go in...
                let twitch = if (WAKE_SPIRIT_FROM + 1.5..WAKE_SPIRIT_TO).contains(&t) {
                    let k = ((t * 7.0).floor() as u32).wrapping_mul(2_654_435_761);
                    if crate::util::rand01(k) > 0.6 { (crate::util::rand01(k ^ 0x51) - 0.5) * 0.12 } else { 0.0 }
                } else {
                    0.0
                };
                // ...then up, and breathing, his head lolling a little.
                let up = snap(t);
                let breathe = if t >= WAKE_SNAP_AT + 0.4 { (now * 1.7).sin() * 0.015 } else { 0.0 };
                let rotation = Quat::from_rotation_x(-FRAC_PI_2 * (1.0 - up) + twitch + breathe)
                    * Quat::from_rotation_z(twitch * 0.5);
                *transform = Transform::from_translation(FEET).with_rotation(rotation);
            }
            WakePart::Eyes => {
                if let Some(mut vis) = vis {
                    vis.set_if_neq(if t >= WAKE_SNAP_AT { Visibility::Inherited } else { Visibility::Hidden });
                }
            }
            WakePart::Wisp(i) => {
                let s = (t - wisp_start(i)) / WISP_TRAVEL;
                let shown = (0.0..1.0).contains(&s);
                if let Some(mut vis) = vis {
                    vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
                }
                if !shown {
                    continue;
                }
                let left = 1.0 - s;
                let angle = i as f32 * 2.4 + s * 9.0;
                let r = 1.6 * left.powf(0.8);
                let pos = chest + Vec3::new(angle.cos() * r, WISP_HEIGHT * left.powf(1.6), angle.sin() * r);
                let size = (s * PI).sin().max(0.0).sqrt() * (0.6 + 0.4 * left);
                *transform = Transform::from_translation(pos).with_scale(Vec3::splat(size.max(0.001)));
            }
            _ => {}
        }
    }
    for (part, mut light, mut transform) in &mut lights {
        match part {
            WakePart::SpiritLight => {
                // Growing as they pour in — then cut dead for the stillness.
                let glow = if t < WAKE_SPIRIT_TO { ramp(t, WAKE_SPIRIT_FROM, WAKE_SPIRIT_TO) } else { 0.0 };
                light.intensity = settings.spirit_light * glow * (0.85 + 0.15 * (now * 13.0).sin());
                transform.translation = chest + Vec3::Y * 0.7;
            }
            WakePart::Flash => {
                let lid = (-(t / 0.12)).exp() * 0.6;
                let up = if t >= WAKE_SNAP_AT { (-((t - WAKE_SNAP_AT) / 0.25)).exp() } else { 0.0 };
                light.intensity = settings.flash_light * lid.max(up);
                transform.translation = FEET + Vec3::Y * 1.4;
            }
            _ => {}
        }
    }
}

/// The lid's boxes, moved by the wake-up — as they sit until it, flying
/// off as it starts, lying beside the sarcophagus after.
fn animate_lid(wake: Res<EggWake>, settings: Res<EggWakeSettings>, mut lids: Query<(&super::LidRest, &mut Transform)>) {
    for (rest, mut transform) in &mut lids {
        let want = match wake.t {
            None => Transform::from_translation(rest.0),
            // (Each box keeps its place on the lid as it goes.)
            Some(t) => {
                let pose = lid_pose(t, super::LID_REST, &settings);
                Transform::from_translation(pose.translation + pose.rotation * (rest.0 - super::LID_REST))
                    .with_rotation(pose.rotation)
            }
        };
        transform.set_if_neq(want);
    }
}

// --- the sounds, jolts and shake --------------------------------------------------------

/// The wake-up's moments with a sound (and the lights stuttering).
#[derive(Clone, Copy)]
enum Cue {
    Blast,
    Spirit,
    Snap,
}

impl Cue {
    const ALL: [Cue; 3] = [Cue::Blast, Cue::Spirit, Cue::Snap];

    fn at(self) -> f32 {
        match self {
            Cue::Blast => 0.0,
            Cue::Spirit => WAKE_SPIRIT_FROM,
            Cue::Snap => WAKE_SNAP_AT,
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Resource)]
struct WakeSounds {
    blast: Handle<AudioSource>,
    spirit: Handle<AudioSource>,
    scream: Handle<AudioSource>,
    hit: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(WakeSounds {
        blast: asset_server.load("audio/zombies/easter_egg/awakening/lid_blast.mp3"),
        spirit: asset_server.load("audio/zombies/easter_egg/awakening/spirit.mp3"),
        scream: asset_server.load("audio/zombies/easter_egg/hierophant/wake.mp3"),
        hit: asset_server.load("audio/zombies/easter_egg/awakening/wake_hit.mp3"),
    });
}

/// A wake-up sound playing — stopped if the egg's reset under it.
#[derive(Component)]
struct WakeSound;

/// The spirit's sound — cut dead with its glow, for the stillness.
#[derive(Component)]
struct SpiritSound;

/// Each moment's sound as the clock passes it (once a wake-up), the lights
/// stuttering with the lid and the snap and holding steady for the
/// stillness between.
#[allow(clippy::too_many_arguments)]
fn wake_cues(
    mut wake: ResMut<EggWake>,
    settings: Res<EggWakeSettings>,
    sounds: Res<WakeSounds>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    playing: Query<Entity, With<WakeSound>>,
    spirit: Query<Entity, With<SpiritSound>>,
    mut light_cue: ResMut<crate::environment::EggLightCue>,
    mut commands: Commands,
) {
    let Some(t) = wake.t else {
        for e in &playing {
            commands.entity(e).despawn();
        }
        light_cue.steady = false;
        return;
    };
    let Some(at) = egg_game(&local, &lobbies).and_then(|l| shared::level::layout(l.map).sarcophagus) else {
        return;
    };
    let steady = (WAKE_SPIRIT_TO..WAKE_SNAP_AT).contains(&t);
    if t >= WAKE_SPIRIT_TO {
        for e in &spirit {
            commands.entity(e).despawn();
        }
    }
    if light_cue.steady != steady {
        light_cue.steady = steady;
    }
    let place = crate::util::placed(at);
    for cue in Cue::ALL {
        if t < cue.at() || wake.cued & cue.bit() != 0 {
            continue;
        }
        wake.cued |= cue.bit();
        let positional = |commands: &mut Commands, clip: &Handle<AudioSource>, local_at: Vec3, volume: f32| {
            commands
                .spawn((
                    StateScoped(AppState::InGame),
                    WakeSound,
                    AudioPlayer::new(clip.clone()),
                    Transform::from_translation(place.transform_point(local_at)),
                    crate::positional_playback(Volume::Linear(volume)),
                ))
                .id()
        };
        match cue {
            Cue::Blast => {
                positional(&mut commands, &sounds.blast, Vec3::Y * 1.0, settings.blast_volume);
                light_cue.jolt = true;
            }
            Cue::Spirit => {
                let e = positional(&mut commands, &sounds.spirit, FEET + Vec3::Y * 2.0, settings.spirit_volume);
                commands.entity(e).insert(SpiritSound);
            }
            Cue::Snap => {
                positional(&mut commands, &sounds.scream, FEET + Vec3::Y * 1.65, settings.scream_volume);
                commands.spawn((
                    StateScoped(AppState::InGame),
                    WakeSound,
                    AudioPlayer::new(sounds.hit.clone()),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(settings.hit_volume)),
                ));
                light_cue.jolt = true;
            }
        }
    }
}

/// The camera's shake: a jolt as the lid goes, a hard one as he snaps up —
/// less the further we are from the sarcophagus.
fn wake_shake(
    wake: Res<EggWake>,
    settings: Res<EggWakeSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut shake: ResMut<EggCameraShake>,
) {
    let s = &*settings;
    let at = egg_game(&local, &lobbies).and_then(|l| shared::level::layout(l.map).sarcophagus);
    let level = match (wake.t, at) {
        (Some(t), Some(at)) if t < AWAKENING_SECS && wake.cued != u8::MAX => {
            let lid = if t < 1.5 { s.lid_shake * (-t / 0.35).exp() } else { 0.0 };
            let up = if (WAKE_SNAP_AT..WAKE_SNAP_AT + 2.5).contains(&t) {
                s.snap_shake * (-(t - WAKE_SNAP_AT) / 0.5).exp()
            } else {
                0.0
            };
            let d = player.as_ref().map_or(0.0, |p| p.translation.distance(at.pos));
            let off = ramp(d, s.shake_full_within, s.shake_far.max(s.shake_full_within + 0.1));
            let reach = 1.0 + (s.shake_far_min - 1.0) * off;
            Some((lid.max(up) * reach, t))
        }
        _ => None,
    };
    let want = match level {
        Some((k, t)) if k > 1e-4 => {
            let w = t * s.shake_hz * TAU;
            let n = |a: f32, b: f32| (w * a + b).sin() * 0.6 + (w * a * 1.73 + b * 2.1).sin() * 0.4;
            let rot = s.shake_rot_deg.to_radians() * k;
            Transform::from_translation(Vec3::new(n(1.0, 0.3), n(0.87, 1.9), 0.0) * s.shake_pos_m * k)
                .with_rotation(Quat::from_euler(EulerRot::YXZ, n(0.61, 4.0) * rot * 0.5, n(0.93, 2.7) * rot, n(0.77, 5.3) * rot))
        }
        _ => Transform::IDENTITY,
    };
    if shake.0 != want {
        shake.0 = want;
    }
}

// --- the debug panel --------------------------------------------------------------------

/// The debug panel's controls for it.
pub(crate) fn wake_section(ui: &mut egui::Ui, s: &mut EggWakeSettings, wake: &EggWake) {
    match wake.t {
        Some(t) if t < AWAKENING_SECS => ui.label(format!("now: {t:.1} s in (of {AWAKENING_SECS:.1})")),
        Some(_) => ui.label("now: he's up"),
        None => ui.label("now: not yet — jump to \"It goes wrong\", then \"He wakes\", to replay it"),
    };
    ui.add(egui::Slider::new(&mut s.blast_volume, 0.0f32..=3.0).text("lid blast volume"));
    ui.add(egui::Slider::new(&mut s.spirit_volume, 0.0f32..=3.0).text("spirit volume"));
    ui.add(egui::Slider::new(&mut s.scream_volume, 0.0f32..=3.0).text("scream volume"));
    ui.add(egui::Slider::new(&mut s.hit_volume, 0.0f32..=3.0).text("hit volume"));
    ui.separator();
    ui.add(egui::Slider::new(&mut s.lid_shake, 0.0f32..=1.0).text("shake as the lid goes"));
    ui.add(egui::Slider::new(&mut s.snap_shake, 0.0f32..=1.0).text("shake as he snaps up"));
    ui.add(egui::Slider::new(&mut s.shake_pos_m, 0.0f32..=0.3).text("shake jolt (m)"));
    ui.add(egui::Slider::new(&mut s.shake_rot_deg, 0.0f32..=10.0).text("shake tilt (°)"));
    ui.add(egui::Slider::new(&mut s.shake_hz, 1.0f32..=40.0).text("shake speed (Hz)"));
    ui.add(egui::Slider::new(&mut s.shake_full_within, 0.0f32..=60.0).text("full shake within (m)"));
    ui.add(egui::Slider::new(&mut s.shake_far, 1.0f32..=150.0).text("faded out by (m)"));
    ui.add(egui::Slider::new(&mut s.shake_far_min, 0.0f32..=1.0).text("down to (× shake)"));
    ui.separator();
    ui.add(egui::Slider::new(&mut s.lid_height, 0.0f32..=8.0).text("lid flies up (m)"));
    ui.add(egui::Slider::new(&mut s.lid_throw, 0.0f32..=8.0).text("lid lands out (m)"));
    ui.add(egui::Slider::new(&mut s.spirit_light, 0.0f32..=1_000_000.0).text("spirit glow (lm)"));
    ui.add(egui::Slider::new(&mut s.flash_light, 0.0f32..=3_000_000.0).text("flashes (lm)"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = EggWakeSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
