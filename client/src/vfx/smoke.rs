//! Barrel smoke: a burst of drifting, camera-billboarded sprites released for
//! a short window after each shot, fading independently as they age.

use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;

use crate::player::{Player, PlayerHead};
use crate::util::{rand01, rand_roll};

/// Hard cap on live smoke particles, and the most that can spawn in one frame.
pub(crate) const SMOKE_MAX: usize = 500;
pub(crate) const SMOKE_MAX_PER_FRAME: f32 = 8.0;

/// A single drifting, fading smoke sprite. World-space: once spawned it lives in
/// the world, so the player can walk through it.
#[derive(Component)]
pub(crate) struct Smoke {
    velocity: Vec3,
    age: f32,
    /// Seconds to ramp 0 -> `peak_alpha` before the fade-out begins.
    fade_in: f32,
    /// Total time alive (`fade_in` + fade-out); despawns at this age.
    lifetime: f32,
    roll: f32,
    /// Opacity at the top of the envelope (later spawns in a burst peak dimmer).
    peak_alpha: f32,
}

/// Time (seconds) since the last shot started a smoke burst; `None` when not
/// emitting.
#[derive(Resource, Default)]
pub(crate) struct SmokeEmission(pub(crate) Option<f32>);

/// Shared mesh + texture for smoke particles (each particle still gets its own
/// material so it can fade independently).
#[derive(Resource)]
pub(crate) struct SmokeAssets {
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) texture: Handle<Image>,
}

/// Panel-adjustable smoke emitter settings. `spawn_offset` is in camera space;
/// particles are then released into world space at that point.
#[derive(Resource)]
pub(crate) struct SmokeSettings {
    pub(crate) spawn_offset: Vec3,
    pub(crate) scale: f32,
    pub(crate) rise_rate: f32,
    pub(crate) spread: f32,
    /// Seconds each particle takes to ramp up from invisible to its peak opacity.
    pub(crate) fade_in: f32,
    /// Seconds each particle then takes to fade back to nothing.
    pub(crate) fade_time: f32,
    pub(crate) spawn_rate: f32,
    /// How long the burst keeps emitting after a shot.
    pub(crate) duration: f32,
    /// Peak opacity of a particle spawned the instant a shot fires. Spawns later
    /// in the burst peak proportionally dimmer; nothing ever exceeds this.
    pub(crate) max_opacity: f32,
}

impl Default for SmokeSettings {
    fn default() -> Self {
        Self {
            spawn_offset: Vec3::new(0.15, -0.08, -1.05),
            scale: 0.2,
            rise_rate: 0.15,
            spread: 0.1,
            fade_in: 0.1,
            fade_time: 0.4,
            spawn_rate: 40.0,
            duration: 1.5,
            max_opacity: 0.03,
        }
    }
}

/// Release smoke from the barrel for `SmokeSettings::duration` seconds after a
/// shot. Particles spawned later in the burst start dimmer, ramping from
/// `max_opacity` (right after the shot) down to 0 (end of the burst).
pub(crate) fn emit_smoke(
    time: Res<Time>,
    settings: Res<SmokeSettings>,
    assets: Res<SmokeAssets>,
    player: Single<&Transform, With<Player>>,
    head: Single<&Transform, With<PlayerHead>>,
    existing: Query<(), With<Smoke>>,
    mut emission: ResMut<SmokeEmission>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut acc: Local<f32>,
    mut seq: Local<u32>,
) {
    // Advance / end the current burst.
    let Some(mut since_shot) = emission.0 else {
        *acc = 0.0;
        return;
    };
    since_shot += time.delta_secs();
    if since_shot >= settings.duration || settings.spawn_rate <= 0.0 {
        emission.0 = None;
        *acc = 0.0;
        return;
    }
    emission.0 = Some(since_shot);

    // Peak opacity for a particle spawned right now (dimmer later in the burst).
    let burst_progress = (since_shot / settings.duration.max(1.0e-4)).clamp(0.0, 1.0);
    let peak_alpha = settings.max_opacity * (1.0 - burst_progress);
    if peak_alpha < 0.02 {
        return;
    }
    let fade_in = settings.fade_in.max(0.0);
    let lifetime = (fade_in + settings.fade_time.max(0.05)).max(0.05);

    let interval = 1.0 / settings.spawn_rate;
    *acc = (*acc + time.delta_secs()).min(interval * SMOKE_MAX_PER_FRAME);

    // Camera pose from the live transforms (Player = eye position, its child
    // Head carries pitch) so a fast turn while firing doesn't lag the origin.
    let cam_rotation = player.rotation * head.rotation;
    let origin = player.translation + cam_rotation * settings.spawn_offset;
    let mut budget = SMOKE_MAX.saturating_sub(existing.iter().count());

    while *acc >= interval {
        *acc -= interval;
        if budget == 0 {
            continue;
        }
        budget -= 1;
        *seq = seq.wrapping_add(1);
        let s = *seq;

        let angle = rand_roll(s.wrapping_mul(3));
        let speed = rand01(s.wrapping_mul(5)) * settings.spread;
        let velocity = Vec3::new(angle.cos() * speed, settings.rise_rate, angle.sin() * speed);
        let roll = rand_roll(s.wrapping_mul(7));

        // Pose and fade it exactly as `update_smoke` would on its first tick —
        // otherwise `commands.spawn` doesn't land until the next sync point, so
        // this frame's render sees the entity before `update_smoke` ever runs
        // on it: the default identity rotation (not facing the camera) and the
        // material's default opaque white, i.e. one full-opacity, wrong-facing
        // frame before the fade-in/billboard even starts.
        let cam_up = cam_rotation * Vec3::Y;
        let cam_right_fallback = cam_rotation * Vec3::X;
        let rotation = smoke_billboard_rotation(origin, player.translation, cam_up, cam_right_fallback, roll);
        let alpha = peak_alpha * smoke_envelope(0.0, fade_in, lifetime);

        let material = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, alpha),
            base_color_texture: Some(assets.texture.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        });

        commands.spawn((
            Smoke {
                velocity,
                age: 0.0,
                fade_in,
                lifetime,
                roll,
                peak_alpha,
            },
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(origin)
                .with_rotation(rotation)
                .with_scale(Vec3::splat(settings.scale.max(1.0e-4))),
            NoFrustumCulling,
        ));
    }
}

/// Drift each smoke particle, keep it facing the camera, fade it, and despawn
/// it (freeing its material) when its lifetime is up.
pub(crate) fn update_smoke(
    time: Res<Time>,
    settings: Res<SmokeSettings>,
    player: Single<&Transform, (With<Player>, Without<Smoke>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<Smoke>)>,
    mut particles: Query<
        (
            Entity,
            &mut Transform,
            &mut Smoke,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<Smoke>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    // Live camera pose (Player = eye, its child carries pitch) — not the
    // previous frame's propagated `GlobalTransform`.
    let cam_pos = player.translation;
    let cam_up = (player.rotation * head.rotation) * Vec3::Y;
    let cam_right_fallback = (player.rotation * head.rotation) * Vec3::X;
    let scale = Vec3::splat(settings.scale.max(1.0e-4));

    for (entity, mut transform, mut particle, material) in &mut particles {
        particle.age += dt;
        if particle.age >= particle.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }

        transform.translation += particle.velocity * dt;

        // Spherical billboard: the quad's front (+Z) points straight at the
        // camera position, its up follows the camera up, so every puff always
        // shows the same head-on face no matter which way the player turns.
        transform.rotation = smoke_billboard_rotation(
            transform.translation,
            cam_pos,
            cam_up,
            cam_right_fallback,
            particle.roll,
        );
        transform.scale = scale;

        let alpha = particle.peak_alpha * smoke_envelope(particle.age, particle.fade_in, particle.lifetime);
        if let Some(material) = materials.get_mut(&material.0) {
            material.base_color = Color::srgba(1.0, 1.0, 1.0, alpha);
        }
    }
}

/// Spherical-billboard rotation for a smoke puff at `pos`: local +Z points
/// straight at `cam_pos`, up follows `cam_up` (falling back to
/// `cam_right_fallback` on the degenerate case where the puff sits dead-on
/// with the camera's up axis), then spun by `roll` around that facing axis.
/// Shared by `emit_smoke` (the puff's pose the instant it spawns) and
/// `update_smoke` (every frame after) so a fresh puff is never caught one
/// frame short of `update_smoke` reaching it — `commands.spawn` doesn't land
/// until the next sync point, so without this a brand-new puff would render
/// at the identity rotation for a frame.
pub(crate) fn smoke_billboard_rotation(
    pos: Vec3,
    cam_pos: Vec3,
    cam_up: Vec3,
    cam_right_fallback: Vec3,
    roll: f32,
) -> Quat {
    let to_cam = cam_pos - pos;
    if to_cam.length_squared() <= 1.0e-6 {
        return Quat::IDENTITY;
    }
    let normal = to_cam.normalize();
    let mut right = cam_up.cross(normal);
    if right.length_squared() < 1.0e-6 {
        right = cam_right_fallback;
    }
    let right = right.normalize();
    let up = normal.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, up, normal)) * Quat::from_rotation_z(roll)
}

/// A smoke puff's opacity envelope at `age`: ramps 0 -> 1 over `fade_in`, then
/// 1 -> 0 over the rest of `lifetime`. Shared by `emit_smoke` (so a puff's
/// material starts at the same alpha this gives for `age: 0.0`, rather than
/// the material's default opaque white for the one frame before
/// `update_smoke` first reaches it) and `update_smoke`.
pub(crate) fn smoke_envelope(age: f32, fade_in: f32, lifetime: f32) -> f32 {
    let envelope = if age < fade_in {
        age / fade_in.max(1.0e-4)
    } else {
        let fade_out = (lifetime - fade_in).max(1.0e-4);
        1.0 - (age - fade_in) / fade_out
    };
    envelope.clamp(0.0, 1.0)
}
