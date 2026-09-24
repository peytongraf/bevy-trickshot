//! Fire tracers: a bright instant flash along the shot's path, settling into
//! a fading smoke trail — a stylised read of a hitscan shot with no real
//! flight time to animate.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::util::{color_from_parts, srgb_parts};
use crate::AppState;

/// A shot's visual tracer path, `start -> end` in world space. Fired for the
/// local shooter's own shot the instant it's taken (`resolve_local_shot`, both
/// game modes) and for every other player's shot off the server's
/// authoritative `ShotResolved` broadcast (`net::receive_shots`). Consumed by
/// `spawn_tracers`.
#[derive(Event)]
pub(crate) struct FireTracer {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
}

/// A streak along a shot's path: a brief bright "just fired" flash, then a
/// lingering smoke trail along the same line that widens and fades out. The
/// sniper is instant-hitscan, so there's no real flight time to animate —
/// both phases are a stylised read of "a shot just went through here."
#[derive(Component)]
pub(crate) struct Tracer {
    age: f32,
}

/// Panel-adjustable tracer look (`ads_tuning_ui`'s "Tracer" section).
#[derive(Resource)]
pub(crate) struct TracerSettings {
    /// Seconds the bright flash phase lasts.
    pub(crate) flash_secs: f32,
    /// Seconds the smoke trail then fades over.
    pub(crate) smoke_secs: f32,
    /// Flash color — bright and additive/emissive so `Bloom` (already on the
    /// world camera) reads it as a hot, glowing streak.
    pub(crate) flash_color: [f32; 3],
    /// How many stops brighter than `flash_color` the emissive glow is.
    pub(crate) flash_emissive_boost: f32,
    /// Line radius (m) during the flash phase.
    pub(crate) flash_radius: f32,
    /// Smoke-trail color — desaturated, alpha-blended, not emissive.
    pub(crate) smoke_color: [f32; 3],
    /// Opacity the smoke trail starts at, right as the flash ends.
    pub(crate) smoke_start_alpha: f32,
    /// Line radius (m) the smoke trail has widened to by the end of its fade.
    pub(crate) smoke_radius: f32,
    /// Thrown-knife trail ([`KnifeTrail`]): color, starting opacity, line
    /// radius (m), and seconds each piece takes to fade out.
    pub(crate) knife_trail_color: [f32; 3],
    pub(crate) knife_trail_alpha: f32,
    pub(crate) knife_trail_radius: f32,
    pub(crate) knife_trail_secs: f32,
}

impl Default for TracerSettings {
    fn default() -> Self {
        Self {
            flash_secs: 0.06,
            smoke_secs: 2.0,
            flash_color: srgb_parts(Color::srgb(1.0, 0.8, 0.35)),
            flash_emissive_boost: 6.0,
            flash_radius: 0.018,
            smoke_color: srgb_parts(Color::srgb(0.72, 0.7, 0.66)),
            smoke_start_alpha: 0.03,
            smoke_radius: 0.05,
            knife_trail_color: srgb_parts(Color::srgb_u8(245, 252, 255)),
            knife_trail_alpha: 0.5,
            knife_trail_radius: 0.012,
            knife_trail_secs: 1.2,
        }
    }
}

/// Shared unit cylinder (radius 0.5, height 1) that `spawn_tracers` scales to
/// each shot's length/width, so tracers don't allocate a fresh mesh per shot.
#[derive(Resource)]
pub(crate) struct TracerAssets {
    pub(crate) mesh: Handle<Mesh>,
}

/// Spawn a streak for each [`FireTracer`] this frame: a thin cylinder spanning
/// `start -> end`, starting in the bright additive "flash" look — `update_tracers`
/// carries it into the smoke phase.
pub(crate) fn spawn_tracers(
    mut events: EventReader<FireTracer>,
    assets: Res<TracerAssets>,
    settings: Res<TracerSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for ev in events.read() {
        let delta = ev.end - ev.start;
        let len = delta.length();
        if len < 0.05 {
            continue; // too short to read as a streak (e.g. an instant self-hit)
        }
        let dir = delta / len;
        let mid = ev.start + delta * 0.5;
        let flash = color_from_parts(settings.flash_color);
        let material = materials.add(StandardMaterial {
            base_color: flash,
            emissive: LinearRgba::from(flash) * settings.flash_emissive_boost,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        commands.spawn((
            Tracer { age: 0.0 },
            StateScoped(AppState::InGame),
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(material),
            Transform {
                translation: mid,
                rotation: Quat::from_rotation_arc(Vec3::Y, dir),
                scale: Vec3::new(
                    settings.flash_radius * 2.0,
                    len,
                    settings.flash_radius * 2.0,
                ),
            },
            NotShadowCaster,
        ));
    }
}

/// Drive each tracer through its two phases — a steady bright flash, then a
/// smoke trail that widens and fades to nothing — then despawn it and free
/// its (per-instance) material.
pub(crate) fn update_tracers(
    time: Res<Time>,
    settings: Res<TracerSettings>,
    mut tracers: Query<(
        Entity,
        &mut Tracer,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let flash_secs = settings.flash_secs.max(0.0);
    let smoke_secs = settings.smoke_secs.max(1.0e-4);
    let total = flash_secs + smoke_secs;

    for (entity, mut tracer, mut transform, material) in &mut tracers {
        tracer.age += dt;
        if tracer.age >= total {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        let Some(m) = materials.get_mut(&material.0) else {
            continue;
        };
        if tracer.age < flash_secs {
            let flash = color_from_parts(settings.flash_color);
            m.alpha_mode = AlphaMode::Add;
            m.base_color = flash;
            m.emissive = LinearRgba::from(flash) * settings.flash_emissive_boost;
            transform.scale.x = settings.flash_radius * 2.0;
            transform.scale.z = settings.flash_radius * 2.0;
        } else {
            let t = ((tracer.age - flash_secs) / smoke_secs).clamp(0.0, 1.0);
            let smoke = color_from_parts(settings.smoke_color);
            m.alpha_mode = AlphaMode::Blend;
            m.emissive = LinearRgba::BLACK;
            m.base_color = smoke.with_alpha(settings.smoke_start_alpha * (1.0 - t));
            let radius = settings.flash_radius.lerp(settings.smoke_radius, t);
            transform.scale.x = radius * 2.0;
            transform.scale.z = radius * 2.0;
        }
    }
}

/// Length (m) of each piece of a thrown knife's trail — short enough that the
/// pieces follow its arc (and bounces) as a smooth line.
const KNIFE_TRAIL_SEGMENT: f32 = 0.25;

/// A thrown knife's trail: a faint line of short pieces laid down along the
/// path it has actually flown (see [`lay_knife_trail`]), each fading out on
/// its own, so it thins away from the tail first.
#[derive(Component)]
pub(crate) struct KnifeTrail {
    age: f32,
}

/// Where a knife's trail last left off. On the knife's avatar.
#[derive(Component, Default)]
pub(crate) struct KnifeTrailHead(pub(crate) Option<Vec3>);

/// Lay the next piece of trail from where `head` left off to `pos`, once the
/// knife has moved [`KNIFE_TRAIL_SEGMENT`] — or reset the head while it's at
/// rest / hidden (`flying == false`), so a trail never bridges a gap.
pub(crate) fn lay_knife_trail(
    head: &mut KnifeTrailHead,
    pos: Vec3,
    flying: bool,
    assets: &TracerAssets,
    settings: &TracerSettings,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
) {
    if !flying {
        head.0 = None;
        return;
    }
    let Some(from) = head.0 else {
        head.0 = Some(pos);
        return;
    };
    let delta = pos - from;
    let len = delta.length();
    if len < KNIFE_TRAIL_SEGMENT {
        return;
    }
    head.0 = Some(pos);
    let color = color_from_parts(settings.knife_trail_color);
    let material = materials.add(StandardMaterial {
        base_color: color.with_alpha(settings.knife_trail_alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.spawn((
        KnifeTrail { age: 0.0 },
        StateScoped(AppState::InGame),
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(material),
        Transform {
            translation: from + delta * 0.5,
            rotation: Quat::from_rotation_arc(Vec3::Y, delta / len),
            scale: Vec3::new(
                settings.knife_trail_radius * 2.0,
                len,
                settings.knife_trail_radius * 2.0,
            ),
        },
        NotShadowCaster,
    ));
}

/// Fade each knife-trail piece out over `knife_trail_secs`, then despawn it
/// and free its (per-piece) material.
pub(crate) fn update_knife_trails(
    time: Res<Time>,
    settings: Res<TracerSettings>,
    mut trails: Query<(Entity, &mut KnifeTrail, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let secs = settings.knife_trail_secs.max(1.0e-4);
    for (entity, mut trail, material) in &mut trails {
        trail.age += time.delta_secs();
        if trail.age >= secs {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        if let Some(m) = materials.get_mut(&material.0) {
            let fade = 1.0 - trail.age / secs;
            m.base_color = color_from_parts(settings.knife_trail_color)
                .with_alpha(settings.knife_trail_alpha * fade);
        }
    }
}
