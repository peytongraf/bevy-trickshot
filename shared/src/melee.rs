//! Knife (melee) hit resolution — the close-range counterpart of
//! [`crate::ballistics`]. Like a Call-of-Duty knife, a stab isn't a precise
//! ray test: if the target is close and the crosshair is *roughly* on them,
//! it lands, and it always kills. The server runs [`resolve_melee`] for every
//! stab request; the client's offline Practice mode runs the same function.

use bevy::math::Vec3;

use crate::ballistics::Target;

/// How far ahead of the eye (metres) the stab reaches along the aim
/// direction. Together with [`KNIFE_AIM_SLACK_M`] and the target's own body
/// radius this is roughly a 2.2 m lunge to the surface of a body.
pub const KNIFE_REACH_M: f32 = 1.8;

/// Extra sideways/vertical forgiveness (metres) on top of the target's body
/// radius — the "roughly aiming at them" tolerance.
pub const KNIFE_AIM_SLACK_M: f32 = 0.4;

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

/// Resolve one knife stab from eye position `origin` along `dir` (need not be
/// normalised): the nearest target that's within reach and roughly in front
/// of the aim direction, or `None` for a whiff. Only targets' body capsules
/// count — there's no headshot bonus, a knife hit kills either way.
pub fn resolve_melee(origin: Vec3, dir: Vec3, targets: &[Target]) -> Option<MeleeHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let reach_end = origin + dir * KNIFE_REACH_M;

    let mut best: Option<MeleeHit> = None;
    for t in targets {
        let (on_ray, on_body) = closest_points_between_segments(origin, reach_end, t.body.a, t.body.b);
        if on_ray.distance(on_body) > t.body.radius + KNIFE_AIM_SLACK_M {
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
        let t = target(1, Vec3::new(0.0, 0.0, -1.5));
        let hit = resolve_melee(EYE, Vec3::NEG_Z, &[t]).expect("should stab");
        assert_eq!(hit.target, 1);
    }

    #[test]
    fn stabs_when_only_roughly_aimed() {
        // ~1.5 m ahead, aim ~15° off to the side.
        let t = target(1, Vec3::new(0.0, 0.0, -1.5));
        let dir = Vec3::new(0.27, 0.0, -1.0);
        assert!(resolve_melee(EYE, dir, &[t]).is_some());
    }

    #[test]
    fn whiffs_when_too_far() {
        let t = target(1, Vec3::new(0.0, 0.0, -5.0));
        assert!(resolve_melee(EYE, Vec3::NEG_Z, &[t]).is_none());
    }

    #[test]
    fn whiffs_when_aimed_away() {
        let t = target(1, Vec3::new(0.0, 0.0, -1.5));
        assert!(resolve_melee(EYE, Vec3::Z, std::slice::from_ref(&t)).is_none());
        assert!(resolve_melee(EYE, Vec3::X, std::slice::from_ref(&t)).is_none());
    }

    #[test]
    fn picks_the_nearest_of_several() {
        let far = target(1, Vec3::new(0.0, 0.0, -2.0));
        let near = target(2, Vec3::new(0.0, 0.0, -1.0));
        let hit = resolve_melee(EYE, Vec3::NEG_Z, &[far, near]).unwrap();
        assert_eq!(hit.target, 2);
    }

    #[test]
    fn zero_direction_is_a_whiff() {
        let t = target(1, Vec3::new(0.0, 0.0, -1.0));
        assert!(resolve_melee(EYE, Vec3::ZERO, &[t]).is_none());
    }
}
