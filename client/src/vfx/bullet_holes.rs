//! Bullet holes: `textures/vfx/bullet_hole.png` stuck flat on whatever surface a
//! shot hit, for everyone in the lobby, gone again after a minute.
//!
//! Where a shot lands comes from the same two places as the ground dust: the
//! shooter's own instant local hit (`weapon::resolve_local_shot`, a rapier ray
//! against the client's map colliders) and, for everyone else's shots, the
//! server's authoritative `ShotResolved` (`net::receive_shots` — the server
//! stops the bullet on the first surface of the very same collision mesh and
//! sends the surface normal along). Both fire a [`BulletImpact`].
//!
//! Each hole is a unit quad lying in the surface's plane — turned to face out
//! along the normal, a random roll about it, and nudged a few millimetres off
//! the surface (plus a small depth bias) so it never z-fights — sharing one
//! blended material.
//!
//! Holes only go on the map: placed objects (perk machines, the ammo crate,
//! ...) are solid through a plain box around their model ([`NoBulletHoles`])
//! that wouldn't match the model, so a hit on one still kicks up dust but
//! leaves no hole. And before a hole goes down, a few short rays probe the
//! surface under its edges ([`fit_hole`]): one hanging off an edge or round a
//! corner is slid back onto the face, or made smaller, or (on something too
//! thin) left out. Not shown again during a kill cam replay (replays don't
//! re-emit it); the holes that are already there stay put.

use std::f32::consts::TAU;

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy_rapier3d::prelude::{QueryFilter, RapierContext, ReadRapierContext};

use crate::util::rand01;
use crate::AppState;

/// Seconds a bullet hole stays before it's removed.
pub(crate) const BULLET_HOLE_LIFETIME_SECS: f32 = 60.0;
/// Most holes alive at once; past this the oldest is removed to make room.
const MAX_BULLET_HOLES: usize = 300;
/// Distance (m) a hole floats off the surface, against z-fighting.
const SURFACE_OFFSET: f32 = 0.004;
/// How far (m) back along the shot, and on past where it stopped, to look
/// for a [`NoBulletHoles`] box it landed on — slack for the server's box
/// differing a little from ours. (Only a little past, so a machine behind a
/// thin wall doesn't take the wall's hole away.)
const NO_HOLES_BACK: f32 = 0.5;
const NO_HOLES_PAST: f32 = 0.15;
/// Edge probes start this far (m) off the surface and look this far either
/// side of it...
const PROBE_LIFT: f32 = 0.05;
/// ...and count as the same face if they meet it within this much (m) of
/// the hole's plane...
const PROBE_DEPTH_TOLERANCE: f32 = 0.025;
/// ...turned at most this far from it (cosine).
const PROBE_MIN_ALIGN: f32 = 0.9;
/// Where the probes go, as fractions of the hole's half size along its two
/// sides: its middle, the middle of each edge and toward each corner (the
/// texture's round, so not the very corners).
const PROBE_POINTS: [Vec2; 9] = [
    Vec2::ZERO,
    Vec2::new(0.85, 0.0),
    Vec2::new(-0.85, 0.0),
    Vec2::new(0.0, 0.85),
    Vec2::new(0.0, -0.85),
    Vec2::new(0.6, 0.6),
    Vec2::new(-0.6, 0.6),
    Vec2::new(0.6, -0.6),
    Vec2::new(-0.6, -0.6),
];
/// Sizes (× [`BulletHoleSettings::size`]) a hole that doesn't fit is tried at,
/// biggest first...
const FIT_SCALES: [f32; 3] = [1.0, 0.75, 0.5];
/// ...each slid back onto the face by up to these fractions of its half size.
const FIT_SHIFTS: [f32; 3] = [0.0, 0.25, 0.5];

/// A shot travelling along `dir` hit a surface at `point`, whose (unit)
/// outward `normal` faces the shooter. Consumed by [`spawn_bullet_holes`].
#[derive(Event)]
pub(crate) struct BulletImpact {
    pub(crate) point: Vec3,
    pub(crate) normal: Vec3,
    pub(crate) dir: Vec3,
}

/// On a placed object's solid box: shots stop on it (and kick up dust) but
/// never leave a bullet hole — the box doesn't match the model inside it.
#[derive(Component)]
pub(crate) struct NoBulletHoles;

/// Panel-adjustable bullet-hole look (`ads_tuning_ui`'s "Bullet impacts"
/// section). Applied to every hole already in the world too, not just new ones.
#[derive(Resource)]
pub(crate) struct BulletHoleSettings {
    /// Edge length (m) of the square the texture is stretched over.
    pub(crate) size: f32,
}

impl Default for BulletHoleSettings {
    fn default() -> Self {
        Self { size: 0.24 }
    }
}

/// One bullet hole in the world.
#[derive(Component)]
struct BulletHole {
    /// `Time::elapsed_secs()` it was placed.
    spawned_at: f32,
    /// Its size, × [`BulletHoleSettings::size`] — under 1 when it had to be
    /// shrunk to fit its face ([`fit_hole`]).
    scale: f32,
}

#[derive(Resource)]
struct BulletHoleAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

pub struct BulletHolePlugin;

impl Plugin for BulletHolePlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<BulletImpact>()
            .init_resource::<BulletHoleSettings>()
            // `PostStartup`, not `Startup` — see `killcam::KillCamPlugin`'s note.
            .add_systems(PostStartup, setup_bullet_hole_assets)
            .add_systems(
                Update,
                (spawn_bullet_holes, apply_bullet_hole_scale, expire_bullet_holes)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn setup_bullet_hole_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(BulletHoleAssets {
        // A 1 × 1 quad facing +Y; scaled to `BulletHoleSettings::size`.
        mesh: meshes.add(Plane3d::default().mesh().size(1.0, 1.0)),
        material: materials.add(StandardMaterial {
            base_color_texture: Some(asset_server.load("textures/vfx/bullet_hole.png")),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            double_sided: true,
            cull_mode: None,
            // Pulled a little toward the camera on top of the physical offset,
            // so it holds up against the surface at range.
            depth_bias: 2.0,
            ..default()
        }),
    });
}

/// The nearest bit of the map along `dir` from `from`, within `max` (m):
/// how far, and its normal. (Not the placed objects' boxes — holes never sit
/// on those.)
fn probe(
    from: Vec3,
    dir: Vec3,
    max: f32,
    rapier: Option<&RapierContext>,
    boxes: &Query<(), With<NoBulletHoles>>,
) -> Option<(f32, Vec3)> {
    let not_box = |e: Entity| !boxes.contains(e);
    rapier.and_then(|r| {
        r.cast_ray_and_get_normal(from, dir, max, true, QueryFilter::default().exclude_sensors().predicate(&not_box))
            .map(|(_, hit)| (hit.time_of_impact, hit.normal))
    })
}

/// Where a hole centred at `center` on a face with outward `normal` (sides
/// along `u` and `v`) can go so it lies wholly on that face rather than
/// hanging off its edge — its centre and size (× the settings' size), or
/// `None` if even the smallest try doesn't fit.
fn fit_hole(
    center: Vec3,
    normal: Vec3,
    (u, v): (Vec3, Vec3),
    size: f32,
    rapier: Option<&RapierContext>,
    boxes: &Query<(), With<NoBulletHoles>>,
) -> Option<(Vec3, f32)> {
    for scale in FIT_SCALES {
        let half = size * 0.5 * scale;
        // Which probes miss the face with the hole where it was hit, and
        // which way that says to slide it (away from them).
        let on_face = |c: Vec3, at: Vec2| {
            let p = c + (u * at.x + v * at.y) * half;
            probe(p + normal * PROBE_LIFT, -normal, PROBE_LIFT * 2.0, rapier, boxes).is_some_and(
                |(d, n)| (d - PROBE_LIFT).abs() <= PROBE_DEPTH_TOLERANCE && n.dot(normal).abs() >= PROBE_MIN_ALIGN,
            )
        };
        let away: Vec2 = PROBE_POINTS.iter().filter(|&&at| !on_face(center, at)).map(|&at| -at).sum();
        if away == Vec2::ZERO {
            return Some((center, scale));
        }
        let away = away.normalize_or_zero();
        if away == Vec2::ZERO {
            continue;
        }
        let slide = (u * away.x + v * away.y) * half;
        for shift in FIT_SHIFTS.into_iter().skip(1) {
            let c = center + slide * shift;
            if PROBE_POINTS.iter().all(|&at| on_face(c, at)) {
                return Some((c, scale));
            }
        }
    }
    None
}

/// Stick a hole on every [`BulletImpact`].
#[allow(clippy::too_many_arguments)]
fn spawn_bullet_holes(
    time: Res<Time>,
    settings: Res<BulletHoleSettings>,
    assets: Option<Res<BulletHoleAssets>>,
    mut events: EventReader<BulletImpact>,
    current_map: Res<crate::CurrentMap>,
    holes: Query<(Entity, &BulletHole)>,
    rapier: ReadRapierContext,
    boxes: Query<(), With<NoBulletHoles>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else {
        events.clear();
        return;
    };
    let rapier = rapier.single().ok();
    let rapier = rapier.as_ref();
    let size = settings.size.max(1.0e-3);
    let mut count = holes.iter().count();
    for ev in events.read() {
        let normal = ev.normal.normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        // Under a pool: none (the water's drawn over it anyway — its splash
        // is `water_wake`'s).
        if crate::water::underwater(current_map.0, ev.point + normal * 0.01) {
            continue;
        }
        let dir = Some(ev.dir.normalize_or_zero()).filter(|d| *d != Vec3::ZERO).unwrap_or(-normal);
        // Landed on a placed object's box: dust, but no hole.
        let on_box = rapier.is_some_and(|r| {
            let only_boxes = |e: Entity| boxes.contains(e);
            r.cast_ray(
                ev.point - dir * NO_HOLES_BACK,
                dir,
                NO_HOLES_BACK + NO_HOLES_PAST,
                true,
                QueryFilter::default().predicate(&only_boxes),
            )
            .is_some()
        });
        if on_box {
            continue;
        }
        *seq = seq.wrapping_add(1);
        let roll = rand01(seq.wrapping_mul(0x9E37_79B9) ^ time.elapsed().subsec_nanos()) * TAU;
        // Lie the quad (whose normal is +Y) in the surface, then spin it about
        // the surface normal by a random roll so repeated hits differ.
        let rotation =
            Quat::from_axis_angle(normal, roll) * Quat::from_rotation_arc(Vec3::Y, normal);
        let sides = (rotation * Vec3::X, rotation * Vec3::Z);
        let Some((center, scale)) = fit_hole(ev.point, normal, sides, size, rapier, &boxes) else {
            continue;
        };
        // Over the cap: drop the oldest hole to make room.
        if count >= MAX_BULLET_HOLES {
            if let Some((oldest, _)) = holes
                .iter()
                .min_by(|a, b| a.1.spawned_at.total_cmp(&b.1.spawned_at))
            {
                commands.entity(oldest).try_despawn();
                count -= 1;
            }
        }
        commands.spawn((
            BulletHole {
                spawned_at: time.elapsed_secs(),
                scale,
            },
            StateScoped(AppState::InGame),
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            Transform {
                translation: center + normal * SURFACE_OFFSET,
                rotation,
                scale: Vec3::splat(size * scale),
            },
            NotShadowCaster,
        ));
        count += 1;
    }
}

/// Push a changed [`BulletHoleSettings::size`] onto every hole in the world.
fn apply_bullet_hole_scale(
    settings: Res<BulletHoleSettings>,
    mut holes: Query<(&mut Transform, &BulletHole)>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut tf, hole) in &mut holes {
        tf.scale = Vec3::splat(settings.size.max(1.0e-3) * hole.scale);
    }
}

/// Remove holes that have been there [`BULLET_HOLE_LIFETIME_SECS`].
fn expire_bullet_holes(
    time: Res<Time>,
    holes: Query<(Entity, &BulletHole)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (entity, hole) in &holes {
        if now - hole.spawned_at >= BULLET_HOLE_LIFETIME_SECS {
            commands.entity(entity).try_despawn();
        }
    }
}
