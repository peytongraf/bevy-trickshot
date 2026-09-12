//! Player spawn-point placement for [`crate::GameMode::FreeForAll`] (initial
//! spawn and every respawn) — kept separate from [`crate::bots`] since it's
//! spreading *players* out rather than placing static targets.

use bevy::math::Vec3;

use crate::bots::rand01;

/// Spawns land on a ring this far from the map centre — far enough apart
/// that two players rarely start right on top of each other, comfortably
/// inside the sniper's `max_range` (300 m, see [`crate::weapon`]) and the
/// ground's playable extent ([`crate::ballistics::GROUND_HALF_EXTENT`], 100 m).
const RING_MIN_RADIUS: f32 = 15.0;
const RING_MAX_RADIUS: f32 = 70.0;

/// A candidate closer than this to any currently-alive player is rerolled —
/// best-effort spawn protection, not a hard guarantee.
const MIN_SEPARATION: f32 = 12.0;

/// How many times to reroll before giving up and returning the least-bad try.
const RETRIES: u32 = 8;

/// Pick a ground spawn point (position + facing) for `GameMode::FreeForAll`,
/// preferring one at least [`MIN_SEPARATION`] from every position in
/// `others` (typically every other currently-alive player in the lobby).
pub fn spawn_point(seed: u64, others: &[Vec3]) -> (Vec3, f32) {
    let mut best: Option<(Vec3, f32, f32)> = None; // (pos, yaw, min_dist)
    for i in 0..RETRIES {
        let s = seed ^ (i as u64).wrapping_mul(0x2545_f491_4f6c_dd1d);
        let r = RING_MIN_RADIUS + rand01(s) * (RING_MAX_RADIUS - RING_MIN_RADIUS);
        let a = rand01(s ^ 0xa1) * core::f32::consts::TAU;
        let pos = Vec3::new(r * a.cos(), 0.0, r * a.sin());
        let yaw = rand01(s ^ 0xb2) * core::f32::consts::TAU;
        let min_dist = others
            .iter()
            .map(|o| (*o - pos).length())
            .fold(f32::INFINITY, f32::min);
        if min_dist >= MIN_SEPARATION {
            return (pos, yaw);
        }
        if best.map(|(_, _, d)| min_dist > d).unwrap_or(true) {
            best = Some((pos, yaw, min_dist));
        }
    }
    best.map(|(p, y, _)| (p, y)).unwrap_or((Vec3::ZERO, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_point_lands_within_the_ring() {
        for seed in 0..2000u64 {
            let (pos, _) = spawn_point(seed, &[]);
            let r = pos.length();
            assert!(
                (RING_MIN_RADIUS - 1.0e-3..=RING_MAX_RADIUS + 1.0e-3).contains(&r),
                "seed {seed} landed at radius {r}"
            );
        }
    }

    #[test]
    fn spawn_point_avoids_others_when_possible() {
        let others = [Vec3::new(20.0, 0.0, 0.0)];
        for seed in 0..500u64 {
            let (pos, _) = spawn_point(seed, &others);
            let d = (pos - others[0]).length();
            assert!(d >= MIN_SEPARATION - 1.0e-3, "seed {seed} landed {d}m away");
        }
    }
}
