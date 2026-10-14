//! The fires a map's layout keeps burning (`shared::level::Flame`) — in a
//! wall-mounted bowl, say: a few overlapping flame quads in the molotov's own
//! fire ([`FireMaterial`]) and a flickering light, lit the whole time, power
//! on or off, day or night.
//!
//! In a game each also crackles — `GameSounds::fire_crackle`, looped from the
//! fire, fading with distance ([`FlameSoundSettings`], the debug panel's
//! "Fire crackle" section) and paused out of earshot.
//!
//! In a game they're the current map's ([`sync_game_flames`], all
//! `StateScoped(InGame)`); in the level editor its own copy follows the
//! layout being edited (`level_editor`, through [`sync_flames`]). Each set is
//! rebuilt whenever its list changes, so nothing carries over between games.

use bevy::ecs::system::SystemParam;
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use shared::level::Flame;

use crate::molotov::{flicker, seeded_scale, FireMaterial, MolotovAssets, MolotovSettings};
use crate::util::rand01;
use crate::{AppState, CurrentMap};

pub(crate) struct FlamesPlugin;

impl Plugin for FlamesPlugin {
    fn build(&self, app: &mut App) {
        // (Ungated: leaving the game is what takes them away.)
        app.init_resource::<FlameSoundSettings>()
            .add_systems(Update, (sync_game_flames, flicker_flames, fade_flame_sounds));
    }
}

/// Panel-tunable crackle: its own short range, so a fire's only heard near it.
#[derive(Resource, Clone)]
pub(crate) struct FlameSoundSettings {
    pub(crate) volume: f32,
    /// Full volume within this far (m) of a fire...
    pub(crate) full_distance: f32,
    /// ...fading to silence here (and paused beyond it).
    pub(crate) max_distance: f32,
    /// The fade's curve between them (1 = linear; higher drops off sooner).
    pub(crate) falloff: f32,
}

impl Default for FlameSoundSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            full_distance: 1.5,
            max_distance: 12.0,
            falloff: 2.0,
        }
    }
}

impl FlameSoundSettings {
    /// Loudness (before the global volume) `distance` m away.
    fn loudness(&self, distance: f32) -> f32 {
        let span = (self.max_distance - self.full_distance).max(1e-3);
        let t = ((distance - self.full_distance) / span).clamp(0.0, 1.0);
        self.volume * (1.0 - t).powf(self.falloff.max(0.01))
    }
}

/// On a fire's looping crackle.
#[derive(Component)]
struct FlameSound;

/// Each flame's overlapping layers: `(width, height)` relative to its size,
/// and its seed — so it has depth instead of reading as one flat sprite.
const LAYERS: [(f32, f32, f32); 3] = [(1.0, 1.0, 0.17), (0.72, 0.88, 0.53), (0.5, 0.68, 0.81)];

/// On a fire's root: its material (its own, freed when it goes).
#[derive(Component)]
pub(crate) struct MapFlame(Handle<FireMaterial>);

/// On a fire's light.
#[derive(Component)]
struct FlameLight {
    lumens: f32,
    flicker: f32,
    seed: f32,
}

/// On the game's fires (not the editor's).
#[derive(Component)]
struct GameFlame;

/// Keep the current map's fires burning while in a game.
#[allow(clippy::too_many_arguments)]
fn sync_game_flames(
    state: Res<State<AppState>>,
    current: Res<CurrentMap>,
    assets: Option<Res<MolotovAssets>>,
    molotov: Res<MolotovSettings>,
    sounds: Option<Res<crate::GameSounds>>,
    existing: Query<(Entity, &MapFlame), With<GameFlame>>,
    mut materials: ResMut<Assets<FireMaterial>>,
    mut commands: Commands,
    mut shown: Local<Vec<Flame>>,
) {
    let wanted: &[Flame] = if *state.get() == AppState::InGame {
        &shared::level::layout(current.0).flames
    } else {
        &[]
    };
    sync_flames(
        wanted,
        &mut shown,
        &existing,
        assets.as_deref(),
        &molotov,
        sounds.as_ref().map(|s| &s.fire_crackle),
        &mut materials,
        &mut commands,
        || (StateScoped(AppState::InGame), GameFlame),
    );
}

/// Make the fires spawned with `marker` match `wanted`: all rebuilt whenever
/// it differs from what was last built (`shown`) or some have gone (left the
/// state they were scoped to). With a `crackle`, each loops it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_flames<F: bevy::ecs::query::QueryFilter, B: Bundle>(
    wanted: &[Flame],
    shown: &mut Vec<Flame>,
    existing: &Query<(Entity, &MapFlame), F>,
    assets: Option<&MolotovAssets>,
    molotov: &MolotovSettings,
    crackle: Option<&Handle<AudioSource>>,
    materials: &mut Assets<FireMaterial>,
    commands: &mut Commands,
    marker: impl Fn() -> B,
) {
    if shown.as_slice() == wanted && existing.iter().count() == wanted.len() {
        return;
    }
    let Some(assets) = assets else { return };
    for (e, flame) in existing {
        materials.remove(&flame.0);
        commands.entity(e).despawn();
    }
    *shown = wanted.to_vec();
    for (i, flame) in wanted.iter().enumerate() {
        let seed = rand01((i as u32).wrapping_mul(2_654_435_761) ^ flame.pos.x.to_bits() ^ flame.pos.z.to_bits());
        let material = materials.add(FireMaterial::flame(molotov, flame.brightness, Vec3::ZERO));
        commands
            .spawn((
                marker(),
                MapFlame(material.clone()),
                Transform::from_translation(flame.pos),
                Visibility::default(),
            ))
            .with_children(|fire| {
                for (w, h, layer_seed) in LAYERS {
                    let size = Vec2::new(flame.size * w, flame.size * flame.height * h);
                    fire.spawn((
                        Mesh3d(assets.quad.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_scale(seeded_scale(size, layer_seed + seed)),
                        NotShadowCaster,
                        // (Drawn well outside the quad's own bounds —
                        // billboarded, its base on the anchor.)
                        NoFrustumCulling,
                    ));
                }
                fire.spawn((
                    FlameLight {
                        lumens: flame.light,
                        flicker: flame.flicker,
                        seed: seed * 10.0,
                    },
                    PointLight {
                        color: molotov.light_color(),
                        intensity: flame.light,
                        range: flame.range,
                        shadows_enabled: false,
                        ..default()
                    },
                    // (Up in the flames, not at their base.)
                    Transform::from_xyz(0.0, flame.size * flame.height * 0.4, 0.0),
                ));
                if let Some(crackle) = crackle {
                    fire.spawn((
                        FlameSound,
                        // Volume is ours to set (`fade_flame_sounds`), not
                        // the one-shot volume pass's.
                        crate::RemoteSoundEmitter,
                        AudioPlayer::new(crackle.clone()),
                        Transform::from_xyz(0.0, flame.size * 0.5, 0.0),
                        // Looped; starts silent, the real volume set from the
                        // next frame.
                        PlaybackSettings::LOOP
                            .with_spatial(true)
                            .with_volume(Volume::Linear(0.0)),
                    ));
                }
            });
    }
}

/// Keep each fire's crackle at the loudness for how far the listener is —
/// paused once out of earshot, so far-off fires cost nothing.
fn fade_flame_sounds(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    settings: Res<FlameSoundSettings>,
    vols: Res<crate::SoundVolumes>,
    global_volume: Res<GlobalVolume>,
    mut sounds: Query<(&GlobalTransform, &mut SpatialAudioSink), With<FlameSound>>,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();
    for (gt, mut sink) in &mut sounds {
        let loudness = settings.loudness(ear.distance(gt.translation())) * vols.fire_crackle;
        if loudness <= 0.0 {
            if !sink.is_paused() {
                sink.pause();
            }
            continue;
        }
        if sink.is_paused() {
            sink.play();
        }
        sink.set_volume(Volume::Linear(loudness) * global_volume.volume);
    }
}

/// The debug panel's "Fire crackle" section.
#[derive(SystemParam)]
pub(crate) struct FlamesDebug<'w> {
    settings: ResMut<'w, FlameSoundSettings>,
}

impl FlamesDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let s = &mut *self.settings;
        ui.label("Every fire the map keeps burning (placed in the level editor) crackles, looped.");
        ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=3.0).text("volume"));
        ui.add(egui::Slider::new(&mut s.full_distance, 0.0f32..=20.0).text("full volume within (m)"));
        ui.add(egui::Slider::new(&mut s.max_distance, 1.0f32..=60.0).text("silent beyond (m)"));
        ui.add(egui::Slider::new(&mut s.falloff, 0.1f32..=5.0).text("falloff curve"));
        if ui.button("Reset").clicked() {
            *s = FlameSoundSettings::default();
        }
    }
}

/// Flicker every fire's light.
fn flicker_flames(time: Res<Time>, mut lights: Query<(&FlameLight, &mut PointLight)>) {
    let t = time.elapsed_secs();
    for (light, mut point) in &mut lights {
        point.intensity = light.lumens * flicker(t, light.seed, light.flicker);
    }
}
