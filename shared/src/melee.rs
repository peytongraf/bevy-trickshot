//! Knife (melee) hit resolution — the close-range counterpart of
//! [`crate::ballistics`]. A stab lands where everyone else sees it land: the
//! stabber's thrust (the client's `held_weapon::stab_curve`, on their
//! soldier) carries the knife's tip [`KNIFE_REACH_M`] straight out ahead of
//! them, at chest height, and whatever body that tip touches on the way is
//! stabbed — at the moment the thrust gets there ([`KNIFE_HIT_SECS`] after
//! the stab starts; the client sends it then). The server runs
//! [`resolve_melee`] for every stab request.

use bevy::math::Vec3;

use crate::ballistics::Target;

/// How far ahead of the eye (m, along the ground) the knife's tip gets at
/// the thrust's full stretch — measured off the soldier model, arm straight
/// out (0.70–0.84 m standing still or walking).
pub const KNIFE_REACH_M: f32 = 0.8;

/// How far below the eye (m) the tip goes — chest height on the model.
pub const KNIFE_TIP_DROP_M: f32 = 0.35;

/// How much smaller (m) a body really is than its hitbox (sized for bullets,
/// [`Target::body`]): the tip has to come within the capsule's radius less
/// this of its middle to touch them (a player's or zombie's 0.35 m → 0.2 m,
/// about their chest's half width).
pub const KNIFE_BODY_INSET_M: f32 = 0.15;

/// When (s into the stab) the thrust reaches full stretch — the stab lands
/// then. (The client's `held_weapon::stab_curve` lunges to full reach 0.32 of
/// the way through its 0.67 s.)
pub const KNIFE_HIT_SECS: f32 = 0.21;

/// A target must be within this cone (cosine of the half-angle, ≈ 50°) of
/// the aim direction, so a stab never lands on someone standing beside or
/// behind you just because they're close.
const KNIFE_MIN_FACING_COS: f32 = 0.64;

/// Damage a knife stab deals in a mode with health — far above any player's
/// health, since a knife is always a one-hit kill.
pub const KNIFE_DAMAGE: f32 = 1000.0;

/// The stab's victim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeleeHit {
    /// The [`Target::id`] that was stabbed.
    pub target: u64,
    /// World-space point on the target's body the stab landed at.
    pub point: Vec3,
    /// Distance from the eye to `point`.
    pub distance: f32,
}

/// Resolve one knife stab by someone whose eye is at `origin`, facing `dir`
/// (need not be normalised; only its heading counts — the thrust goes
/// straight out ahead, whichever way they're looking up or down): the
/// nearest body the knife's tip touches on its way out, or `None` for a
/// whiff. Only targets' bodies count — there's no headshot bonus.
pub fn resolve_melee(origin: Vec3, dir: Vec3, targets: &[Target]) -> Option<MeleeHit> {
    let dir = Vec3::new(dir.x, 0.0, dir.z).try_normalize().unwrap_or_else(|| dir.normalize_or_zero());
    if dir == Vec3::ZERO {
        return None;
    }
    let chest = origin - Vec3::Y * KNIFE_TIP_DROP_M;
    let tip = chest + dir * KNIFE_REACH_M;

    let mut best: Option<MeleeHit> = None;
    for t in targets {
        let (on_ray, on_body) = closest_points_between_segments(chest, tip, t.body.a, t.body.b);
        if on_ray.distance(on_body) > (t.body.radius - KNIFE_BODY_INSET_M).max(0.05) {
            continue;
        }
        let to_body = on_body - origin;
        let distance = to_body.length();
        // Inside the target (or nearly) always counts; otherwise it has to be
        // in front of where the player is looking.
        if distance > 0.1 && to_body.dot(dir) / distance < KNIFE_MIN_FACING_COS {
            continue;
        }
        if best.is_none_or(|b| distance < b.distance) {
            best = Some(MeleeHit {
                target: t.id,
                point: on_body,
                distance,
            });
        }
    }
    best
}

/// Closest pair of points between segments `p1..q1` and `p2..q2` (Ericson,
/// *Real-Time Collision Detection* §5.1.9), returned as `(on_first,
/// on_second)`.
fn closest_points_between_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);
    const EPS: f32 = 1e-9;

    let (s, t);
    if a <= EPS && e <= EPS {
        s = 0.0;
        t = 0.0;
    } else if a <= EPS {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s_ = if denom > EPS {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t_ = (b * s_ + f) / e;
            if t_ < 0.0 {
                t_ = 0.0;
                s_ = (-c / a).clamp(0.0, 1.0);
            } else if t_ > 1.0 {
                t_ = 1.0;
                s_ = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s_;
            t = t_;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hitbox::Capsule;

    fn target(id: u64, feet: Vec3) -> Target {
        Target {
            id,
            body: Capsule::standing(feet, 1.8, 0.35),
            head: Capsule::head(feet, 1.8, 0.12),
        }
    }

    const EYE: Vec3 = Vec3::new(0.0, 1.7, 0.0);

    #[test]
    fn stabs_a_target_directly_ahead_in_reach() {
        let t = target(1, Vec3::new(0.0, 0.0, -0.9));
        let hit = resolve_melee(EYE, Vec3::NEG_Z, &[t]).expect("should stab");
        assert_eq!(hit.target, 1);
    }

    #[test]
    fn whiffs_just_past_where_the_tip_gets() {
        // Its middle 1.05 m off: the tip (0.8 m) stops short of its chest
        // (0.2 m in front of that).
        let t = target(1, Vec3::new(0.0, 0.0, -1.05));
        assert!(resolve_melee(EYE, Vec3::NEG_Z, &[t]).is_none());
    }

    #[test]
    fn stabs_a_little_off_to_the_side() {
        // 0.7 m ahead, 0.15 m to the side — the tip grazes their chest.
        let t = target(1, Vec3::new(0.15, 0.0, -0.7));
        assert!(resolve_melee(EYE, Vec3::NEG_Z, &[t]).is_some());
        // 0.3 m to the side: past them.
        let t = target(1, Vec3::new(0.3, 0.0, -0.7));
        assert!(resolve_melee(EYE, Vec3::NEG_Z, &[t]).is_none());
    }

    #[test]
    fn looking_up_or_down_doesnt_shorten_it() {
        let t = target(1, Vec3::new(0.0, 0.0, -0.9));
        assert!(resolve_melee(EYE, Vec3::new(0.0, -1.0, -1.0), std::slice::from_ref(&t)).is_some());
        assert!(resolve_melee(EYE, Vec3::new(0.0, 1.0, -1.0), std::slice::from_ref(&t)).is_some());
    }

    #[test]
    fn whiffs_when_too_far() {
        let t = target(1, Vec3::new(0.0, 0.0, -5.0));
        assert!(resolve_melee(EYE, Vec3::NEG_Z, &[t]).is_none());
    }

    #[test]
    fn whiffs_when_aimed_away() {
        let t = target(1, Vec3::new(0.0, 0.0, -0.9));
        assert!(resolve_melee(EYE, Vec3::Z, std::slice::from_ref(&t)).is_none());
        assert!(resolve_melee(EYE, Vec3::X, std::slice::from_ref(&t)).is_none());
    }

    #[test]
    fn picks_the_nearest_of_several() {
        let far = target(1, Vec3::new(0.0, 0.0, -0.9));
        let near = target(2, Vec3::new(0.0, 0.0, -0.6));
        let hit = resolve_melee(EYE, Vec3::NEG_Z, &[far, near]).unwrap();
        assert_eq!(hit.target, 2);
    }

    #[test]
    fn zero_direction_is_a_whiff() {
        let t = target(1, Vec3::new(0.0, 0.0, -1.0));
        assert!(resolve_melee(EYE, Vec3::ZERO, &[t]).is_none());
    }
}
