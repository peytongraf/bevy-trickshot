//! Bullet-impact particles: the rock + dust burst kicked up by a ground hit,
//! and the blood squirt from a bot hit — both built on the same billboarded,
//! gravity/drag-integrated `ImpactParticle`.

use std::f32::consts::PI;

use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;

use crate::player::{Player, PlayerHead};
use crate::util::{rand01, rand_roll, srgb_parts};
use crate::AppState;

/// Hard cap on live bullet-impact particles (rocks + dust together).
pub(crate) const IMPACT_MAX: usize = 400;

/// `rocks.png` is a loose grid of rocks; each rock sprite shows one cell of this
/// many columns × rows so it's a single rock, not the whole sheet.
pub(crate) const ROCK_COLS: u32 = 3;
pub(crate) const ROCK_ROWS: u32 = 4;

/// Server → everyone: a shot hit the ground at this world point. Consumed by
/// `spawn_ground_impact`, which kicks up a short rock + dust burst there.
#[derive(Event)]
pub(crate) struct GroundImpact(pub(crate) Vec3);

/// One rock or dust sprite from a ground impact. World-space, billboarded at the
/// camera; rocks arc under `gravity`, dust drifts and swells with `drag`.
#[derive(Component)]
pub(crate) struct ImpactParticle {
    velocity: Vec3,
    /// Downward acceleration (m/s²). Rocks fall; dust is ~0.
    gravity: f32,
    /// Per-second velocity damping. Dust high (billows then stalls); rocks ~0.
    drag: f32,
    age: f32,
    lifetime: f32,
    /// Seconds to ramp opacity 0 → `peak_alpha` before the fade-out.
    fade_in: f32,
    roll: f32,
    /// Roll spin (rad/s); rocks tumble, dust doesn't.
    spin: f32,
    /// Sprite size (m) at spawn and at end of life — dust grows, rocks hold.
    scale0: f32,
    scale1: f32,
    peak_alpha: f32,
    /// RGB the sprite's blended material is tinted with (multiplies the
    /// texture) — white for rocks / dust, `BloodSettings::color` for blood.
    tint: [f32; 3],
}

/// Shared quad + the impact textures (`spawn_ground_impact` / `spawn_blood_impact`
/// clone a fresh material per particle so each fades on its own). `blood` is a
/// transparent PNG whose droplets already carry their own colour.
#[derive(Resource)]
pub(crate) struct ImpactAssets {
    pub(crate) quad: Handle<Mesh>,
    pub(crate) dust: Handle<Image>,
    pub(crate) rocks: Handle<Image>,
    pub(crate) blood: Handle<Image>,
}

/// Panel-adjustable rock-debris burst for a ground hit.
#[derive(Resource)]
pub(crate) struct RockSettings {
    /// Rocks launched per impact.
    pub(crate) count: u32,
    /// Launch speed (m/s), randomised ±45%.
    pub(crate) speed: f32,
    /// Cone half-angle off straight-up (degrees).
    pub(crate) spread_deg: f32,
    /// Downward acceleration (m/s²).
    pub(crate) gravity: f32,
    /// Peak tumble rate (rad/s).
    pub(crate) spin: f32,
    /// Sprite size (m).
    pub(crate) scale: f32,
    /// Seconds a rock lives, randomised.
    pub(crate) lifetime: f32,
}

impl Default for RockSettings {
    fn default() -> Self {
        Self {
            count: 9,
            speed: 6.0,
            spread_deg: 32.0,
            gravity: 20.0,
            spin: 14.0,
            scale: 0.14,
            lifetime: 1.1,
        }
    }
}

/// Panel-adjustable dust puff for a ground hit.
#[derive(Resource)]
pub(crate) struct DustSettings {
    /// Puffs per impact.
    pub(crate) count: u32,
    /// Initial speed off the cone (m/s).
    pub(crate) speed: f32,
    /// Cone half-angle off straight-up (degrees).
    pub(crate) spread_deg: f32,
    /// Extra straight-up drift added to every puff (m/s).
    pub(crate) rise: f32,
    /// Per-second velocity damping.
    pub(crate) drag: f32,
    /// Sprite size at spawn / at end of life (m) — dust swells.
    pub(crate) start_scale: f32,
    pub(crate) end_scale: f32,
    /// Seconds a puff lives.
    pub(crate) lifetime: f32,
    /// Seconds to fade in.
    pub(crate) fade_in: f32,
    /// Peak opacity.
    pub(crate) opacity: f32,
}

impl Default for DustSettings {
    fn default() -> Self {
        Self {
            count: 12,
            speed: 2.4,
            spread_deg: 58.0,
            rise: 0.6,
            drag: 2.6,
            start_scale: 0.25,
            end_scale: 1.15,
            lifetime: 0.85,
            fade_in: 0.04,
            opacity: 0.5,
        }
    }
}

/// A shot connected with a bot at `point`, travelling along `dir` (unit).
/// Consumed by `spawn_blood_impact`, which squirts a blood burst out along the
/// shot from that point. Written by `net::follow_bot_avatars` when a
/// server-owned bot drops.
#[derive(Event)]
pub(crate) struct BloodImpact {
    pub(crate) point: Vec3,
    pub(crate) dir: Vec3,
}

/// Panel-adjustable blood squirt for a bot hit (`ads_tuning_ui`'s "Blood
/// splatter" section).
#[derive(Resource)]
pub(crate) struct BloodSettings {
    /// Droplets launched per hit.
    pub(crate) count: u32,
    /// Squirt speed (m/s), randomised.
    pub(crate) speed: f32,
    /// Spray cone half-angle around the shot direction (degrees).
    pub(crate) spread_deg: f32,
    /// Downward acceleration (m/s²) — droplets arc down.
    pub(crate) gravity: f32,
    /// Per-second velocity damping.
    pub(crate) drag: f32,
    /// Droplet sprite size at spawn (m), randomised.
    pub(crate) scale: f32,
    /// How much a droplet grows over its life (× `scale`).
    pub(crate) growth: f32,
    /// Seconds a droplet lives, randomised.
    pub(crate) lifetime: f32,
    /// Peak opacity, multiplying the texture's own alpha.
    pub(crate) opacity: f32,
    /// Multiplies the texture's colour — white keeps the PNG's own dark red.
    pub(crate) color: [f32; 3],
}

impl Default for BloodSettings {
    fn default() -> Self {
        Self {
            count: 40,
            speed: 6.0,
            spread_deg: 50.0,
            gravity: 17.0,
            drag: 1.4,
            scale: 0.2,
            growth: 2.0,
            lifetime: 1.0,
            opacity: 0.8,
            color: srgb_parts(Color::WHITE),
        }
    }
}

/// A unit vector inside a cone of half-angle `half` around `axis`, chosen
/// deterministically from `seed` (uniform over the cap).
pub(crate) fn cone_dir(axis: Vec3, half: f32, seed: u32) -> Vec3 {
    let a = rand01(seed.wrapping_mul(3)) * (PI * 2.0);
    let z = 1.0 - rand01(seed.wrapping_mul(7)) * (1.0 - half.cos());
    let r = (1.0 - z * z).max(0.0).sqrt();
    let local = Vec3::new(r * a.cos(), z, r * a.sin());
    if axis.abs_diff_eq(Vec3::Y, 1.0e-4) {
        local
    } else {
        Quat::from_rotation_arc(Vec3::Y, axis.normalize_or_zero()) * local
    }
}

/// Fresh unlit blended material for one impact sprite.
pub(crate) fn impact_material(texture: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(texture),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// Kick up a short rock + dust burst at every ground impact the server reported
/// this frame. World-space, so it's seen from every angle and by every client.
pub(crate) fn spawn_ground_impact(
    mut events: EventReader<GroundImpact>,
    assets: Res<ImpactAssets>,
    rocks: Res<RockSettings>,
    dust: Res<DustSettings>,
    existing: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let mut budget = IMPACT_MAX.saturating_sub(existing.iter().count());

    for ev in events.read() {
        // Nudge just above the surface so the sprites don't z-fight the ground.
        let at = ev.0 + Vec3::Y * 0.02;
        *seq = seq.wrapping_add(1);

        for i in 0..rocks.count {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let s = seq
                .wrapping_mul(2_654_435_761)
                .wrapping_add(i.wrapping_mul(40_503))
                .wrapping_add(0x11);
            let dir = cone_dir(Vec3::Y, rocks.spread_deg.to_radians(), s);
            let speed = rocks.speed * (0.55 + 0.45 * rand01(s ^ 0x9e37));
            // `rocks.png` is a sheet of ~25 rocks; show one 1/ROCK_COLS × 1/ROCK_ROWS
            // cell of it per particle so each sprite is a single rock, not the pile.
            let col = (rand01(s ^ 0x3) * ROCK_COLS as f32) as u32 % ROCK_COLS;
            let row = (rand01(s ^ 0x5) * ROCK_ROWS as f32) as u32 % ROCK_ROWS;
            let mut material = impact_material(assets.rocks.clone());
            material.uv_transform = Affine2::from_scale_angle_translation(
                Vec2::new(1.0 / ROCK_COLS as f32, 1.0 / ROCK_ROWS as f32),
                0.0,
                Vec2::new(col as f32 / ROCK_COLS as f32, row as f32 / ROCK_ROWS as f32),
            );
            commands.spawn((
                StateScoped(AppState::InGame),
                ImpactParticle {
                    velocity: dir * speed,
                    gravity: rocks.gravity,
                    drag: 0.0,
                    age: 0.0,
                    lifetime: rocks.lifetime.max(0.1) * (0.7 + 0.6 * rand01(s ^ 0x1234)),
                    fade_in: 0.0,
                    roll: rand_roll(s ^ 0x77),
                    spin: (rand01(s ^ 0xab) * 2.0 - 1.0) * rocks.spin,
                    scale0: rocks.scale,
                    scale1: rocks.scale,
                    peak_alpha: 1.0,
                    tint: [1.0, 1.0, 1.0],
                },
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(material)),
                Transform::from_translation(at).with_scale(Vec3::splat(rocks.scale.max(1.0e-4))),
                NoFrustumCulling,
            ));
        }

        for i in 0..dust.count {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let s = seq
                .wrapping_mul(40_503)
                .wrapping_add(i.wrapping_mul(2_654_435_761))
                .wrapping_add(0xd057);
            let dir = cone_dir(Vec3::Y, dust.spread_deg.to_radians(), s);
            let speed = dust.speed * (0.5 + 0.5 * rand01(s ^ 0x55));
            commands.spawn((
                StateScoped(AppState::InGame),
                ImpactParticle {
                    velocity: dir * speed + Vec3::Y * dust.rise,
                    gravity: 0.0,
                    drag: dust.drag,
                    age: 0.0,
                    lifetime: dust.lifetime.max(0.1) * (0.75 + 0.5 * rand01(s ^ 0x9f)),
                    fade_in: dust.fade_in.max(0.0),
                    roll: rand_roll(s ^ 0x21),
                    spin: 0.0,
                    scale0: dust.start_scale,
                    scale1: dust.end_scale,
                    peak_alpha: dust.opacity,
                    tint: [1.0, 1.0, 1.0],
                },
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(impact_material(assets.dust.clone()))),
                Transform::from_translation(at)
                    .with_scale(Vec3::splat(dust.start_scale.max(1.0e-4))),
                NoFrustumCulling,
            ));
        }
    }
}

/// Squirt a short blood burst out of a bot the instant a shot connects, along
/// the shot's travel direction from the hit point. World-space, so every client
/// and camera angle sees it. Mirrors `spawn_ground_impact`.
pub(crate) fn spawn_blood_impact(
    mut events: EventReader<BloodImpact>,
    assets: Res<ImpactAssets>,
    blood: Res<BloodSettings>,
    existing: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let mut budget = IMPACT_MAX.saturating_sub(existing.iter().count());

    for ev in events.read() {
        let jet = ev.dir.normalize_or(Vec3::NEG_Y);
        *seq = seq.wrapping_add(1);

        for i in 0..blood.count {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let s = seq
                .wrapping_mul(2_246_822_519)
                .wrapping_add(i.wrapping_mul(83_492_791))
                .wrapping_add(0xb100d);
            let dir = cone_dir(jet, blood.spread_deg.to_radians(), s);
            let speed = blood.speed * (0.35 + 0.65 * rand01(s ^ 0x9e37));
            // `blood-splatter-texture.png` is one wide field of droplets on
            // transparent; show a small random window into it per particle so
            // each droplet sprite is a handful of specks, not the whole field.
            // Bias toward the dense centre-left so a window is never empty.
            let win = Vec2::new(0.16, 0.26);
            let uv_off = Vec2::new(
                0.10 + rand01(s ^ 0x3).powf(1.5) * (0.62 - win.x),
                0.05 + rand01(s ^ 0x5) * (0.88 - win.y),
            );
            let mut material = impact_material(assets.blood.clone());
            material.uv_transform = Affine2::from_scale_angle_translation(win, 0.0, uv_off);
            let scale0 = (blood.scale * (0.6 + 0.8 * rand01(s ^ 0x2c))).max(1.0e-4);
            commands.spawn((
                StateScoped(AppState::InGame),
                ImpactParticle {
                    velocity: dir * speed,
                    gravity: blood.gravity,
                    drag: blood.drag,
                    age: 0.0,
                    lifetime: blood.lifetime.max(0.1) * (0.7 + 0.6 * rand01(s ^ 0x1234)),
                    fade_in: 0.0,
                    roll: rand_roll(s ^ 0x77),
                    spin: 0.0,
                    scale0,
                    scale1: scale0 * blood.growth.max(0.1),
                    peak_alpha: blood.opacity,
                    tint: blood.color,
                },
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(material)),
                Transform::from_translation(ev.point).with_scale(Vec3::splat(scale0)),
                NoFrustumCulling,
            ));
        }
    }
}

/// Integrate every live impact particle: gravity + drag, camera billboard with
/// its own roll/spin, scale ramp, opacity envelope, then despawn (freeing the
/// material) at end of life.
pub(crate) fn update_impact_particles(
    time: Res<Time>,
    player: Single<&Transform, (With<Player>, Without<ImpactParticle>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<ImpactParticle>)>,
    mut particles: Query<(
        Entity,
        &mut Transform,
        &mut ImpactParticle,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cam_pos = player.translation;
    let cam_rot = player.rotation * head.rotation;
    let cam_up = cam_rot * Vec3::Y;
    let cam_right_fallback = cam_rot * Vec3::X;

    for (entity, mut transform, mut p, material) in &mut particles {
        p.age += dt;
        if p.age >= p.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }

        let (gravity, drag) = (p.gravity, p.drag);
        p.velocity.y -= gravity * dt;
        p.velocity *= (1.0 - drag * dt).max(0.0);
        transform.translation += p.velocity * dt;

        let f = (p.age / p.lifetime.max(1.0e-4)).clamp(0.0, 1.0);
        let scale = p.scale0.lerp(p.scale1, f).max(1.0e-4);

        let to_cam = cam_pos - transform.translation;
        let roll = p.roll + p.spin * p.age;
        if to_cam.length_squared() > 1.0e-6 {
            let normal = to_cam.normalize();
            let mut right = cam_up.cross(normal);
            if right.length_squared() < 1.0e-6 {
                right = cam_right_fallback;
            }
            let right = right.normalize();
            let up = normal.cross(right);
            let facing = Quat::from_mat3(&Mat3::from_cols(right, up, normal));
            transform.rotation = facing * Quat::from_rotation_z(roll);
        }
        transform.scale = Vec3::splat(scale);

        let envelope = if p.age < p.fade_in {
            p.age / p.fade_in.max(1.0e-4)
        } else {
            let fade_out = (p.lifetime - p.fade_in).max(1.0e-4);
            1.0 - (p.age - p.fade_in) / fade_out
        };
        if let Some(m) = materials.get_mut(&material.0) {
            let [tr, tg, tb] = p.tint;
            m.base_color = Color::srgba(tr, tg, tb, p.peak_alpha * envelope.clamp(0.0, 1.0));
        }
    }
}
