//! Target-bot dimensions and placement, shared so offline Practice and the
//! authoritative server spawn identical bots in identical spots.

use bevy::math::Vec3;

/// Bot hitbox dimensions (fed to [`crate::hitbox::Capsule`]).
pub const BOT_HEIGHT: f32 = 1.8;
pub const BOT_RADIUS: f32 = 0.4;
pub const BOT_HEAD_RADIUS: f32 = 0.14;

/// How many bots a game keeps alive at once.
pub const BOTS_ALIVE: usize = 4;
/// Seconds a shot bot takes to topple flat.
pub const BOT_FALL_SECS: f32 = 0.4;
/// Seconds a dead bot lingers before it's removed (and one respawns).
pub const BOT_DEAD_SECS: f32 = 2.0;

/// Bots respawn on the ground somewhere in this disc, in front of spawn.
pub const BOT_AREA_CENTER: Vec3 = Vec3::new(0.0, 0.0, -5.0);
pub const BOT_AREA_RADIUS: f32 = 10.0;

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

/// A ground position uniformly inside the respawn disc, plus a random facing.
pub fn respawn_pose(seed: u64) -> (Vec3, f32) {
    let r = BOT_AREA_RADIUS * rand01(seed).sqrt();
    let a = rand01(seed ^ 0xa1) * core::f32::consts::TAU;
    let pos = BOT_AREA_CENTER + Vec3::new(r * a.cos(), 0.0, r * a.sin());
    let yaw = rand01(seed ^ 0xb2) * core::f32::consts::TAU;
    (pos, yaw)
}
