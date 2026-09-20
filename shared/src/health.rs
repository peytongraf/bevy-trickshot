//! Player health rules — the numbers the server applies (`server::pvp`) and
//! the tests that pin them. Health is entirely server-side: the client only
//! reports *that* it landed from some height ([`crate::FallLanded`]) and shows
//! whatever health the server replicates back ([`crate::PlayerHealth`]).

/// Every player starts (and respawns) at this much health. The sniper's
/// close-range body shot kills outright against it (`weapon::WeaponId::spec`).
pub const FULL_HEALTH: f32 = 100.0;

/// A fall (apex → landing, metres) shorter than this does no damage.
pub const FALL_MIN_DISTANCE: f32 = 16.0;
/// A fall this far or farther kills. Between [`FALL_MIN_DISTANCE`] and this,
/// damage rises linearly from nothing to a full health bar.
pub const FALL_MAX_DISTANCE: f32 = 30.0;
/// Clients only bother reporting landings from at least this far up — well
/// under [`FALL_MIN_DISTANCE`], so the client never has to know the real
/// damage thresholds.
pub const FALL_REPORT_MIN_DISTANCE: f32 = 2.0;

/// Seconds after taking damage that health holds before it starts to recover.
pub const REGEN_DELAY_SECS: f32 = 3.0;
/// Health regained per second once recovery starts.
pub const REGEN_PER_SEC: f32 = 20.0;

/// Health lost landing from a fall of `distance` metres: `0` at or below
/// [`FALL_MIN_DISTANCE`], [`FULL_HEALTH`] (a kill) at or above
/// [`FALL_MAX_DISTANCE`], linear in between.
pub fn fall_damage(distance: f32) -> f32 {
    if !distance.is_finite() {
        return 0.0;
    }
    let span = (FALL_MAX_DISTANCE - FALL_MIN_DISTANCE).max(1e-3);
    ((distance - FALL_MIN_DISTANCE) / span).clamp(0.0, 1.0) * FULL_HEALTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_damage_under_the_minimum_linear_after_and_lethal_at_the_maximum() {
        assert_eq!(fall_damage(3.0), 0.0);
        assert_eq!(fall_damage(FALL_MIN_DISTANCE), 0.0);
        let mid = (FALL_MIN_DISTANCE + FALL_MAX_DISTANCE) * 0.5;
        assert!((fall_damage(mid) - FULL_HEALTH * 0.5).abs() < 1e-3);
        assert!((fall_damage(FALL_MAX_DISTANCE) - FULL_HEALTH).abs() < 1e-3);
        assert!((fall_damage(500.0) - FULL_HEALTH).abs() < 1e-3);
    }

    #[test]
    fn nonsense_distances_do_no_damage() {
        assert_eq!(fall_damage(f32::NAN), 0.0);
        assert_eq!(fall_damage(f32::INFINITY), 0.0);
        assert_eq!(fall_damage(-5.0), 0.0);
    }

    #[test]
    fn the_report_threshold_is_below_the_damage_threshold() {
        assert!(FALL_REPORT_MIN_DISTANCE < FALL_MIN_DISTANCE);
    }
}
