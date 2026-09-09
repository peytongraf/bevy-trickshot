//! Authoritative shot resolution. The server runs [`resolve_shot`] for every
//! fire request; the client can run the same function to predict the result.

use bevy::math::Vec3;

use crate::hitbox::{ray_capsule, Capsule};
use crate::weapon::WeaponId;

/// One player considered as a shot target for the tick being resolved.
pub struct Target {
    /// Opaque id echoed back in [`ShotHit::target`]. Use `PeerId::to_bits()`.
    pub id: u64,
    pub body: Capsule,
    pub head: Capsule,
}

/// The winning hit from [`resolve_shot`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotHit {
    pub target: u64,
    pub headshot: bool,
    /// World-space impact point.
    pub point: Vec3,
    /// Distance from the muzzle to the impact.
    pub distance: f32,
    pub damage: f32,
}

/// Resolve a single shot.
///
/// * `origin` / `dir` — muzzle position and aim direction (`dir` need not be
///   normalised).
/// * `targets` — every *other* player's hitboxes for the tick being resolved.
///   For a fair head-to-head you'll want these rewound to the shooter's view of
///   the world; see the lag-compensation note in `server/README.md`.
/// * `blocked` — static-geometry occlusion: return `true` when the segment from
///   `a` to `b` is stopped by the map. Pass `|_, _| false` until you wire one up
///   (see [`crate::map::CollisionWorld`]).
///
/// Returns the closest valid hit, or `None` for a clean miss.
pub fn resolve_shot(
    weapon: WeaponId,
    origin: Vec3,
    dir: Vec3,
    targets: &[Target],
    mut blocked: impl FnMut(Vec3, Vec3) -> bool,
) -> Option<ShotHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let spec = weapon.spec();

    if weapon.is_hitscan() {
        return segment_hit(weapon, origin, dir, spec.max_range, targets, &mut blocked);
    }

    // Projectile: walk the trajectory in short steps, testing each step segment.
    const STEP_M: f32 = 4.0;
    let dt = STEP_M / spec.muzzle_velocity;
    let mut pos = origin;
    let mut vel = dir * spec.muzzle_velocity;
    let mut travelled = 0.0f32;

    while travelled < spec.max_range {
        let next = pos + vel * dt;
        let seg = next - pos;
        let seg_len = seg.length();
        if seg_len > 1e-6 {
            let remaining = spec.max_range - travelled;
            if let Some(mut hit) = segment_hit(
                weapon,
                pos,
                seg / seg_len,
                seg_len.min(remaining),
                targets,
                &mut blocked,
            ) {
                hit.distance += travelled;
                hit.damage = damage_for(weapon, hit.distance, hit.headshot);
                return Some(hit);
            }
        }
        pos = next;
        vel += Vec3::NEG_Y * spec.gravity * dt;
        travelled += seg_len;
    }
    None
}

/// Ray test over the bounded segment `origin .. origin + max_dist * dir`.
fn segment_hit(
    weapon: WeaponId,
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
    targets: &[Target],
    blocked: &mut impl FnMut(Vec3, Vec3) -> bool,
) -> Option<ShotHit> {
    let mut best: Option<ShotHit> = None;
    for t in targets {
        // If the ray passes through the head hitbox at all it's a headshot — the
        // head is the smaller, more specific target and it overlaps the top of
        // the body capsule. Generous headshots suit a trickshot game; tighten
        // this (e.g. only when `head_dist <= body_dist`) if you want it stricter.
        let (dist, headshot) = match (
            ray_capsule(origin, dir, &t.head),
            ray_capsule(origin, dir, &t.body),
        ) {
            (Some(head_dist), _) => (head_dist, true),
            (None, Some(body_dist)) => (body_dist, false),
            (None, None) => continue,
        };
        if dist > max_dist {
            continue;
        }
        let point = origin + dir * dist;
        if blocked(origin, point) {
            continue;
        }
        if best.map_or(true, |b| dist < b.distance) {
            best = Some(ShotHit {
                target: t.id,
                headshot,
                point,
                distance: dist,
                damage: damage_for(weapon, dist, headshot),
            });
        }
    }
    best
}

/// Height of the flat ground plane.
pub const GROUND_Y: f32 = 0.0;
/// Half-extent of the ground plane on X and Z (metres).
pub const GROUND_HALF_EXTENT: f32 = 100.0;

/// Where a clean-miss shot meets the ground, or `None` if the ray points up or
/// lands beyond the ground plane. `dir` need not be normalised.
pub fn ground_impact(origin: Vec3, dir: Vec3) -> Option<Vec3> {
    let dir = dir.normalize_or_zero();
    if dir.y >= -1.0e-4 {
        return None; // level or rising — never meets the ground ahead
    }
    let t = (GROUND_Y - origin.y) / dir.y;
    if t <= 0.0 {
        return None;
    }
    let point = origin + dir * t;
    if point.x.abs() > GROUND_HALF_EXTENT || point.z.abs() > GROUND_HALF_EXTENT {
        return None;
    }
    Some(point)
}

fn damage_for(weapon: WeaponId, distance: f32, headshot: bool) -> f32 {
    let spec = weapon.spec();
    let f = (distance / spec.max_range).clamp(0.0, 1.0);
    let falloff = 1.0 - f * (1.0 - spec.min_damage_fraction);
    let mult = if headshot {
        spec.headshot_multiplier
    } else {
        1.0
    };
    spec.base_damage * falloff * mult
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(id: u64, feet: Vec3) -> Target {
        Target {
            id,
            body: Capsule::standing(feet, 1.8, 0.35),
            head: Capsule::head(feet, 1.8, 0.12),
        }
    }

    #[test]
    fn hitscan_center_mass_hits_nearest() {
        let targets = [dummy(1, Vec3::new(0.0, 0.0, -10.0)), dummy(2, Vec3::new(0.0, 0.0, -30.0))];
        let hit = resolve_shot(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| false,
        )
        .expect("should hit");
        assert_eq!(hit.target, 1);
        assert!(!hit.headshot);
    }

    #[test]
    fn map_geometry_blocks_the_shot() {
        let targets = [dummy(1, Vec3::new(0.0, 0.0, -10.0))];
        let hit = resolve_shot(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| true,
        );
        assert!(hit.is_none());
    }

    #[test]
    fn headshot_multiplies_damage() {
        let targets = [dummy(7, Vec3::new(0.0, 0.0, -12.0))];
        let hit = resolve_shot(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.68, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| false,
        )
        .expect("should hit");
        assert!(hit.headshot);
        assert!(hit.damage >= WeaponId::Sniper.spec().base_damage * 1.9);
    }
}
