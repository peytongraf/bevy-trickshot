//! Target-bot dimensions and placement, shared so offline Practice and the
//! authoritative server spawn identical bots in identical spots.

use bevy::math::Vec3;

/// Bot hitbox dimensions (fed to [`crate::hitbox::Capsule`]).
pub const BOT_HEIGHT: f32 = 1.8;
pub const BOT_RADIUS: f32 = 0.4;
pub const BOT_HEAD_RADIUS: f32 = 0.14;

/// How many bots a game keeps alive at once.
pub const BOTS_ALIVE: usize = 10;
/// Seconds a shot bot takes to topple flat.
pub const BOT_FALL_SECS: f32 = 0.4;
/// Seconds a dead bot lingers before it's removed (and one respawns).
pub const BOT_DEAD_SECS: f32 = 2.0;

/// Bots respawn on the ground somewhere around this point, in front of spawn.
pub const BOT_AREA_CENTER: Vec3 = Vec3::new(0.0, 0.0, -5.0);

/// One concentric distance band a respawning bot can roll into — see
/// [`DISTANCE_TIERS`].
struct DistanceTier {
    /// Share of respawns that land in this tier; the three should sum to 1.
    weight: f32,
    min_radius: f32,
    max_radius: f32,
}

/// Distance tiers for [`respawn_pose`], from [`BOT_AREA_CENTER`]: most bots
/// spawn close to the middle of the map, a good handful at a moderate
/// distance, and a few far out — far enough that hitting one is a real
/// long-range shot (rewarded by `shared::scoring`'s distance multiplier), but
/// still well inside both the sniper's `max_range`
/// (`shared::weapon::WeaponId::Sniper`, 300 m) and the flat ground's
/// playable extent (`shared::ballistics::GROUND_HALF_EXTENT`, 100 m) so a far
/// bot never lands off the edge of the map.
const DISTANCE_TIERS: &[DistanceTier] = &[
    DistanceTier {
        weight: 0.6,
        min_radius: 0.0,
        max_radius: 15.0,
    },
    DistanceTier {
        weight: 0.2,
        min_radius: 15.0,
        max_radius: 40.0,
    },
    DistanceTier {
        weight: 0.2,
        min_radius: 40.0,
        max_radius: 90.0,
    },
];

/// Pick a tier given a `[0, 1)` roll, weighted by [`DistanceTier::weight`].
fn pick_tier(roll: f32) -> &'static DistanceTier {
    let mut acc = 0.0;
    for tier in DISTANCE_TIERS {
        acc += tier.weight;
        if roll < acc {
            return tier;
        }
    }
    // Floating-point rounding could leave `roll` a hair past the last
    // threshold — fall back to the last tier rather than panic.
    DISTANCE_TIERS.last().expect("DISTANCE_TIERS is non-empty")
}

// --- deterministic placement (no `rand` dependency) ---------------------

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Deterministic hash → `[0, 1)`.
pub fn rand01(seed: u64) -> f32 {
    (splitmix64(seed) >> 40) as f32 / (1u64 << 24) as f32
}

/// A ground position around [`BOT_AREA_CENTER`] — uniformly distributed by
/// area within a randomly (weighted) picked [`DistanceTier`] — plus a random
/// facing.
pub fn respawn_pose(seed: u64) -> (Vec3, f32) {
    let tier = pick_tier(rand01(seed ^ 0xc3));
    // Uniform-by-area sampling within an annulus [min_radius, max_radius]:
    // r = sqrt(u * (max² - min²) + min²). `min_radius == 0.0` (the innermost
    // tier) reduces to the usual disc case, r = max * sqrt(u).
    let u = rand01(seed);
    let r = (u * (tier.max_radius * tier.max_radius - tier.min_radius * tier.min_radius)
        + tier.min_radius * tier.min_radius)
        .sqrt();
    let a = rand01(seed ^ 0xa1) * core::f32::consts::TAU;
    let pos = BOT_AREA_CENTER + Vec3::new(r * a.cos(), 0.0, r * a.sin());
    let yaw = rand01(seed ^ 0xb2) * core::f32::consts::TAU;
    (pos, yaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_weights_sum_to_one() {
        let sum: f32 = DISTANCE_TIERS.iter().map(|t| t.weight).sum();
        assert!((sum - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn respawn_pose_never_lands_outside_the_farthest_tier() {
        let max_radius = DISTANCE_TIERS.last().unwrap().max_radius;
        for seed in 0..2000u64 {
            let (pos, _) = respawn_pose(seed);
            let r = (pos - BOT_AREA_CENTER).length();
            assert!(r <= max_radius + 1.0e-3, "seed {seed} landed at radius {r}");
        }
    }

    #[test]
    fn respawn_pose_uses_every_tier_over_many_rolls() {
        // Statistical, not exact: over enough seeds every tier should show up
        // at roughly its configured share, and in particular the far tier
        // ("a few" bots at distance) should never come up empty.
        let mut counts = [0u32; 3];
        const N: u64 = 5000;
        for seed in 0..N {
            let (pos, _) = respawn_pose(seed);
            let r = (pos - BOT_AREA_CENTER).length();
            let idx = DISTANCE_TIERS
                .iter()
                .position(|t| r >= t.min_radius - 1.0e-3 && r <= t.max_radius + 1.0e-3)
                .expect("radius should fall in exactly one tier's range");
            counts[idx] += 1;
        }
        for (tier, count) in DISTANCE_TIERS.iter().zip(counts) {
            let frac = count as f32 / N as f32;
            assert!(
                (frac - tier.weight).abs() < 0.05,
                "tier weight {} got fraction {frac}",
                tier.weight
            );
        }
    }
}
