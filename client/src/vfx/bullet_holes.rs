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
//! blended material. Not shown again during a kill cam replay (replays don't
//! re-emit it); the holes that are already there stay put.

use std::f32::consts::TAU;

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::util::rand01;
use crate::AppState;

/// Seconds a bullet hole stays before it's removed.
pub(crate) const BULLET_HOLE_LIFETIME_SECS: f32 = 60.0;
/// Most holes alive at once; past this the oldest is removed to make room.
const MAX_BULLET_HOLES: usize = 300;
/// Distance (m) a hole floats off the surface, against z-fighting.
const SURFACE_OFFSET: f32 = 0.004;

/// A shot hit a surface at `point`, whose (unit) outward `normal` faces the
/// shooter. Consumed by [`spawn_bullet_holes`].
#[derive(Event)]
pub(crate) struct BulletImpact {
    pub(crate) point: Vec3,
    pub(crate) normal: Vec3,
}

/// Panel-adjustable bullet-hole look (`ads_tuning_ui`'s "Bullet impacts"
/// section). Applied to every hole already in the world too, not just new ones.
#[derive(Resource)]
pub(crate) struct BulletHoleSettings {
    /// Edge length (m) of the square the texture is stretched over.
    pub(crate) size: f32,
}

impl Default for BulletHoleSettings {
    fn default() -> Self {
        Self { size: 0.4 }
    }
}

/// One bullet hole in the world.
#[derive(Component)]
struct BulletHole {
    /// `Time::elapsed_secs()` it was placed.
    spawned_at: f32,
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

/// Stick a hole on every [`BulletImpact`].
fn spawn_bullet_holes(
    time: Res<Time>,
    settings: Res<BulletHoleSettings>,
    assets: Option<Res<BulletHoleAssets>>,
    mut events: EventReader<BulletImpact>,
    holes: Query<(Entity, &BulletHole)>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else {
        events.clear();
        return;
    };
    let mut count = holes.iter().count();
    for ev in events.read() {
        let normal = ev.normal.normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
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
        *seq = seq.wrapping_add(1);
        let roll = rand01(seq.wrapping_mul(0x9E37_79B9) ^ time.elapsed().subsec_nanos()) * TAU;
        // Lie the quad (whose normal is +Y) in the surface, then spin it about
        // the surface normal by a random roll so repeated hits differ.
        let rotation =
            Quat::from_axis_angle(normal, roll) * Quat::from_rotation_arc(Vec3::Y, normal);
        commands.spawn((
            BulletHole {
                spawned_at: time.elapsed_secs(),
            },
            StateScoped(AppState::InGame),
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            Transform {
                translation: ev.point + normal * SURFACE_OFFSET,
                rotation,
                scale: Vec3::splat(settings.size.max(1.0e-3)),
            },
            NotShadowCaster,
        ));
        count += 1;
    }
}

/// Push a changed [`BulletHoleSettings::size`] onto every hole in the world.
fn apply_bullet_hole_scale(
    settings: Res<BulletHoleSettings>,
    mut holes: Query<&mut Transform, With<BulletHole>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut tf in &mut holes {
        tf.scale = Vec3::splat(settings.size.max(1.0e-3));
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
