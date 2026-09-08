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
}
