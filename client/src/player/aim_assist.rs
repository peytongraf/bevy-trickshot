//! Shroom aim assist: while the shroom effect is on and the player is aimed
//! down sight, a bot within a small cone of the crosshair (and in plain
//! sight) gently pulls the aim onto its upper chest. Strength follows the
//! effect's own fade (`ShroomLevel`); tuned from the debug panel's "Shroom
//! effect" section (`ShroomSettings::assist_*`).
//!
//! Rotates the body (yaw) and head (pitch) the same way `look_around` does,
//! but deliberately doesn't report it in `LookDelta`: that feeds trickshot
//! spin tracking, and an assisted turn shouldn't count toward a 360.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::Interpolated;

use crate::{Ads, ShroomLevel, ShroomSettings};

use super::camera::{PlayerHead, PITCH_LIMIT};
use super::movement::Player;

/// Where on a bot the assist aims: its upper chest, this far below the eye...
const CHEST_BELOW_EYE: f32 = 0.35;
/// ...or this far above a `Freestyle` target's feet.
const TARGET_CHEST_HEIGHT: f32 = 1.35;

/// Yaw / pitch (radians) that look along unit `dir` — yaw 0 faces -Z,
/// positive pitch looks up (the rig's own convention).
fn look_angles(dir: Vec3) -> (f32, f32) {
    (f32::atan2(-dir.x, -dir.z), dir.y.clamp(-1.0, 1.0).asin())
}

/// Wrap an angle difference into `(-π, π]`.
fn wrap(a: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    let d = a.rem_euclid(tau);
    if d > core::f32::consts::PI {
        d - tau
    } else {
        d
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn shroom_aim_assist(
    time: Res<Time>,
    settings: Res<ShroomSettings>,
    level: Res<ShroomLevel>,
    ads: Res<Ads>,
    rapier: ReadRapierContext,
    bot_players: Query<(&shared::PlayerPose, &shared::PlayerId), With<Interpolated>>,
    targets: Query<&shared::Bot, With<Interpolated>>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    if !settings.assist_enabled || level.0 <= 1e-3 || ads.t < settings.assist_min_ads {
        return;
    }
    let dt = time.delta_secs();
    let eye = player.translation;
    let yaw = player.rotation.to_euler(EulerRot::YXZ).0;
    let pitch = head.rotation.to_euler(EulerRot::YXZ).1;
    let forward = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z;

    // Every living bot's aim point: bot players (FFA bots, zombies) and
    // `Freestyle` targets, where this client sees them.
    let points = bot_players
        .iter()
        .filter(|(pose, id)| pose.alive && shared::bot_players::is_bot_peer(id.0))
        .map(|(pose, _)| pose.translation - Vec3::Y * CHEST_BELOW_EYE)
        .chain(
            targets
                .iter()
                .filter(|b| b.alive)
                .map(|b| b.pos + Vec3::Y * TARGET_CHEST_HEIGHT),
        );

    let cone = settings.assist_cone_deg.to_radians();
    let rapier = rapier.single().ok();
    // The one nearest the crosshair (by angle), inside the cone, in range,
    // and not behind a wall.
    let best = points
        .filter_map(|p| {
            let to = p - eye;
            let dist = to.length();
            if dist < 1e-3 || dist > settings.assist_range {
                return None;
            }
            let dir = to / dist;
            let off = forward.angle_between(dir);
            (off <= cone).then_some((dir, dist, off))
        })
        .filter(|(dir, dist, _)| {
            rapier.as_ref().is_none_or(|r| {
                r.cast_ray(eye, *dir, (dist - 0.3).max(0.0), true, QueryFilter::default())
                    .is_none()
            })
        })
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((dir, _, off)) = best else {
        return;
    };

    // Ease toward it — harder the closer it already is to the crosshair (a
    // gentle tug at the cone's edge, sticky near the middle) — capped at a
    // top turn speed, all scaled by how far the shroom has faded in.
    let (want_yaw, want_pitch) = look_angles(dir);
    let (dy, dp) = (wrap(want_yaw - yaw), want_pitch - pitch);
    let closeness = 1.0 - (off / cone.max(1e-4)).clamp(0.0, 1.0);
    let pull = settings.assist_strength * (0.35 + 0.65 * closeness) * level.0;
    let frac = 1.0 - (-pull * dt).exp();
    let max_step = settings.assist_max_speed_deg.to_radians() * level.0 * dt;
    let step = Vec2::new(dy, dp) * frac;
    let step = if step.length() > max_step {
        step.normalize_or_zero() * max_step
    } else {
        step
    };

    player.rotate_y(step.x);
    head.rotation = Quat::from_rotation_x((pitch + step.y).clamp(-PITCH_LIMIT, PITCH_LIMIT));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_angles_match_the_rig() {
        let (y, p) = look_angles(Vec3::NEG_Z);
        assert!(y.abs() < 1e-5 && p.abs() < 1e-5);
        let dir = Vec3::new(-1.0, 0.5, -1.0).normalize();
        let (y, p) = look_angles(dir);
        let back = Quat::from_euler(EulerRot::YXZ, y, p, 0.0) * Vec3::NEG_Z;
        assert!((back - dir).length() < 1e-4, "{back:?} vs {dir:?}");
    }
}
