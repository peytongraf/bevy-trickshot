//! The fires a map's layout keeps burning (`shared::level::Flame`) — in a
//! wall-mounted bowl, say: a few overlapping flame quads in the molotov's own
//! fire ([`FireMaterial`]) and a flickering light, lit the whole time, power
//! on or off, day or night.
//!
//! A fire marked to (`Flame::shadows` — the big ones, whose light would
//! otherwise shine through walls and floors into the rooms around them)
//! casts shadows on its turn: the ones in view and nearest first, as many as
//! the shadow quality allows, those out of view fading out rather than shining
//! through the walls ([`limit_flame_shadows`], `crate::light_budget`).
//!
//! In a game each also crackles — `GameSounds::fire_crackle`, looped from the
//! fire, fading with distance by the "Fire" profile in the audio mixer (and
//! held silent out of earshot, so far-off fires cost nothing).
//!
//! While Der Tempel's easter egg has gone wrong they burn green, light and
//! all, and gutter as every light stutters ([`tint_flames`],
//! `environment::egg_mood`) — back to themselves once it's over.
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
        app.add_systems(
            Update,
            (sync_game_flames, tint_flames, flicker_flames, limit_flame_shadows).chain(),
        );
    }
}

/// On a fire's looping crackle.
#[derive(Component)]
struct FlameSound;

/// Each flame's overlapping layers: `(width, height)` relative to its size,
/// and its seed — so it has depth instead of reading as one flat sprite.
const LAYERS: [(f32, f32, f32); 3] = [(1.0, 1.0, 0.17), (0.72, 0.88, 0.53), (0.5, 0.68, 0.81)];

/// On a fire's root: its material (its own, freed when it goes), and its
/// brightness.
#[derive(Component)]
pub(crate) struct MapFlame(Handle<FireMaterial>, f32);

/// On a fire's light.
#[derive(Component)]
struct FlameLight {
    lumens: f32,
    flicker: f32,
    seed: f32,
    /// May cast shadows, out to its range (m).
    shadows: bool,
    range: f32,
    /// How far up it is on its turn at casting shadows (`light_budget`).
    budget_fade: f32,
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
                MapFlame(material.clone(), flame.brightness),
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
                        shadows: flame.shadows,
                        range: flame.range,
                        budget_fade: 1.0,
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

/// How much nearer (m) another fire has to be to take a shadow over from
/// one casting it — so two about as far off don't keep swapping.
const SHADOW_SWAP_MARGIN: f32 = 3.0;

/// Let the few shadow-casting fires in view and whose reach we're nearest
/// cast shadows (as many as `ShadowQuality::fire_shadows`); in a game those
/// out of view fade out rather than shine through the walls
/// (`crate::light_budget`).
/// In the level editor they just stay on, so a fire can always be seen where
/// it's being placed.
#[allow(clippy::too_many_arguments)]
fn limit_flame_shadows(
    settings: Res<crate::settings::Settings>,
    state: Res<State<AppState>>,
    time: Res<Time>,
    rapier: bevy_rapier3d::prelude::ReadRapierContext,
    camera: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    mut lights: Query<(Entity, &mut FlameLight, &GlobalTransform, &mut PointLight)>,
    mut chosen: Local<Vec<Entity>>,
    mut last_seen: Local<std::collections::HashMap<Entity, f32>>,
) {
    if lights.is_empty() {
        chosen.clear();
        last_seen.clear();
        return;
    }
    let turns = settings.shadow_quality.fire_shadows();
    let now = time.elapsed_secs();
    let eye = camera.single().ok().map(GlobalTransform::translation);
    let mut nearest: Vec<(f32, Entity)> = lights
        .iter()
        .filter(|(_, light, ..)| light.shadows)
        .map(|(e, light, at, _)| {
            let at = at.translation();
            let held = chosen.contains(&e);
            let score = eye.map_or(0.0, |eye| {
                // How far we are from the edge of its reach (negative inside
                // it): standing just outside it, the walls nearer the fire
                // are still in it.
                let distance = eye.distance(at) - light.range;
                crate::light_budget::score(&rapier, &mut last_seen, e, eye, at, now, distance, held, SHADOW_SWAP_MARGIN)
            });
            (score, e)
        })
        .collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    *chosen = nearest.into_iter().take(turns).map(|(_, e)| e).collect();
    let in_game = *state.get() == AppState::InGame;
    let dt = time.delta_secs();
    for (e, mut light, _, mut point) in &mut lights {
        let has_turn = chosen.contains(&e);
        let wanted = if !light.shadows {
            false
        } else if in_game {
            let in_view = crate::light_budget::seen(&last_seen, &e, now);
            crate::light_budget::step_fade(&mut light.budget_fade, has_turn, in_view, turns, dt)
        } else {
            light.budget_fade = 1.0;
            has_turn
        };
        if point.shadows_enabled != wanted {
            point.shadows_enabled = wanted;
        }
    }
}

/// Turn the fires green as the easter egg goes wrong (and back after), and
/// gutter their flames as the lights stutter — only when either moves (or a
/// fire's new).
fn tint_flames(
    fade: Res<crate::environment::EggMoodFade>,
    stutter: Res<crate::environment::LightFlicker>,
    look: Res<crate::environment::EggMoodNow>,
    molotov: Res<MolotovSettings>,
    flames: Query<&MapFlame>,
    new: Query<(), Added<MapFlame>>,
    mut materials: ResMut<Assets<FireMaterial>>,
) {
    if !fade.is_changed() && !stutter.is_changed() && !look.is_changed() && new.is_empty() {
        return;
    }
    // (A flame gutters rather than going out with the lights.)
    let gutter = 0.35 + 0.65 * stutter.0;
    let toward = [look.fire_core, look.fire_mid, look.fire_tip];
    for flame in &flames {
        if let Some(m) = materials.get_mut(&flame.0) {
            *m = FireMaterial::flame_tinted(&molotov, flame.1 * gutter, toward, fade.0);
        }
    }
}

/// Flicker every fire's light (green, as far as the easter egg's gone
/// wrong).
fn flicker_flames(
    time: Res<Time>,
    molotov: Res<MolotovSettings>,
    fade: Res<crate::environment::EggMoodFade>,
    look: Res<crate::environment::EggMoodNow>,
    mut lights: Query<(&FlameLight, &mut PointLight)>,
) {
    let t = time.elapsed_secs();
    let color = crate::util::color_from_parts(crate::environment::blend3(molotov.light_color, look.fire_light, fade.0));
    for (light, mut point) in &mut lights {
        point.intensity = light.lumens * flicker(t, light.seed, light.flicker) * light.budget_fade;
        point.color = color;
    }
}
