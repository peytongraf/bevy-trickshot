//! Small deterministic-math and color-conversion helpers with no dependency
//! on any particular game system — used across environment, weapon and VFX
//! code alike.

use bevy::prelude::Color;

/// Cheap deterministic hash → a float in `[0, 1)`.
pub(crate) fn rand01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    x ^= x >> 22;
    x as f32 / u32::MAX as f32
}

/// Cheap deterministic hash → a roll angle in `[-PI, PI)`.
pub(crate) fn rand_roll(seed: u32) -> f32 {
    rand01(seed) * (std::f32::consts::PI * 2.0) - std::f32::consts::PI
}

pub(crate) fn srgb_parts(c: Color) -> [f32; 3] {
    let s = c.to_srgba();
    [s.red, s.green, s.blue]
}

pub(crate) fn color_from_parts(p: [f32; 3]) -> Color {
    Color::srgb(p[0], p[1], p[2])
}

/// Smoothstep easing, used for the scope sight-picture fade.
pub(crate) fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The hip↔ADS blend curve. `amount` blends from a straight line (`0`) to
/// smootherstep (`1`), so the aim-in slows into both ends by a tunable degree.
pub(crate) fn ads_ease(t: f32, amount: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let smootherstep = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    t + (smootherstep - t) * amount.clamp(0.0, 1.0)
}

/// A `Zombies` thing's root standing at `at` (`shared::level`): on its
/// spot, turned its way, at its scale — its model, solid box and light are
/// children, so they all scale with it.
pub(crate) fn placed(at: shared::level::Placement) -> bevy::prelude::Transform {
    bevy::prelude::Transform::from_translation(at.pos)
        .with_rotation(at.rotation())
        .with_scale(bevy::prelude::Vec3::splat(at.scale.max(0.01)))
}
