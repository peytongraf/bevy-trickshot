//! Authoritative shot resolution. The server runs [`resolve_shot`] for every
//! fire request with no client behind it (a bot's), testing the shot against
//! simple capsules. A real player's shot is tested on their own client
//! against every enemy's actual animated model, exactly as it was drawn
//! there — an arm, a finger — and sent up as [`Claim`]s, which the server
//! checks over with [`resolve_claimed_hits`] before anything takes damage.

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

use crate::hitbox::{ray_capsule, Capsule};
use crate::weapon::WeaponId;

/// One player considered as a shot target for the tick being resolved.
pub struct Target {
    /// Opaque id echoed back in [`ShotHit::target`]. Use `PeerId::to_bits()`.
    pub id: u64,
    pub body: Capsule,
    pub head: Capsule,
}

/// Where on a target a shot landed — each zone has its own damage multiplier
/// (see [`crate::weapon::WeaponSpec`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitZone {
    Head,
    /// Upper body (waist and up, arms included) — full body-shot damage.
    Torso,
    /// Lower body (below [`LOWER_BODY_HEIGHT_FRAC`] of the target's height).
    Legs,
}

/// A body hit below this fraction of the target's height (`0` at the feet,
/// `1` at the top of the head) is a [`HitZone::Legs`] hit.
pub const LOWER_BODY_HEIGHT_FRAC: f32 = 0.5;

/// Which zone a hit at `point` on `target` falls in.
fn zone_of(target: &Target, point: Vec3, headshot: bool) -> HitZone {
    if headshot {
        return HitZone::Head;
    }
    let feet_y = target.body.a.y.min(target.body.b.y) - target.body.radius;
    let height = (target.body.b.y - target.body.a.y).abs() + 2.0 * target.body.radius;
    if height > 1e-3 && (point.y - feet_y) / height < LOWER_BODY_HEIGHT_FRAC {
        HitZone::Legs
    } else {
        HitZone::Torso
    }
}

/// The winning hit from [`resolve_shot`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotHit {
    pub target: u64,
    pub headshot: bool,
    pub zone: HitZone,
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
                hit.damage = damage_for(weapon, hit.distance, hit.zone);
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
    let zone = targets
        .iter()
        .find(|t| t.id == target)
        .map_or(HitZone::Torso, |t| zone_of(t, point, headshot));
    Some(ShotHit {
        target,
        headshot,
        zone,
        point,
        distance: dist,
        damage: damage_for(weapon, dist, zone),
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
        let zone = targets
            .iter()
            .find(|t| t.id == target)
            .map_or(HitZone::Torso, |t| zone_of(t, point, headshot));
        hits.push(ShotHit {
            target,
            headshot,
            zone,
            point,
            distance,
            damage: damage_for(weapon, distance, zone),
        });
        budget -= leg_dist + PIERCE_RANGE_COST_M;
        leg_start = point;
    }
    hits
}

/// The path a `weapon`'s bullet fired from `origin` along `dir` (unit)
/// follows, as a polyline out to its `max_range`: one straight leg for a
/// hitscan weapon, the same short steps of its drop [`resolve_shot`] walks
/// for a projectile one.
pub fn shot_path(weapon: WeaponId, origin: Vec3, dir: Vec3) -> Vec<Vec3> {
    let spec = weapon.spec();
    if weapon.is_hitscan() {
        return vec![origin, origin + dir * spec.max_range];
    }
    const STEP_M: f32 = 4.0;
    let dt = STEP_M / spec.muzzle_velocity;
    let mut path = vec![origin];
    let mut pos = origin;
    let mut vel = dir * spec.muzzle_velocity;
    let mut travelled = 0.0f32;
    while travelled < spec.max_range {
        let next = pos + vel * dt;
        let seg_len = (next - pos).length();
        if seg_len <= 1e-6 {
            break;
        }
        let remaining = spec.max_range - travelled;
        path.push(pos + (next - pos) * (remaining / seg_len).min(1.0));
        pos = next;
        vel += Vec3::NEG_Y * spec.gravity * dt;
        travelled += seg_len;
    }
    path
}

/// Where `point` sits along `path` ([`shot_path`]): how far down it (m) its
/// nearest spot is, and how far off the path it is.
pub fn along_path(path: &[Vec3], point: Vec3) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    let mut travelled = 0.0;
    for leg in path.windows(2) {
        let (a, b) = (leg[0], leg[1]);
        let ab = b - a;
        let len = ab.length();
        if len <= 1e-6 {
            continue;
        }
        let t = ((point - a).dot(ab) / (len * len)).clamp(0.0, 1.0);
        let off = point.distance(a + ab * t);
        if best.is_none_or(|(_, o)| off < o) {
            best = Some((travelled + t * len, off));
        }
        travelled += len;
    }
    best
}

/// One hit a player's client found its shot made on a target's model
/// (`target`: a [`Target::id`]), where (`point`) and on which part of it
/// (`zone`: the bone it struck — [`crate::hitbox::zone_of_bone`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Claim {
    pub target: u64,
    pub zone: HitZone,
    pub point: Vec3,
}

/// How far (m) a [`Claim`]'s point may lie off the shot's path — rounding,
/// and the camera having moved a hair between drawing and sending.
pub const CLAIM_PATH_SLACK_M: f32 = 0.3;

/// How far (m) a [`Claim`]'s point may lie outside its target's body capsule
/// on the server: the shooter saw everyone a moment in the past (the
/// interpolation delay plus the trip up), so a fast hellhound can be a few
/// metres on by now — and an arm flung out reaches past the capsule anyway.
pub const CLAIM_BODY_SLACK_M: f32 = 3.0;

/// The server's side of a player's shot: check each of their client's
/// [`Claim`]s — its target's still one (in `targets`), the point's on the
/// shot's path, near where that target really is, and in front of the first
/// wall (`wall_dist`, along the shot from `origin`) — and turn the ones that
/// hold up into hits, nearest first, with the same piercing rules (and
/// damage) as [`resolve_shot_pierce`]. Each target counts once, at the
/// nearest point claimed on it.
pub fn resolve_claimed_hits(
    weapon: WeaponId,
    origin: Vec3,
    dir: Vec3,
    claims: &[Claim],
    targets: &[Target],
    wall_dist: Option<f32>,
) -> Vec<ShotHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return Vec::new();
    }
    let path = shot_path(weapon, origin, dir);
    let mut valid: Vec<(f32, Claim)> = Vec::new();
    for claim in claims {
        let Some(target) = targets.iter().find(|t| t.id == claim.target) else {
            continue;
        };
        if !near_target(target, claim.point) {
            continue;
        }
        let Some((along, off)) = along_path(&path, claim.point) else {
            continue;
        };
        if off > CLAIM_PATH_SLACK_M || wall_dist.is_some_and(|d| origin.distance(claim.point) > d) {
            continue;
        }
        match valid.iter_mut().find(|(_, c)| c.target == claim.target) {
            Some(prev) if prev.0 <= along => {}
            Some(prev) => *prev = (along, *claim),
            None => valid.push((along, *claim)),
        }
    }
    valid.sort_by(|a, b| a.0.total_cmp(&b.0));
    if !weapon.is_hitscan() {
        // (A projectile stops in the first thing it hits.)
        valid.truncate(1);
    }

    let mut hits = Vec::new();
    let mut budget = weapon.spec().max_range;
    let mut last = 0.0;
    for (along, claim) in valid {
        let leg = along - last;
        if leg > budget {
            break;
        }
        budget -= leg + PIERCE_RANGE_COST_M;
        last = along;
        hits.push(ShotHit {
            target: claim.target,
            headshot: claim.zone == HitZone::Head,
            zone: claim.zone,
            point: claim.point,
            distance: along,
            damage: damage_for(weapon, along, claim.zone),
        });
    }
    hits
}

/// Whether a point a client claims it hit `target` at is close enough to
/// where the server has them ([`CLAIM_BODY_SLACK_M`]).
pub fn near_target(target: &Target, point: Vec3) -> bool {
    let body = &target.body;
    segment_point_distance(body.a, body.b, point) <= body.radius + CLAIM_BODY_SLACK_M
}

/// How far `p` is from the segment `a`..`b`.
fn segment_point_distance(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 1e-12 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
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

fn damage_for(weapon: WeaponId, distance: f32, zone: HitZone) -> f32 {
    let spec = weapon.spec();
    // Full damage out to `falloff_start`, then linear to `min_damage_fraction`
    // at `max_range`.
    let span = (spec.max_range - spec.falloff_start).max(1e-3);
    let f = ((distance - spec.falloff_start) / span).clamp(0.0, 1.0);
    let falloff = 1.0 - f * (1.0 - spec.min_damage_fraction);
    let mult = match zone {
        HitZone::Head => spec.headshot_multiplier,
        HitZone::Torso => 1.0,
        HitZone::Legs => spec.lower_body_multiplier,
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

    /// The occlusion rule the server uses: a target's impact point farther
    /// along the shot than the first solid surface is behind it. Bots in front
    /// of the wall still die (and pierce), ones behind it don't.
    #[test]
    fn a_wall_stops_a_pierce_chain() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let wall_dist = Some(20.0f32);
        let targets = [
            dummy(1, Vec3::new(0.0, 0.0, -10.0)), // in front of the wall
            dummy(2, Vec3::new(0.0, 0.0, -15.0)), // in front of the wall
            dummy(3, Vec3::new(0.0, 0.0, -30.0)), // behind it
        ];
        let hits = resolve_shot_pierce(WeaponId::Sniper, origin, Vec3::NEG_Z, &targets, |_, to| {
            wall_dist.is_some_and(|d| origin.distance(to) > d)
        });
        let ids: Vec<u64> = hits.iter().map(|h| h.target).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    fn claim(target: u64, zone: HitZone, point: Vec3) -> Claim {
        Claim { target, zone, point }
    }

    #[test]
    fn a_claimed_hit_on_the_shot_and_its_target_counts_with_its_zone() {
        let targets = [dummy(1, Vec3::new(0.0, 0.0, -10.0))];
        // An outflung arm: past the capsule, but on the shot.
        let point = Vec3::new(0.0, 1.0, -10.0 + 0.9);
        let hits = resolve_claimed_hits(
            WeaponId::Sniper,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::NEG_Z,
            &[claim(1, HitZone::Head, point)],
            &targets,
            None,
        );
        assert_eq!(hits.len(), 1);
        assert!(hits[0].headshot && hits[0].zone == HitZone::Head);
        assert!((hits[0].distance - 9.1).abs() < 1e-3);
        assert_eq!(hits[0].damage, damage_for(WeaponId::Sniper, 9.1, HitZone::Head));
    }

    #[test]
    fn a_claim_off_the_shot_far_from_its_target_or_on_no_target_is_thrown_out() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let targets = [dummy(1, Vec3::new(0.0, 0.0, -10.0)), dummy(2, Vec3::new(0.0, 0.0, -40.0))];
        let bad = [
            // Not on the shot (a metre to the side).
            claim(1, HitZone::Torso, Vec3::new(1.0, 1.0, -10.0)),
            // On the shot, but nowhere near target 2.
            claim(2, HitZone::Torso, Vec3::new(0.0, 1.0, -20.0)),
            // No such target.
            claim(9, HitZone::Torso, Vec3::new(0.0, 1.0, -10.0)),
        ];
        let hits = resolve_claimed_hits(WeaponId::Sniper, origin, Vec3::NEG_Z, &bad, &targets, None);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn claims_behind_a_wall_dont_count_and_the_rest_pierce_nearest_first() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let targets = [
            dummy(1, Vec3::new(0.0, 0.0, -10.0)),
            dummy(2, Vec3::new(0.0, 0.0, -15.0)),
            dummy(3, Vec3::new(0.0, 0.0, -30.0)),
        ];
        let claims = [
            claim(3, HitZone::Torso, Vec3::new(0.0, 1.0, -29.7)),
            claim(2, HitZone::Legs, Vec3::new(0.0, 1.0, -14.7)),
            claim(1, HitZone::Torso, Vec3::new(0.0, 1.0, -9.7)),
            // A second point on target 1, farther in: only the first counts.
            claim(1, HitZone::Head, Vec3::new(0.0, 1.0, -10.2)),
        ];
        let hits = resolve_claimed_hits(WeaponId::Sniper, origin, Vec3::NEG_Z, &claims, &targets, Some(20.0));
        let got: Vec<(u64, HitZone)> = hits.iter().map(|h| (h.target, h.zone)).collect();
        assert_eq!(got, vec![(1, HitZone::Torso), (2, HitZone::Legs)]);
    }

    #[test]
    fn a_projectiles_claim_follows_its_drop_and_stops_at_the_first() {
        let origin = Vec3::new(0.0, 1.5, 0.0);
        let path = shot_path(WeaponId::Marksman, origin, Vec3::NEG_Z);
        // A spot well down range, where the bullet has really dropped to.
        let far = path.iter().copied().find(|p| p.z < -250.0).unwrap();
        assert!(far.y < origin.y - 0.1, "no drop by {far:?}");
        let targets = [
            dummy(1, Vec3::new(0.0, far.y - 1.0, far.z)),
            dummy(2, Vec3::new(0.0, far.y - 1.0, far.z - 5.0)),
        ];
        let claims = [
            claim(1, HitZone::Torso, far),
            claim(2, HitZone::Torso, far + Vec3::NEG_Z * 5.0),
        ];
        let hits = resolve_claimed_hits(WeaponId::Marksman, origin, Vec3::NEG_Z, &claims, &targets, None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, 1);
        // ...and the same point with no drop (straight along the aim) is off it.
        let straight = Vec3::new(0.0, origin.y, far.z);
        let none = resolve_claimed_hits(
            WeaponId::Marksman,
            origin,
            Vec3::NEG_Z,
            &[claim(1, HitZone::Torso, straight)],
            &[dummy(1, Vec3::new(0.0, origin.y - 1.0, far.z))],
            None,
        );
        assert!(none.is_empty(), "{none:?}");
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

    /// A bullet at `height` metres up a standing dummy `range` metres away.
    fn shoot(range: f32, height: f32) -> ShotHit {
        let targets = [dummy(7, Vec3::new(0.0, 0.0, -range))];
        resolve_shot(
            WeaponId::Sniper,
            Vec3::new(0.0, height, 0.0),
            Vec3::NEG_Z,
            &targets,
            |_, _| false,
        )
        .expect("should hit")
    }

    const HEALTH: f32 = 100.0;
    // Chest / head / leg heights on the 1.8 m dummy.
    const TORSO_Y: f32 = 1.2;
    const HEAD_Y: f32 = 1.68;
    const LEGS_Y: f32 = 0.4;

    #[test]
    fn zones_are_classified_from_the_hit_height() {
        assert_eq!(shoot(12.0, TORSO_Y).zone, HitZone::Torso);
        assert_eq!(shoot(12.0, HEAD_Y).zone, HitZone::Head);
        assert_eq!(shoot(12.0, LEGS_Y).zone, HitZone::Legs);
    }

    #[test]
    fn up_close_everything_kills_even_a_leg_shot() {
        for y in [TORSO_Y, HEAD_Y, LEGS_Y] {
            let d = shoot(10.0, y).damage;
            assert!(d >= HEALTH, "at 10 m a shot at height {y} must kill, did {d}");
        }
    }

    #[test]
    fn a_leg_shot_stops_killing_at_range_but_a_torso_shot_still_does() {
        let leg = shoot(120.0, LEGS_Y).damage;
        assert!(leg > 0.0 && leg < HEALTH, "leg shot at 120 m must not kill: {leg}");
        let torso = shoot(120.0, TORSO_Y).damage;
        assert!(torso >= HEALTH, "torso shot at 120 m must still kill: {torso}");
    }

    #[test]
    fn far_enough_away_a_torso_shot_no_longer_kills_but_a_headshot_does() {
        let torso = shoot(220.0, TORSO_Y).damage;
        assert!(torso > 0.0 && torso < HEALTH, "torso at 220 m must not kill: {torso}");
        let head = shoot(220.0, HEAD_Y).damage;
        assert!(head >= HEALTH, "headshot at 220 m must still kill: {head}");
    }

    #[test]
    fn at_extreme_range_even_a_headshot_does_not_kill() {
        let head = shoot(290.0, HEAD_Y).damage;
        assert!(head > 0.0 && head < HEALTH, "headshot at 290 m must not kill: {head}");
    }

    #[test]
    fn damage_never_rises_with_distance() {
        for y in [TORSO_Y, HEAD_Y, LEGS_Y] {
            let mut last = f32::MAX;
            for range in [5.0, 20.0, 60.0, 120.0, 200.0, 280.0] {
                let d = shoot(range, y).damage;
                assert!(d <= last + 1e-3, "damage rose from {last} to {d} at {range} m");
                last = d;
            }
        }
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
