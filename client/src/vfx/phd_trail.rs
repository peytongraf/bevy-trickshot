//! PhD Flopper's slide trail: anyone sliding with the perk leaves a trail of
//! purple fire behind them that burns a moment and fades out over a few
//! seconds. Every client draws it for itself, for everyone — us (our own
//! `Slide`) and every other lobby member (their replicated
//! `PlayerPose::sliding`, and the perk in their replicated
//! `LobbyMember::perks`) — so the whole lobby sees the same trails without
//! anything extra on the wire.
//!
//! Two layers: a bed of glowing purple puffs (camera-facing sprites of the
//! explosion's fire puffs, `ExplosionAssets`), and over it real licking
//! flames — the molotov's fire (`molotov::FireMaterial`) in purple — that
//! flare up and die down. Each is `StateScoped(InGame)`, and where each
//! slider last dropped one is forgotten as soon as they stop sliding, so
//! nothing carries over between slides or games.

use std::collections::HashMap;

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use lightyear::prelude::{LocalId, PeerId};
use shared::perks::Perk;

use super::explosion::ExplosionAssets;
use super::smoke_billboard_rotation;
use crate::net::GameClient;
use crate::player::{Player, PlayerHead, Slide, Stance};
use crate::util::{rand01, rand_roll};
use crate::{AppState, EYE_HEIGHT};

pub(crate) struct PhdTrailPlugin;

impl Plugin for PhdTrailPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (lay_phd_trails, (update_phd_flames, update_trail_fires))
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// A flame drops every this many metres a slider travels...
const SPACING: f32 = 0.22;
/// ...two or three side by side, scattered across this width (m).
const SPREAD: f32 = 0.35;
const PER_DROP: u32 = 2;
/// Seconds a flame lives: it burns bright at first, then dims and fades.
const LIFETIME: f32 = 2.6;
/// Its size (m) as it appears, at its fullest, and its peak glow (HDR).
const SIZE_START: f32 = 0.35;
const SIZE_PEAK: f32 = 0.75;
const BRIGHTNESS: f32 = 3.5;
/// How fast (m/s) it drifts up as it burns.
const RISE: f32 = 0.35;
/// Most flames alive at once (every trail together).
const MAX_FLAMES: usize = 600;

/// The licking fire on top: one flame every this many puff clusters...
const FIRE_EVERY: u32 = 2;
/// ...this wide and tall (m, each a little smaller or bigger)...
const FIRE_WIDTH: f32 = 0.7;
const FIRE_HEIGHT: f32 = 0.85;
/// ...this bright (the molotov's own fire is 0.8)...
const FIRE_BRIGHTNESS: f32 = 1.1;
/// ...flaring up over this long (s), and burning about this long in all.
const FIRE_GROW: f32 = 0.15;
const FIRE_LIFETIME: f32 = 2.2;
/// Most of these alive at once.
const MAX_FIRES: usize = 200;

/// One licking flame in a trail (its own material, for its own fade).
#[derive(Component)]
struct TrailFire {
    age: f32,
    lifetime: f32,
    size: Vec2,
    seed: f32,
}

/// One flame in a trail.
#[derive(Component)]
struct PhdFlame {
    age: f32,
    lifetime: f32,
    roll: f32,
    spin: f32,
    size: f32,
    /// Per-flame 0..1 variation (its colour, how quickly it cools).
    vary: f32,
}

/// Drop flames behind everyone sliding with PhD Flopper, one cluster each
/// [`SPACING`] metres they've slid. (Not during a kill cam — the world on
/// screen then is the replay's.)
#[allow(clippy::too_many_arguments)]
fn lay_phd_trails(
    slide: Res<Slide>,
    classic: Res<crate::zombies_hud::ClassicPerks>,
    killcam: Res<crate::killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    poses: Query<(&shared::PlayerPose, &shared::PlayerId)>,
    assets: Option<Res<ExplosionAssets>>,
    (fire_assets, molotov, mut fire_materials, fires): (
        Option<Res<crate::molotov::MolotovAssets>>,
        Res<crate::molotov::MolotovSettings>,
        ResMut<Assets<crate::molotov::FireMaterial>>,
        Query<(), With<TrailFire>>,
    ),
    flames: Query<(), With<PhdFlame>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    // Where each slider last dropped flames (their feet).
    mut last: Local<HashMap<PeerId, Vec3>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    let me = local.iter().next().map(|l| l.0);
    let lobby = me.and_then(|me| lobbies.iter().find(|l| l.has(me)));

    // Everyone sliding with the perk right now, and where their feet are.
    let mut sliders: Vec<(PeerId, Vec3)> = Vec::new();
    if killcam.0.is_none() {
        if let (Some(me), Some(player)) = (me, &player) {
            if slide.stance == Stance::Sliding && classic.has(Perk::PhdFlopper) {
                sliders.push((me, player.translation - Vec3::Y * EYE_HEIGHT));
            }
        }
        if let Some(lobby) = lobby {
            for (pose, id) in &poses {
                if Some(id.0) == me || !pose.alive || !pose.sliding {
                    continue;
                }
                let has_phd = lobby
                    .members
                    .iter()
                    .any(|m| m.peer == id.0 && m.perks.contains(&Perk::PhdFlopper));
                if has_phd {
                    sliders.push((id.0, pose.translation - Vec3::Y * EYE_HEIGHT));
                }
            }
        }
    }
    // Stopped sliding: forget where they were, so the next slide starts fresh.
    last.retain(|peer, _| sliders.iter().any(|(p, _)| p == peer));

    let mut budget = MAX_FLAMES.saturating_sub(flames.iter().count());
    let mut fire_budget = MAX_FIRES.saturating_sub(fires.iter().count());
    for (peer, feet) in sliders {
        let from = *last.entry(peer).or_insert(feet);
        let travelled = Vec3::new(feet.x - from.x, 0.0, feet.z - from.z);
        let dist = travelled.length();
        if dist < SPACING {
            continue;
        }
        let dir = travelled / dist;
        let side = Vec3::new(-dir.z, 0.0, dir.x);
        // One cluster per spacing covered since the last (a fast slide in a
        // slow frame still leaves an even line).
        let steps = (dist / SPACING).floor() as u32;
        for step in 1..=steps.min(8) {
            let at = from + dir * (step as f32 * SPACING);
            // A licking purple flame over every few clusters.
            *seq = seq.wrapping_add(1);
            if let (Some(fire_assets), true) = (&fire_assets, *seq % FIRE_EVERY == 0 && fire_budget > 0) {
                fire_budget -= 1;
                let r = seq.wrapping_mul(2_246_822_519);
                let k = 0.75 + 0.5 * rand01(r ^ 0x2c);
                let size = Vec2::new(FIRE_WIDTH * k, FIRE_HEIGHT * k * (0.8 + 0.4 * rand01(r ^ 0x4d)));
                let seed = rand01(r ^ 0x5d);
                commands.spawn((
                    StateScoped(AppState::InGame),
                    TrailFire {
                        age: 0.0,
                        lifetime: FIRE_LIFETIME * (0.8 + 0.4 * rand01(r ^ 0x1234)),
                        size,
                        seed,
                    },
                    Mesh3d(fire_assets.quad.clone()),
                    MeshMaterial3d(
                        fire_materials.add(crate::molotov::FireMaterial::phd_flame(&molotov, FIRE_BRIGHTNESS, 0.0)),
                    ),
                    // (Its base on the ground; it grows from nothing.)
                    Transform::from_translation(at + side * (rand01(r ^ 0x51) - 0.5) * SPREAD * 0.6)
                        .with_scale(crate::molotov::seeded_scale(size * 0.01, seed)),
                    NotShadowCaster,
                    // The quad's drawn well outside its own bounds
                    // (billboarded, base on the anchor).
                    NoFrustumCulling,
                ));
            }
            for _ in 0..PER_DROP {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                *seq = seq.wrapping_add(1);
                let r = seq.wrapping_mul(2_654_435_761);
                let offset = side * (rand01(r ^ 0x51) - 0.5) * SPREAD + Vec3::Y * (0.08 + 0.12 * rand01(r ^ 0x77));
                let vary = rand01(r ^ 0x3131);
                let material = materials.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                    base_color_texture: Some(assets.fire[(rand01(r ^ 0x61) * 2.0) as usize % 2].clone()),
                    unlit: true,
                    alpha_mode: AlphaMode::Add,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                commands.spawn((
                    StateScoped(AppState::InGame),
                    PhdFlame {
                        age: 0.0,
                        lifetime: LIFETIME * (0.75 + 0.5 * rand01(r ^ 0x1234)),
                        roll: rand_roll(r ^ 0xab),
                        spin: (rand01(r ^ 0xcd) * 2.0 - 1.0) * 1.5,
                        size: 0.8 + 0.4 * rand01(r ^ 0x2c),
                        vary,
                    },
                    Mesh3d(assets.quad.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(at + offset).with_scale(Vec3::splat(SIZE_START)),
                    NoFrustumCulling,
                    NotShadowCaster,
                ));
            }
        }
        last.insert(peer, from + dir * (steps as f32 * SPACING));
    }
}

/// Burn, rise, face the camera and fade each flame; despawn it (and free its
/// material) once it's out.
#[allow(clippy::type_complexity)]
fn update_phd_flames(
    time: Res<Time>,
    player: Option<Single<&Transform, (With<Player>, Without<PhdFlame>)>>,
    head: Option<Single<&Transform, (With<PlayerHead>, Without<PhdFlame>)>>,
    mut flames: Query<(Entity, &mut PhdFlame, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let (Some(player), Some(head)) = (player, head) else {
        return;
    };
    let dt = time.delta_secs();
    let cam_pos = player.translation;
    let cam_rot = player.rotation * head.rotation;
    let (cam_up, cam_right) = (cam_rot * Vec3::Y, cam_rot * Vec3::X);
    for (entity, mut flame, mut tf, material) in &mut flames {
        flame.age += dt;
        if flame.age >= flame.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        let f = flame.age / flame.lifetime;
        // Flares up fast, then shrinks a little as it dies down.
        let grow = (flame.age / 0.12).min(1.0);
        let size = (SIZE_START + (SIZE_PEAK - SIZE_START) * grow) * (1.0 - 0.35 * f) * flame.size;
        tf.scale = Vec3::splat(size.max(1.0e-4));
        tf.translation.y += RISE * dt * (1.0 - f);
        tf.rotation = smoke_billboard_rotation(tf.translation, cam_pos, cam_up, cam_right, flame.roll + flame.spin * flame.age);

        // Hot pink-white at the start, cooling through violet to a deep
        // purple as it fades out.
        let t = (f * (1.1 - 0.3 * flame.vary)).clamp(0.0, 1.0);
        let c = if t < 0.2 {
            lerp3([1.0, 0.75, 1.0], [0.75, 0.2, 1.0], t / 0.2)
        } else {
            lerp3([0.75, 0.2, 1.0], [0.3, 0.04, 0.55], (t - 0.2) / 0.8)
        };
        let b = BRIGHTNESS * (1.0 - f).powf(1.4) * (flame.age / 0.04).min(1.0);
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::LinearRgba(LinearRgba::new(c[0] * b, c[1] * b, c[2] * b, 1.0));
        }
    }
}

/// Flare each licking flame up, let it burn, then sink and fade it out;
/// despawn it (and free its material) once it's out.
fn update_trail_fires(
    time: Res<Time>,
    mut fires: Query<(Entity, &mut TrailFire, &mut Transform, &MeshMaterial3d<crate::molotov::FireMaterial>)>,
    mut materials: ResMut<Assets<crate::molotov::FireMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut fire, mut tf, material) in &mut fires {
        fire.age += dt;
        if fire.age >= fire.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        let grow = (fire.age / FIRE_GROW).min(1.0);
        // Burning full for the first part, then dying down.
        let f = fire.age / fire.lifetime;
        let fade = grow * (1.0 - ((f - 0.35) / 0.65).clamp(0.0, 1.0)).powf(1.2);
        tf.scale = crate::molotov::seeded_scale(fire.size * (0.3 + 0.7 * fade).max(0.01), fire.seed);
        if let Some(m) = materials.get_mut(&material.0) {
            m.set_fade(fade);
        }
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
