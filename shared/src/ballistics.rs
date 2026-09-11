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

/// Extra range (metres) a hitscan bullet burns punching through a pierced
/// target — modelling the energy lost, so a lined-up chain of bots can't
/// collateral forever regardless of `WeaponSpec::max_range`. Purely a range
/// cost; damage falloff (`damage_for`) is unaffected, since bots die to any
/// hit anyway and this only needs to cap *how far* the chain can reach.
pub const PIERCE_RANGE_COST_M: f32 = 15.0;

/// Nearest target along `origin + t·dir` (`t` in `0..=max_dist`) not already in
/// `exclude`, or `None` on a clean miss / everything excluded / blocked.
/// Shared by [`segment_hit`] (single hit) and [`resolve_shot_pierce`] (chases
/// this leg by leg, excluding what it's already pierced).
fn nearest_unpierced(
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
    targets: &[Target],
    exclude: &[u64],
    blocked: &mut impl FnMut(Vec3, Vec3) -> bool,
) -> Option<(u64, bool, f32, Vec3)> {
    let mut best: Option<(u64, bool, f32, Vec3)> = None;
    for t in targets {
        if exclude.contains(&t.id) {
            continue;
        }
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
        if dist > max_dist || best.is_some_and(|(_, _, best_dist, _)| dist >= best_dist) {
            continue;
        }
        let point = origin + dir * dist;
        if blocked(origin, point) {
            continue;
        }
        best = Some((t.id, headshot, dist, point));
    }
    best
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
    let (target, headshot, dist, point) =
        nearest_unpierced(origin, dir, max_dist, targets, &[], blocked)?;
    Some(ShotHit {
        target,
        headshot,
        point,
        distance: dist,
        damage: damage_for(weapon, dist, headshot),
    })
}

/// Resolve a single hitscan bullet allowing it to pierce through however many
/// targets lie along its path — Call-of-Duty "collateral" style: a hit doesn't
/// stop the bullet, it just spends [`PIERCE_RANGE_COST_M`] of the remaining
/// range and keeps going. The chain ends when the bullet runs out of range, is
/// stopped by map geometry, or has nothing left in front of it. Occlusion is
/// tested leg by leg (last hit → next candidate), so a wall between two
/// targets stops the chain there even if a farther target would otherwise be
/// reachable.
///
/// Returns every hit, nearest first — empty on a clean miss. Projectile
/// weapons (bullet drop / travel time) don't pierce: this just falls back to
/// [`resolve_shot`]'s single hit for those, wrapped in a `Vec`.
pub fn resolve_shot_pierce(
    weapon: WeaponId,
    origin: Vec3,
    dir: Vec3,
    targets: &[Target],
    mut blocked: impl FnMut(Vec3, Vec3) -> bool,
) -> Vec<ShotHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return Vec::new();
    }
    let spec = weapon.spec();
    if !weapon.is_hitscan() {
        return resolve_shot(weapon, origin, dir, targets, blocked)
            .into_iter()
            .collect();
    }

    let mut hits: Vec<ShotHit> = Vec::new();
    let mut pierced: Vec<u64> = Vec::new();
    let mut leg_start = origin;
    let mut budget = spec.max_range;

    while budget > 0.0 {
        let Some((target, headshot, leg_dist, point)) =
            nearest_unpierced(leg_start, dir, budget, targets, &pierced, &mut blocked)
        else {
            break;
        };
        pierced.push(target);
        let distance = (point - origin).length();
        hits.push(ShotHit {
            target,
            headshot,
            point,
            distance,
            damage: damage_for(weapon, distance, headshot),
        });
        budget -= leg_dist + PIERCE_RANGE_COST_M;
        leg_start = point;
    }
    hits
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

    #[test]
    fn pierce_hits_every_lined_up_target_nearest_first() {
        let targets = [
            dummy(1, Vec3::new(0.0, 0.0, -10.0)),
            dummy(2, Vec3::new(0.0, 0.0, -20.0)),
            dummy(3, Vec3::new(0.0, 0.0, -30.0)),
        ];
        let hits = resolve_shot_pierce(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| false,
        );
        let ids: Vec<u64> = hits.iter().map(|h| h.target).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        // Nearest-first and strictly increasing, since each is farther along
        // the same ray.
        assert!(hits.windows(2).all(|w| w[0].distance < w[1].distance));
    }

    #[test]
    fn pierce_stops_at_map_geometry_between_targets() {
        let targets = [
            dummy(1, Vec3::new(0.0, 0.0, -10.0)),
            dummy(2, Vec3::new(0.0, 0.0, -20.0)),
        ];
        // A "wall" that blocks any leg starting past the first target, so the
        // bullet should never reach the second.
        let hits = resolve_shot_pierce(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &targets,
            |a: Vec3, _b: Vec3| a.z < -5.0,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, 1);
    }

    #[test]
    fn pierce_gives_up_once_out_of_range() {
        // Sniper max range is 300 m; three targets 149 m apart put the third
        // just past what's left once each pierce burns `PIERCE_RANGE_COST_M`.
        let targets = [
            dummy(1, Vec3::new(0.0, 0.0, -1.0)),
            dummy(2, Vec3::new(0.0, 0.0, -150.0)),
            dummy(3, Vec3::new(0.0, 0.0, -299.0)),
        ];
        let hits = resolve_shot_pierce(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| false,
        );
        let ids: Vec<u64> = hits.iter().map(|h| h.target).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn pierce_falls_back_to_single_hit_for_projectile_weapons() {
        // The Marksman has travel time / drop, so it doesn't pierce — even
        // lined-up targets should only ever yield the first hit.
        let targets = [
            dummy(1, Vec3::new(0.0, 0.0, -10.0)),
            dummy(2, Vec3::new(0.0, 0.0, -20.0)),
        ];
        let hits = resolve_shot_pierce(
            WeaponId::Marksman,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| false,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, 1);
    }
}
