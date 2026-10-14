//! The fires a map's layout keeps burning (`shared::level::Flame`) — in a
//! wall-mounted bowl, say: a few overlapping flame quads in the molotov's own
//! fire ([`FireMaterial`]) and a flickering light, lit the whole time, power
//! on or off, day or night.
//!
//! In a game each also crackles — `GameSounds::fire_crackle`, looped from the
//! fire, fading with distance by the "Fire" profile in the audio mixer (and
//! held silent out of earshot, so far-off fires cost nothing).
//!
//! In a game they're the current map's ([`sync_game_flames`], all
//! `StateScoped(InGame)`); in the level editor its own copy follows the
//! layout being edited (`level_editor`, through [`sync_flames`]). Each set is
//! rebuilt whenever its list changes, so nothing carries over between games.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use bevy::render::view::NoFrustumCulling;
use shared::level::Flame;

use crate::molotov::{flicker, seeded_scale, FireMaterial, MolotovAssets, MolotovSettings};
use crate::util::rand01;
use crate::{AppState, CurrentMap};

pub(crate) struct FlamesPlugin;

impl Plugin for FlamesPlugin {
    fn build(&self, app: &mut App) {
        // (Ungated: leaving the game is what takes them away.)
        app.add_systems(Update, (sync_game_flames, flicker_flames));
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
                        AudioPlayer::new(crackle.clone()),
                        Transform::from_xyz(0.0, flame.size * 0.5, 0.0),
                        PlaybackSettings::LOOP.with_spatial(true),
                    ));
                }
            });
    }
}

/// Flicker every fire's light.
fn flicker_flames(time: Res<Time>, mut lights: Query<(&FlameLight, &mut PointLight)>) {
    let t = time.elapsed_secs();
    for (light, mut point) in &mut lights {
        point.intensity = light.lumens * flicker(t, light.seed, light.flicker);
    }
}
