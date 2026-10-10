//! Shroom aim assist: while the shroom effect is on (or with the classic
//! Deadshot Daiquiri perk, at full strength) and the player is aimed down
//! sight, a bot within a small cone of the crosshair (and in plain
//! sight) gently pulls the aim onto its upper chest. Strength follows the
//! effect's own fade (`ShroomLevel`); tuned from the debug panel's "Shroom
//! effect" section (`ShroomSettings::assist_*`).
//!
//! Deadshot pulls a bit harder ([`DEADSHOT_PULL`]) and onto the head, like
//! Call of Duty's: the model's actual head bone ([`HeadBone`]), as drawn —
//! the same bone a shot has to strike for a headshot (`hit_detection`).
//!
//! Rotates the body (yaw) and head (pitch) the same way `look_around` does,
//! but deliberately doesn't report it in `LookDelta`: that feeds trickshot
//! spin tracking, and an assisted turn shouldn't count toward a 360.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::Interpolated;

use crate::boss::BossVisual;
use crate::dogs::DogVisual;
use crate::net::{BotPose, RemoteAvatar};
use crate::{Ads, ShroomLevel, ShroomSettings};

use super::camera::{PlayerHead, PITCH_LIMIT};
use super::movement::Player;

/// Where on a bot the assist aims: its upper chest, this far below the eye...
const CHEST_BELOW_EYE: f32 = 0.35;
/// ...or this far above a `Freestyle` target's feet.
const TARGET_CHEST_HEIGHT: f32 = 1.35;

/// How much harder (pull and top turn speed) Deadshot tugs than a full
/// shroom.
const DEADSHOT_PULL: f32 = 1.5;
/// Deadshot's head aim point: this far (m) above the head bone, which sits
/// at the base of the skull — up to the middle of the head. A boss's head is
/// bigger; a hellhound's bone already sits mid-head.
const HEAD_LIFT: f32 = 0.07;
const BOSS_HEAD_LIFT: f32 = 0.15;

/// On a [`RemoteAvatar`]: its model's head bone, once its scene's in.
#[derive(Component)]
pub(crate) struct HeadBone(Entity);

/// Whether a model's bone called `name` is its head (every rig's: `soldier`,
/// `zombie`, `dog`, `boss.glb`) — not the neck, jaw, eyes, the boss's
/// `HeadNub` or the zombie's `Z_Head` mesh.
fn is_head_bone(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("head") && !["neck", "jaw", "eye", "nub", "top", "z_head"].iter().any(|w| n.contains(w))
}

/// Find each new avatar's [`HeadBone`] (retried every frame until its scene
/// has loaded). Gone with the avatar, so nothing carries between games.
pub(crate) fn tag_head_bones(
    mut commands: Commands,
    avatars: Query<Entity, (With<RemoteAvatar>, Without<HeadBone>)>,
    children: Query<&Children>,
    names: Query<&Name>,
) {
    for avatar in &avatars {
        if let Some(bone) = children
            .iter_descendants(avatar)
            .find(|&e| names.get(e).is_ok_and(|n| is_head_bone(n.as_str())))
        {
            commands.entity(avatar).insert(HeadBone(bone));
        }
    }
}

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
    classic: Res<crate::zombies_hud::ClassicPerks>,
    ads: Res<Ads>,
    rapier: ReadRapierContext,
    bot_players: Query<(&shared::PlayerPose, &shared::PlayerId), With<Interpolated>>,
    targets: Query<&shared::Bot, With<Interpolated>>,
    heads: Query<(&RemoteAvatar, &HeadBone, Has<BossVisual>, Has<DogVisual>)>,
    sources: Query<(&shared::PlayerPose, Option<&shared::PlayerId>, Option<&BotPose>)>,
    all_bots: Query<&shared::Bot>,
    bones: Query<&GlobalTransform>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    // Deadshot Daiquiri (a classic perk) gives a bit more than the full
    // pull, no shroom needed.
    let deadshot = classic.has(shared::perks::Perk::DeadshotDaiquiri);
    let level = if deadshot { 1.0 } else { level.0 };
    if !settings.assist_enabled || level <= 1e-3 || ads.t < settings.assist_min_ads {
        return;
    }
    let dt = time.delta_secs();
    let eye = player.translation;
    let yaw = player.rotation.to_euler(EulerRot::YXZ).0;
    let pitch = head.rotation.to_euler(EulerRot::YXZ).1;
    let forward = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z;

    // Every living bot's aim point: bot players (FFA bots, zombies) and
    // `Freestyle` targets, where this client sees them — their heads with
    // Deadshot, upper chests otherwise.
    let points: Vec<Vec3> = if deadshot {
        heads
            .iter()
            .filter_map(|(avatar, head, boss, dog)| {
                let (pose, id, stand_in) = sources.get(avatar.src).ok()?;
                let bot = match (id, stand_in) {
                    (_, Some(stand_in)) => all_bots.get(stand_in.bot).is_ok_and(|b| b.alive),
                    (Some(id), None) => shared::bot_players::is_bot_peer(id.0),
                    (None, None) => false,
                };
                if !pose.alive || !bot {
                    return None;
                }
                let lift = if boss {
                    BOSS_HEAD_LIFT
                } else if dog {
                    0.0
                } else {
                    HEAD_LIFT
                };
                Some(bones.get(head.0).ok()?.translation() + Vec3::Y * lift)
            })
            .collect()
    } else {
        bot_players
            .iter()
            .filter(|(pose, id)| pose.alive && shared::bot_players::is_bot_peer(id.0))
            .map(|(pose, _)| pose.translation - Vec3::Y * CHEST_BELOW_EYE)
            .chain(
                targets
                    .iter()
                    .filter(|b| b.alive)
                    .map(|b| b.pos + Vec3::Y * TARGET_CHEST_HEIGHT),
            )
            .collect()
    };

    let cone = settings.assist_cone_deg.to_radians();
    let rapier = rapier.single().ok();
    // The one nearest the crosshair (by angle), inside the cone, in range,
    // and not behind a wall.
    let best = points
        .into_iter()
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
    let boost = if deadshot { DEADSHOT_PULL } else { 1.0 };
    let pull = settings.assist_strength * (0.35 + 0.65 * closeness) * level * boost;
    let frac = 1.0 - (-pull * dt).exp();
    let max_step = settings.assist_max_speed_deg.to_radians() * level * boost * dt;
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
    fn finds_every_rigs_head_bone() {
        for head in ["Base HumanHead_09", "Bind_Head_014_015", "head_23", "Bip01_Head_014"] {
            assert!(is_head_bone(head), "{head}");
        }
        for not in ["Base HumanNeck_08", "Base HumanHeadJaw_010", "Base HumanHeadREye_013", "Z_Head", "head neck.001_24", "head jaw_22", "Bip01_HeadNub"] {
            assert!(!is_head_bone(not), "{not}");
        }
    }

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
