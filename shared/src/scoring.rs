//! Style-point scoring for a landed shot. Server-authoritative: the client
//! ships the trick metadata (spin degrees, airborne, no-scope) on its fire
//! input and the server turns a confirmed bot kill into points here. Kept in
//! `shared` so the client can predict the same numbers later.

use crate::protocol::ScoreLine;

/// Flat points for the kill itself.
pub const KILL_POINTS: u32 = 10;
/// Points per completed 180° of spin.
pub const SPIN_PER_180: u32 = 25;
/// Spin points are multiplied by this when any of the spin was airborne.
pub const MIDAIR_MULT: u32 = 2;
/// Points for landing the shot with no scope at all.
pub const NOSCOPE_POINTS: u32 = 30;

/// Breakdown + total for a bot kill given the shot's trick metadata. The first
/// line is always the base kill; spin / no-scope lines are added when earned.
pub fn score_kill(spin_deg: f32, airborne: bool, noscope: bool) -> (u32, Vec<ScoreLine>) {
    let mut lines = vec![ScoreLine {
        label: "BOT KILL".to_string(),
        points: KILL_POINTS,
    }];

    let halves = (spin_deg.max(0.0) / 180.0).floor() as u32;
    if halves >= 1 {
        let degrees = halves * 180;
        let mut points = halves * SPIN_PER_180;
        let label = if airborne {
            points *= MIDAIR_MULT;
            format!("MIDAIR {degrees}\u{00B0} SPIN")
        } else {
            format!("{degrees}\u{00B0} SPIN")
        };
        lines.push(ScoreLine { label, points });
    }

    if noscope {
        lines.push(ScoreLine {
            label: "NO SCOPE".to_string(),
            points: NOSCOPE_POINTS,
        });
    }

    let total = lines.iter().map(|l| l.points).sum();
    (total, lines)
}

/// Score multiplier at full `max_range` — a point-blank kill is worth `1.0×`
/// this; that's what it eases up to (linearly with distance) by the time a
/// shot has travelled the weapon's whole effective range. Rewards the harder,
/// longer shot, the way a trickshot game should.
pub const MAX_DISTANCE_MULT: f32 = 3.0;

/// Score multiplier for a shot that travelled `distance` metres out of a
/// weapon whose effective range is `max_range`: `1.0` at the muzzle, easing
/// linearly up to [`MAX_DISTANCE_MULT`] at `max_range` (and clamped there, so
/// a shot a hair past the nominal range from floating-point slop is never
/// worth less than one just inside it).
pub fn distance_multiplier(distance: f32, max_range: f32) -> f32 {
    let t = (distance / max_range.max(1.0)).clamp(0.0, 1.0);
    1.0 + t * (MAX_DISTANCE_MULT - 1.0)
}

/// Breakdown + total for a shot that killed `bots_hit` bots in one pull of the
/// trigger, `distance` metres from the muzzle to the first (nearest) one hit.
/// The shot's own trick lines (kill + spin + no-scope) are worked out once via
/// [`score_kill`], then two multipliers stack on top of that base, each broken
/// out as its own line so the popup shows exactly where the bonus came from
/// instead of just a bigger kill total:
///
/// * **Distance** ([`distance_multiplier`]) — the farther the shot travelled,
///   the more the whole thing is worth.
/// * **Collateral** — a Call-of-Duty-style collateral, the bullet pierced
///   clean through one bot and killed at least one more standing behind it
///   (`shared::ballistics::resolve_shot_pierce`); the distance-scaled total is
///   multiplied by the number of bots hit.
///
/// `bots_hit <= 1` and `distance <= 0.0` reduces to plain [`score_kill`] — no
/// multiplier, no extra lines.
pub fn score_multi_kill(
    spin_deg: f32,
    airborne: bool,
    noscope: bool,
    bots_hit: u32,
    distance: f32,
    max_range: f32,
) -> (u32, Vec<ScoreLine>) {
    let (base_total, mut lines) = score_kill(spin_deg, airborne, noscope);

    let mult = distance_multiplier(distance, max_range);
    let distanced_total = ((base_total as f32) * mult).round() as u32;
    if distanced_total > base_total {
        lines.push(ScoreLine {
            label: format!("LONG SHOT x{mult:.1}"),
            points: distanced_total - base_total,
        });
    }

    let bots_hit = bots_hit.max(1);
    let total = if bots_hit > 1 {
        let multiplied = distanced_total * bots_hit;
        lines.push(ScoreLine {
            label: format!("COLLATERAL x{bots_hit}"),
            points: multiplied - distanced_total,
        });
        multiplied
    } else {
        distanced_total
    };

    (total, lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_kill_is_just_the_base() {
        let (total, lines) = score_kill(20.0, false, false);
        assert_eq!(total, KILL_POINTS);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn midair_360_noscope_stacks() {
        let (total, lines) = score_kill(365.0, true, true);
        // kill + (2 * 25 * 2) + 30
        assert_eq!(total, KILL_POINTS + 100 + NOSCOPE_POINTS);
        assert_eq!(lines.len(), 3);
        assert!(lines[1].label.contains("MIDAIR 360"));
    }

    #[test]
    fn single_bot_multi_kill_matches_plain_kill() {
        let (total, lines) = score_multi_kill(20.0, false, false, 1, 0.0, 300.0);
        assert_eq!(total, KILL_POINTS);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn collateral_multiplies_and_breaks_out_the_bonus() {
        let (total, lines) = score_multi_kill(20.0, false, false, 3, 0.0, 300.0);
        // 3 bots × the base kill, with the +200 difference broken out as its
        // own COLLATERAL line.
        assert_eq!(total, KILL_POINTS * 3);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].label, "COLLATERAL x3");
        assert_eq!(lines[1].points, KILL_POINTS * 2);
    }

    #[test]
    fn collateral_multiplies_the_whole_trick_total() {
        // A no-scope kill (base + NOSCOPE_POINTS) hitting 2 bots should
        // multiply the *whole* shot's worth, not just the base kill.
        let (total, lines) = score_multi_kill(0.0, false, true, 2, 0.0, 300.0);
        let base = KILL_POINTS + NOSCOPE_POINTS;
        assert_eq!(total, base * 2);
        assert_eq!(lines.last().unwrap().points, base);
    }

    #[test]
    fn distance_multiplier_is_1x_at_muzzle_and_max_at_range() {
        assert_eq!(distance_multiplier(0.0, 300.0), 1.0);
        assert_eq!(distance_multiplier(300.0, 300.0), MAX_DISTANCE_MULT);
        // Never keeps climbing past max_range.
        assert_eq!(distance_multiplier(600.0, 300.0), MAX_DISTANCE_MULT);
        // Halfway there is halfway up the multiplier.
        let half = distance_multiplier(150.0, 300.0);
        assert!((half - (1.0 + (MAX_DISTANCE_MULT - 1.0) * 0.5)).abs() < 1.0e-5);
    }

    #[test]
    fn long_shot_bonus_is_broken_out_and_scales_the_total() {
        let (total, lines) = score_multi_kill(0.0, false, false, 1, 300.0, 300.0);
        let expected = (KILL_POINTS as f32 * MAX_DISTANCE_MULT).round() as u32;
        assert_eq!(total, expected);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].label, "LONG SHOT x3.0");
        assert_eq!(lines[1].points, expected - KILL_POINTS);
    }

    #[test]
    fn long_shot_and_collateral_stack() {
        // A max-range shot (3x) that also collaterals 2 bots (×2) should
        // compound: base -> distance-scaled -> collateral-multiplied.
        let (total, lines) = score_multi_kill(0.0, false, false, 2, 300.0, 300.0);
        let distanced = (KILL_POINTS as f32 * MAX_DISTANCE_MULT).round() as u32;
        assert_eq!(total, distanced * 2);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].label, "LONG SHOT x3.0");
        assert_eq!(lines[2].label, "COLLATERAL x2");
    }

    #[test]
    fn lines_always_sum_to_the_total() {
        // The popup shows `total` as its own headline above the itemised
        // lines — they'd better actually add up to it.
        for (spin, air, ns, bots, dist) in [
            (0.0, false, false, 1, 0.0),
            (365.0, true, true, 1, 0.0),
            (20.0, false, false, 3, 0.0),
            (0.0, false, true, 2, 0.0),
            (0.0, false, false, 1, 300.0),
            (200.0, true, true, 4, 180.0),
        ] {
            let (total, lines) = score_multi_kill(spin, air, ns, bots, dist, 300.0);
            let summed: u32 = lines.iter().map(|l| l.points).sum();
            assert_eq!(summed, total, "spin {spin} bots {bots} dist {dist}");
        }
    }

    #[test]
    fn point_blank_kill_has_no_long_shot_line() {
        let (total, lines) = score_multi_kill(0.0, false, false, 1, 0.0, 300.0);
        assert_eq!(total, KILL_POINTS);
        assert_eq!(lines.len(), 1);
    }
}
