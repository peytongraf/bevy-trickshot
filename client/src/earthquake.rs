//! Der Tempel's earthquake (`shared::power::scary_power_on`): the moment the
//! power comes on in a `Zombies` game there, the ground shakes for a few seconds — fading in fast, out
//! slowly — and dirt, rocks and dust pour down from whatever's overhead.
//!
//! The shake is its own camera offset ([`Quake::offset`]), layered under the
//! per-shot shake by `weapons::camera_shake` (which rebuilds the shake node
//! from zero every frame, so neither can drift the aim). The debris is the
//! same billboarded `vfx::ImpactParticle` a bullet hitting the ground kicks
//! up, spawned only under something solid (a ray straight up from near the
//! player has to hit the map), within [`EarthquakeSettings::debris_radius`]
//! of the player and mostly in front of them, so none is wasted where it
//! can't be seen.
//!
//! It comes with its own sound — a rumble and scary music
//! (`GameSounds::der_tempel_power_on`), heard by the whole lobby, not
//! positional, its volume following the shake's strength (on top of the
//! lever's own power-on sound, which still plays).
//!
//! Only a flip of `Lobby::power_on` seen while in the game starts it — joining
//! with the power already on doesn't. Out of a game it's dropped straight
//! back to nothing, and the debris is `StateScoped(InGame)`, so nothing
//! carries into the next one. Tuned (and test-fired on any map) in the debug
//! panel's "Earthquake (Der Tempel)" section.

use std::f32::consts::TAU;

use bevy::ecs::system::SystemParam;
use bevy::math::Affine2;
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::LocalId;
use shared::Lobby;

use crate::net::GameClient;
use crate::util::{rand01, rand_roll};
use crate::vfx::{impact_material, ImpactAssets, ImpactParticle, IMPACT_MAX, ROCK_COLS, ROCK_ROWS};
use crate::zombies_hud::zombies_game;
use crate::{AppState, MapModel, Player};

pub(crate) struct EarthquakePlugin;

impl Plugin for EarthquakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EarthquakeSettings>()
            .init_resource::<Quake>()
            .add_systems(
                Update,
                (
                    // (Ungated, so leaving a game mid-quake stops it.)
                    (start_quake, update_quake, quake_sound).chain(),
                    quake_debris.run_if(in_state(AppState::InGame)),
                )
                    .chain()
                    .before(crate::camera_shake),
            );
    }
}

/// Panel-tunable earthquake.
#[derive(Resource, Clone)]
pub(crate) struct EarthquakeSettings {
    pub(crate) enabled: bool,
    /// Seconds to build to full strength, hold there, and die away.
    pub(crate) fade_in_secs: f32,
    pub(crate) hold_secs: f32,
    pub(crate) fade_out_secs: f32,
    /// The rumble's speed (Hz) — the fast jolting...
    pub(crate) frequency: f32,
    /// ...and the slow rolling sway under it.
    pub(crate) sway_frequency: f32,
    /// Peak camera jolt (m) at full strength.
    pub(crate) pos_max: f32,
    /// Peak tilt (°) at full strength: side to side, nodding, and turning.
    pub(crate) roll_deg: f32,
    pub(crate) pitch_deg: f32,
    pub(crate) yaw_deg: f32,
    /// Debris falls only within this far (m) of the player...
    pub(crate) debris_radius: f32,
    /// ...from something at most this far (m) overhead.
    pub(crate) ceiling_max: f32,
    /// Spots debris falls from per second at full strength.
    pub(crate) debris_rate: f32,
    /// Rocks and dust puffs from each spot.
    pub(crate) rocks: u32,
    pub(crate) dust: u32,
    /// Rock sprite size (m) and how far they bounce sideways (m/s).
    pub(crate) rock_size: f32,
    pub(crate) rock_scatter: f32,
    /// Dust: how fast it pours down (m/s), its size at the top and once
    /// spread out (m), how long it hangs (s), and its peak opacity.
    pub(crate) dust_fall: f32,
    pub(crate) dust_start_size: f32,
    pub(crate) dust_end_size: f32,
    pub(crate) dust_secs: f32,
    pub(crate) dust_opacity: f32,
    /// The dust's colour (sRGB).
    pub(crate) dust_tint: [f32; 3],
}

impl Default for EarthquakeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            fade_in_secs: 0.3,
            hold_secs: 1.2,
            fade_out_secs: 3.0,
            frequency: 9.0,
            sway_frequency: 1.3,
            pos_max: 0.045,
            roll_deg: 1.4,
            pitch_deg: 0.9,
            yaw_deg: 0.5,
            debris_radius: 22.0,
            ceiling_max: 25.0,
            debris_rate: 26.0,
            rocks: 3,
            dust: 2,
            rock_size: 0.07,
            rock_scatter: 0.35,
            dust_fall: 1.6,
            dust_start_size: 0.35,
            dust_end_size: 1.8,
            dust_secs: 2.6,
            dust_opacity: 0.35,
            dust_tint: [0.72, 0.64, 0.54],
        }
    }
}

impl EarthquakeSettings {
    fn total_secs(&self) -> f32 {
        self.fade_in_secs.max(0.0) + self.hold_secs.max(0.0) + self.fade_out_secs.max(0.0)
    }

    /// How strong it is (0..=1) `t` seconds in.
    fn strength(&self, t: f32) -> f32 {
        let smooth = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let (fade_in, hold) = (self.fade_in_secs.max(0.0), self.hold_secs.max(0.0));
        if t < 0.0 {
            0.0
        } else if t < fade_in {
            smooth(t / fade_in)
        } else if t < fade_in + hold {
            1.0
        } else {
            // Dies away slowly, with a long tail.
            let x = (t - fade_in - hold) / self.fade_out_secs.max(0.01);
            (1.0 - x.clamp(0.0, 1.0)).powi(2)
        }
    }

    /// The camera's offset at strength `level`, `t` seconds in — a layered
    /// rumble: fast jolting over a slow, rolling sway.
    fn pose(&self, t: f32, level: f32) -> Transform {
        if level <= 0.0 {
            return Transform::IDENTITY;
        }
        let (fast, slow) = (self.frequency * TAU, self.sway_frequency * TAU);
        let wave = |f: f32, offset: f32| (t * f + offset).sin();
        let jolt = |a: f32, b: f32| wave(fast * a, b) * 0.6 + wave(fast * a * 1.73, b * 2.3 + 1.0) * 0.4;
        let sway = |a: f32, b: f32| wave(slow * a, b);
        let translation = Vec3::new(
            jolt(1.0, 0.3) * 0.7 + sway(1.0, 1.1) * 0.3,
            jolt(1.21, 2.0),
            jolt(0.87, 4.1) * 0.5,
        ) * self.pos_max
            * level;
        let rotation = Quat::from_euler(
            EulerRot::YXZ,
            (jolt(0.93, 3.3) * 0.5 + sway(0.8, 0.5) * 0.5) * self.yaw_deg.to_radians() * level,
            (jolt(1.11, 5.2) * 0.6 + sway(1.3, 2.4) * 0.4) * self.pitch_deg.to_radians() * level,
            (jolt(1.37, 0.2) * 0.4 + sway(0.9, 1.9) * 0.6) * self.roll_deg.to_radians() * level,
        );
        Transform {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }
}

/// The earthquake in progress, if any.
#[derive(Resource, Default)]
pub(crate) struct Quake {
    /// When it started (`Time::elapsed_secs`), while it's going.
    started: Option<f32>,
    /// How strong it is right now, 0..=1.
    level: f32,
    /// The camera's shake offset right now — `camera_shake` layers it in.
    pub(crate) offset: Transform,
    /// Debris spots owed so far (fractional).
    owed: f32,
    /// Debug: start one now (the panel's "Test").
    test: bool,
}

/// On the earthquake's sound while it plays.
#[derive(Component)]
struct QuakeSound;

/// Start one when the power comes on in a Der Tempel `Zombies` game (or the
/// panel asks), with its sound; forget everything once out of a game.
#[allow(clippy::too_many_arguments)]
fn start_quake(
    state: Res<State<AppState>>,
    settings: Res<EarthquakeSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    time: Res<Time>,
    mut quake: ResMut<Quake>,
    sounds: Option<Res<crate::GameSounds>>,
    playing: Query<Entity, With<QuakeSound>>,
    mut commands: Commands,
    // Whether the power was on last frame — `None` until seen in a game.
    mut was_on: Local<Option<bool>>,
) {
    if *state.get() != AppState::InGame {
        *was_on = None;
        if quake.started.is_some() || quake.level != 0.0 || quake.test {
            *quake = Quake::default();
        }
        return;
    }
    let on = zombies_game(&local, &lobbies).filter(|l| shared::power::scary_power_on(l.map)).map(|l| l.power_on);
    let flipped = *was_on == Some(false) && on == Some(true);
    *was_on = on;
    let test = std::mem::take(&mut quake.bypass_change_detection().test);
    if (flipped && settings.enabled) || test {
        quake.started = Some(time.elapsed_secs());
        quake.owed = 0.0;
        // (Starting over if one was still going.)
        for e in &playing {
            commands.entity(e).despawn();
        }
        if let Some(sounds) = &sounds {
            commands.spawn((
                QuakeSound,
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.der_tempel_power_on.clone()),
                // Starts silent; `quake_sound` fades it with the shake.
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0)),
            ));
        }
    }
}

/// Its strength and the camera offset, this frame.
fn update_quake(settings: Res<EarthquakeSettings>, time: Res<Time>, mut quake: ResMut<Quake>) {
    let Some(started) = quake.started else {
        return;
    };
    let t = time.elapsed_secs() - started;
    if t >= settings.total_secs() {
        quake.started = None;
        quake.level = 0.0;
        quake.offset = Transform::IDENTITY;
        return;
    }
    quake.level = settings.strength(t);
    quake.offset = settings.pose(t, quake.level);
}

/// Keep the sound's volume on the shake's strength (and the panel's and
/// master volume), and stop it once the quake's over.
fn quake_sound(
    quake: Res<Quake>,
    vols: Res<crate::SoundVolumes>,
    settings: Res<crate::settings::Settings>,
    mut sounds: Query<(Entity, Option<&mut AudioSink>), With<QuakeSound>>,
    mut commands: Commands,
) {
    for (e, sink) in &mut sounds {
        if quake.started.is_none() {
            commands.entity(e).despawn();
            continue;
        }
        let Some(mut sink) = sink else { continue };
        let volume = Volume::Linear(quake.level * vols.der_tempel_power_on * settings.master_volume);
        if sink.volume() != volume {
            sink.set_volume(volume);
        }
    }
}

/// Pour debris down from overhead, near the player, while it shakes.
#[allow(clippy::too_many_arguments)]
fn quake_debris(
    time: Res<Time>,
    settings: Res<EarthquakeSettings>,
    mut quake: ResMut<Quake>,
    player: Single<&Transform, With<Player>>,
    rapier: ReadRapierContext,
    map_models: Query<(), With<MapModel>>,
    parents: Query<&ChildOf>,
    assets: Res<ImpactAssets>,
    existing: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    if quake.level <= 0.0 {
        return;
    }
    quake.owed += settings.debris_rate * quake.level * time.delta_secs();
    if quake.owed < 1.0 {
        return;
    }
    let Ok(rapier) = rapier.single() else {
        return;
    };
    // Only the map itself counts as something overhead — not a player,
    // a zombie or a machine.
    let on_map = |mut e: Entity| loop {
        if map_models.contains(e) {
            return true;
        }
        match parents.get(e) {
            Ok(parent) => e = parent.parent(),
            Err(_) => return false,
        }
    };
    let filter = QueryFilter::default().predicate(&on_map);
    let eye = player.translation;
    let facing = {
        let f = player.rotation * Vec3::NEG_Z;
        f.z.atan2(f.x)
    };
    let mut budget = IMPACT_MAX.saturating_sub(existing.iter().count());

    while quake.owed >= 1.0 {
        quake.owed -= 1.0;
        *seq = seq.wrapping_add(1);
        let s = seq.wrapping_mul(2_654_435_761);
        // Mostly ahead of the player (where it can be seen), now and then
        // anywhere round them.
        let spread = if rand01(s ^ 0x1) < 0.8 { 1.1 } else { std::f32::consts::PI };
        let angle = facing + (rand01(s ^ 0x2) * 2.0 - 1.0) * spread;
        // (Square root: even over the disc, not bunched at the middle.)
        let r = settings.debris_radius.max(0.5) * rand01(s ^ 0x3).sqrt();
        let from = Vec3::new(eye.x + r * angle.cos(), eye.y, eye.z + r * angle.sin());
        // Something overhead to fall from? (Starting inside a wall counts
        // as no.)
        let Some((_, up)) = rapier.cast_ray(from, Vec3::Y, settings.ceiling_max, true, filter) else {
            continue;
        };
        if up < 0.3 {
            continue;
        }
        let ceiling = from + Vec3::Y * (up - 0.05);
        // And what it lands on.
        let floor = rapier
            .cast_ray(ceiling, Vec3::NEG_Y, up + 30.0, true, filter)
            .map_or(eye.y - crate::EYE_HEIGHT, |(_, down)| ceiling.y - down)
            + 0.03;
        spawn_falling_debris(&mut commands, &mut materials, &assets, &settings, ceiling, floor, s, &mut budget);
        if budget == 0 {
            quake.owed = 0.0;
        }
    }
}

/// Rocks dropping and dust pouring down from `ceiling`, onto `floor`.
#[allow(clippy::too_many_arguments)]
fn spawn_falling_debris(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    assets: &ImpactAssets,
    settings: &EarthquakeSettings,
    ceiling: Vec3,
    floor: f32,
    seq: u32,
    budget: &mut usize,
) {
    const GRAVITY: f32 = 9.8;
    let height = (ceiling.y - floor).max(0.0);
    let fall_secs = (2.0 * height / GRAVITY).sqrt();
    for i in 0..settings.rocks {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let s = seq.wrapping_add(i.wrapping_mul(40_503)) ^ 0x51;
        let side = Vec3::new(rand01(s ^ 0x1) * 2.0 - 1.0, 0.0, rand01(s ^ 0x2) * 2.0 - 1.0);
        let size = settings.rock_size * (0.5 + rand01(s ^ 0x3));
        // One rock off `vfx/rocks.png`'s sheet (as `vfx::spawn_debris`).
        let col = (rand01(s ^ 0x4) * ROCK_COLS as f32) as u32 % ROCK_COLS;
        let row = (rand01(s ^ 0x5) * ROCK_ROWS as f32) as u32 % ROCK_ROWS;
        let mut material = impact_material(assets.rocks.clone());
        material.uv_transform = Affine2::from_scale_angle_translation(
            Vec2::new(1.0 / ROCK_COLS as f32, 1.0 / ROCK_ROWS as f32),
            0.0,
            Vec2::new(col as f32 / ROCK_COLS as f32, row as f32 / ROCK_ROWS as f32),
        );
        // Falls the whole way solid, lies there a moment, then fades.
        let lifetime = fall_secs + 0.8;
        commands.spawn((
            StateScoped(AppState::InGame),
            ImpactParticle {
                velocity: side * settings.rock_scatter,
                gravity: GRAVITY,
                drag: 0.0,
                age: 0.0,
                lifetime,
                fade_in: 0.0,
                roll: rand_roll(s ^ 0x6),
                spin: (rand01(s ^ 0x7) * 2.0 - 1.0) * 9.0,
                scale0: size,
                scale1: size,
                peak_alpha: 1.0,
                tint: [0.8, 0.74, 0.66],
                floor,
                hold: (fall_secs + 0.3) / lifetime,
            },
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(materials.add(material)),
            // (Staggered down a little, so they trickle rather than drop as one.)
            Transform::from_translation(ceiling - Vec3::Y * rand01(s ^ 0x8) * 0.4)
                .with_scale(Vec3::splat(size.max(1.0e-4))),
            NoFrustumCulling,
            NotShadowCaster,
        ));
    }
    let [r, g, b] = settings.dust_tint;
    for i in 0..settings.dust {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let s = seq.wrapping_add(i.wrapping_mul(2_654_435_761)) ^ 0xd057;
        let side = Vec3::new(rand01(s ^ 0x1) * 2.0 - 1.0, 0.0, rand01(s ^ 0x2) * 2.0 - 1.0) * 0.2;
        let size = 0.75 + 0.5 * rand01(s ^ 0x3);
        commands.spawn((
            StateScoped(AppState::InGame),
            ImpactParticle {
                // Pours down, slowing as it spreads.
                velocity: side - Vec3::Y * settings.dust_fall * (0.6 + 0.8 * rand01(s ^ 0x4)),
                gravity: 1.2,
                drag: 0.5,
                age: 0.0,
                lifetime: settings.dust_secs.max(0.1) * (0.75 + 0.5 * rand01(s ^ 0x5)),
                fade_in: 0.15,
                roll: rand_roll(s ^ 0x6),
                spin: (rand01(s ^ 0x7) * 2.0 - 1.0) * 0.4,
                scale0: settings.dust_start_size * size,
                scale1: settings.dust_end_size * size,
                peak_alpha: settings.dust_opacity,
                tint: [r, g, b],
                floor: floor + 0.2,
                hold: 0.0,
            },
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(materials.add(impact_material(assets.dust.clone()))),
            Transform::from_translation(ceiling - Vec3::Y * 0.1)
                .with_scale(Vec3::splat((settings.dust_start_size * size).max(1.0e-4))),
            NoFrustumCulling,
            NotShadowCaster,
        ));
    }
}

/// The debug panel's "Earthquake (Der Tempel)" section.
#[derive(SystemParam)]
pub(crate) struct EarthquakeDebug<'w> {
    settings: ResMut<'w, EarthquakeSettings>,
    quake: ResMut<'w, Quake>,
}

impl EarthquakeDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let s = &mut *self.settings;
        ui.label("Shakes the ground and pours debris from overhead when the power comes on (Der Tempel only).");
        ui.label(format!("now: {:.0}%", self.quake.level * 100.0));
        ui.checkbox(&mut s.enabled, "enabled");
        if ui.button("Test now (any map)").clicked() {
            self.quake.test = true;
        }
        ui.separator();
        ui.add(egui::Slider::new(&mut s.fade_in_secs, 0.0f32..=3.0).text("fade in (s)"));
        ui.add(egui::Slider::new(&mut s.hold_secs, 0.0f32..=6.0).text("hold (s)"));
        ui.add(egui::Slider::new(&mut s.fade_out_secs, 0.0f32..=8.0).text("fade out (s)"));
        ui.add(egui::Slider::new(&mut s.frequency, 1.0f32..=25.0).text("rumble speed (Hz)"));
        ui.add(egui::Slider::new(&mut s.sway_frequency, 0.1f32..=5.0).text("sway speed (Hz)"));
        ui.add(egui::Slider::new(&mut s.pos_max, 0.0f32..=0.3).text("jolt (m)"));
        ui.add(egui::Slider::new(&mut s.roll_deg, 0.0f32..=8.0).text("roll (°)"));
        ui.add(egui::Slider::new(&mut s.pitch_deg, 0.0f32..=8.0).text("pitch (°)"));
        ui.add(egui::Slider::new(&mut s.yaw_deg, 0.0f32..=8.0).text("yaw (°)"));
        ui.separator();
        ui.label("Debris");
        ui.add(egui::Slider::new(&mut s.debris_radius, 2.0f32..=60.0).text("max distance (m)"));
        ui.add(egui::Slider::new(&mut s.ceiling_max, 2.0f32..=60.0).text("max height overhead (m)"));
        ui.add(egui::Slider::new(&mut s.debris_rate, 0.0f32..=120.0).text("spots / s"));
        ui.add(egui::Slider::new(&mut s.rocks, 0..=12).text("rocks per spot"));
        ui.add(egui::Slider::new(&mut s.dust, 0..=8).text("dust puffs per spot"));
        ui.add(egui::Slider::new(&mut s.rock_size, 0.01f32..=0.4).text("rock size (m)"));
        ui.add(egui::Slider::new(&mut s.rock_scatter, 0.0f32..=3.0).text("rock scatter (m/s)"));
        ui.add(egui::Slider::new(&mut s.dust_fall, 0.0f32..=6.0).text("dust pour speed (m/s)"));
        ui.add(egui::Slider::new(&mut s.dust_start_size, 0.05f32..=2.0).text("dust size at top (m)"));
        ui.add(egui::Slider::new(&mut s.dust_end_size, 0.1f32..=5.0).text("dust size spread out (m)"));
        ui.add(egui::Slider::new(&mut s.dust_secs, 0.2f32..=8.0).text("dust hangs (s)"));
        ui.add(egui::Slider::new(&mut s.dust_opacity, 0.0f32..=1.0).text("dust opacity"));
        ui.horizontal(|ui| {
            ui.color_edit_button_rgb(&mut s.dust_tint);
            ui.label("dust colour");
        });
        if ui.button("Reset").clicked() {
            *s = EarthquakeSettings {
                enabled: s.enabled,
                ..default()
            };
        }
    }
}
