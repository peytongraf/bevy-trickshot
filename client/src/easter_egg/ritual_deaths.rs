//! Every zombie dropping dead as the ritual goes wrong
//! ([`EggStep::Unleashed`] — the server drops them all at once,
//! `shared::easter_egg::UNLEASHED_ZOMBIES_DROP_SECS` in): each one's soul is
//! torn up out of its body — a twisting column of the Exhumer's green
//! soul-smoke (`exhumer::SoulSmoke`) ripping upward from its chest, thinning
//! as it climbs — each giving its normal death cry (`zombie_sounds`). The
//! nearest few can also scream as it goes (the Nuke's soul screams, pitched
//! down, staggered — off by default: [`RitualDeathSettings::screamers`]).
//!
//! Each column is `StateScoped(InGame)` and gone once it's played out;
//! which zombies we last saw alive is forgotten out of a game. Tuned in the
//! debug panel's "Easter egg (Der Tempel)" → "Ritual deaths"
//! ([`RitualDeathSettings`]).

use bevy::pbr::NotShadowCaster;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::easter_egg::EggStep;
use shared::{Lobby, PlayerPose};

use crate::audio_engine::prelude::*;
use crate::ZombieVisual;
use crate::exhumer::{along_rotation, SoulSmoke};
use crate::net::{GameClient, RemoteAvatar};
use crate::util::rand01;
use crate::vfx::{smoke_billboard_rotation, ExplosionAssets};
use crate::{AppState, WorldModelCamera, EYE_HEIGHT};

pub(super) struct RitualDeathsPlugin;

impl Plugin for RitualDeathsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RitualDeathSettings>()
            .init_resource::<SeenAlive>()
            .add_systems(OnExit(AppState::InGame), |mut seen: ResMut<SeenAlive>| seen.0.clear())
            .add_systems(
                Update,
                (tear_souls, scream_later, rise_souls).chain().run_if(in_state(AppState::InGame)),
            );
    }
}

/// Panel-tunable look ("Ritual deaths").
#[derive(Resource, Clone, Debug)]
pub(crate) struct RitualDeathSettings {
    /// How long (s) each soul takes to tear free and fade, and how high (m)
    /// it climbs.
    pub(crate) secs: f32,
    pub(crate) height: f32,
    /// Its streaks: how many, how long (m), how wide it twists (m), how
    /// fast (turns/s).
    pub(crate) streaks: u32,
    pub(crate) streak_len: f32,
    pub(crate) twist_radius: f32,
    pub(crate) twist_speed: f32,
    /// The haze round the body (m).
    pub(crate) haze_size: f32,
    /// The screams: how many zombies (the nearest) scream, spread over how
    /// long (s), pitched to (× speed), how loud.
    pub(crate) screamers: u32,
    pub(crate) scream_spread: f32,
    pub(crate) scream_pitch: f32,
    pub(crate) scream_volume: f32,
}

impl Default for RitualDeathSettings {
    fn default() -> Self {
        Self {
            secs: 2.4,
            height: 3.4,
            streaks: 7,
            streak_len: 1.3,
            twist_radius: 0.35,
            twist_speed: 0.8,
            haze_size: 1.6,
            screamers: 0,
            scream_spread: 0.45,
            scream_pitch: 0.78,
            scream_volume: 1.2,
        }
    }
}

/// The zombie avatars we last saw alive — so a drop is seen as it happens.
#[derive(Resource, Default)]
struct SeenAlive(HashSet<Entity>);

/// A soul torn out of a body at `from` (its chest): how long (s) since,
/// and its own seed.
#[derive(Component)]
struct TornSoul {
    from: Vec3,
    age: f32,
    seed: f32,
}

/// One part of a [`TornSoul`]: a streak of the swirl (its `i`th, climbing
/// a little behind the last) or the haze round the body.
#[derive(Component)]
struct TornPart {
    i: u32,
    haze: bool,
    flip: Vec2,
}

/// A scream to start at `at` (`Time::elapsed_secs`), from `from`.
#[derive(Component)]
struct ScreamLater {
    at: f32,
    from: Vec3,
    clip: Handle<AudioSource>,
}

/// Watch every zombie: one dropping while the ritual's gone wrong has its
/// soul torn out — and the nearest few scream.
#[allow(clippy::too_many_arguments)]
fn tear_souls(
    time: Res<Time>,
    settings: Res<RitualDeathSettings>,
    (local, lobbies): (Query<&LocalId, With<GameClient>>, Query<&Lobby>),
    (fx, smoke, sounds): (Option<Res<ExplosionAssets>>, Option<Res<SoulSmoke>>, Option<Res<crate::GameSounds>>),
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    zombies: Query<(Entity, &RemoteAvatar), With<ZombieVisual>>,
    poses: Query<&PlayerPose>,
    mut seen: ResMut<SeenAlive>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let unleashed = crate::zombies_hud::zombies_game(&local, &lobbies).is_some_and(|l| l.easter_egg.step == EggStep::Unleashed);
    let mut torn: Vec<Vec3> = Vec::new();
    let mut alive_now = HashSet::default();
    for (entity, avatar) in &zombies {
        let Ok(pose) = poses.get(avatar.src) else { continue };
        if pose.alive {
            alive_now.insert(entity);
        } else if unleashed && seen.0.contains(&entity) {
            torn.push(pose.translation - Vec3::Y * EYE_HEIGHT);
        }
    }
    seen.0 = alive_now;
    if torn.is_empty() {
        return;
    }
    let s = &*settings;
    let (Some(fx), Some(smoke)) = (fx, smoke) else { return };
    let mut roll = || {
        *seq = seq.wrapping_add(1);
        rand01(seq.wrapping_mul(2_654_435_761) ^ now.to_bits())
    };
    for feet in &torn {
        let from = *feet + Vec3::Y * 1.2;
        let seed = roll();
        let flip = |r: f32| if r < 0.5 { -1.0 } else { 1.0 };
        commands
            .spawn((
                StateScoped(AppState::InGame),
                TornSoul { from, age: 0.0, seed },
                Transform::from_translation(from),
                Visibility::default(),
            ))
            .with_children(|c| {
                for i in 0..s.streaks.max(1) {
                    let tier = if i == 0 { 0 } else if i < 3 { 1 } else { 2 };
                    c.spawn((
                        TornPart { i, haze: false, flip: Vec2::new(flip(roll()), flip(roll())) },
                        Mesh3d(fx.quad.clone()),
                        MeshMaterial3d(smoke.swirl_material(tier)),
                        Transform::from_scale(Vec3::ZERO),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                }
                c.spawn((
                    TornPart { i: 0, haze: true, flip: Vec2::ONE },
                    Mesh3d(fx.quad.clone()),
                    MeshMaterial3d(smoke.puff_material()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            });
    }
    // The nearest few scream, staggered.
    let (Some(sounds), Some(cam)) = (sounds, camera) else { return };
    if sounds.zombie_souls.is_empty() {
        return;
    }
    let eye = cam.translation();
    torn.sort_by(|a, b| a.distance(eye).total_cmp(&b.distance(eye)));
    for feet in torn.into_iter().take(s.screamers as usize) {
        let pick = ((roll() * sounds.zombie_souls.len() as f32) as usize).min(sounds.zombie_souls.len() - 1);
        commands.spawn((
            StateScoped(AppState::InGame),
            ScreamLater { at: now + roll() * s.scream_spread, from: feet + Vec3::Y * 1.6, clip: sounds.zombie_souls[pick].clone() },
        ));
    }
}

/// Start each scream once its moment comes.
fn scream_later(
    time: Res<Time>,
    settings: Res<RitualDeathSettings>,
    screams: Query<(Entity, &ScreamLater)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (e, scream) in &screams {
        if now < scream.at {
            continue;
        }
        commands.entity(e).despawn();
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(scream.clip.clone()),
            Transform::from_translation(scream.from),
            crate::positional_playback(Volume::Linear(settings.scream_volume)).with_speed(settings.scream_pitch.max(0.1)),
        ));
    }
}

/// Every torn soul climbing: its streaks laid along the way they're
/// going, twisting round each other as they rip upward — the first
/// leading, the rest a little behind — swelling, then thinning to nothing
/// at the top; the haze round the body swelling and fading under them.
fn rise_souls(
    time: Res<Time>,
    settings: Res<RitualDeathSettings>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    mut souls: Query<(Entity, &mut TornSoul, &Children)>,
    mut parts: Query<(&TornPart, &mut Transform), Without<TornSoul>>,
    mut commands: Commands,
) {
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    let s = &*settings;
    let dt = time.delta_secs();
    let secs = s.secs.max(0.1);
    let n = s.streaks.max(1) as f32;
    for (e, mut soul, children) in &mut souls {
        soul.age += dt;
        if soul.age >= secs {
            commands.entity(e).despawn();
            continue;
        }
        let u = soul.age / secs;
        // Where a streak `lag` (share of the climb) behind the lead is: up
        // and twisting round the line straight up from the chest (ripped
        // out fast, slowing as it goes).
        let path = |x: f32, phase: f32| {
            let x = x.clamp(0.0, 1.0);
            let up = 1.0 - (1.0 - x) * (1.0 - x);
            let a = phase + x * s.twist_speed * std::f32::consts::TAU * 2.0;
            Vec3::new(a.cos(), 0.0, a.sin()) * s.twist_radius * (0.3 + x) + Vec3::Y * up * s.height
        };
        for child in children.iter() {
            let Ok((part, mut tf)) = parts.get_mut(child) else { continue };
            if part.haze {
                let swell = (u * 4.0).min(1.0) * (1.0 - u).powi(2);
                let size = (s.haze_size * swell).max(1.0e-4);
                let at = soul.from + Vec3::Y * 0.2;
                *tf = Transform::from_translation(Vec3::Y * 0.2)
                    .with_rotation(smoke_billboard_rotation(at, cam_pos, cam_up, cam_right, soul.seed * 6.0 + u))
                    .with_scale(Vec3::new(size, size, 1.0));
                continue;
            }
            let k = part.i as f32 / n;
            let x = u - k * 0.25;
            if x <= 0.0 {
                tf.scale = Vec3::ZERO;
                continue;
            }
            let phase = soul.seed * std::f32::consts::TAU + k * std::f32::consts::TAU;
            let here = path(x, phase);
            let going = (path(x + 0.02, phase) - path(x - 0.02, phase)).normalize_or(Vec3::Y);
            // (In fast, thinning away as it reaches the top.)
            let size = s.streak_len * (0.7 + 0.5 * k) * (x * 6.0).min(1.0) * (1.0 - x).max(0.0).sqrt();
            let size = size.max(1.0e-4);
            let world = soul.from + here;
            *tf = Transform::from_translation(here)
                .with_rotation(along_rotation(world, going, cam_pos, 0.25 * (x * 9.0 + phase).sin()))
                .with_scale(Vec3::new(size * part.flip.x, size * 0.5 * part.flip.y, 1.0));
        }
    }
}

/// The debug panel's controls for it.
pub(crate) fn ritual_deaths_section(ui: &mut egui::Ui, s: &mut RitualDeathSettings) {
    ui.add(egui::Slider::new(&mut s.secs, 0.3f32..=6.0).text("soul tears free over (s)"));
    ui.add(egui::Slider::new(&mut s.height, 0.5f32..=10.0).text("climbs (m)"));
    ui.add(egui::Slider::new(&mut s.streaks, 1..=16).text("streaks (next ones)"));
    ui.add(egui::Slider::new(&mut s.streak_len, 0.1f32..=4.0).text("streak length (m)"));
    ui.add(egui::Slider::new(&mut s.twist_radius, 0.0f32..=2.0).text("twists out (m)"));
    ui.add(egui::Slider::new(&mut s.twist_speed, 0.0f32..=4.0).text("twists (turns)"));
    ui.add(egui::Slider::new(&mut s.haze_size, 0.0f32..=5.0).text("haze round the body (m)"));
    ui.add(egui::Slider::new(&mut s.screamers, 0..=20).text("nearest that scream"));
    ui.add(egui::Slider::new(&mut s.scream_spread, 0.0f32..=2.0).text("screams spread over (s)"));
    ui.add(egui::Slider::new(&mut s.scream_pitch, 0.3f32..=1.5).text("scream pitch (× speed)"));
    ui.add(egui::Slider::new(&mut s.scream_volume, 0.0f32..=3.0).text("scream volume"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = RitualDeathSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
