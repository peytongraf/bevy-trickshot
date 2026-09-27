//! `Zombies` zombies' body: how fast they move each round, and their melee
//! swipe — shared so the client's animation and the server's AI agree.
//! (Rounds, spawning and the AI itself are `server::zombies` / `server::ai`.)

/// A walker's speed (m/s) range in round 1, climbing [`WALK_SPEED_PER_ROUND`]
/// a round up to [`WALK_SPEED_MAX`].
const WALK_SPEED_MIN: f32 = 1.0;
const WALK_SPEED_SPREAD: f32 = 0.4;
const WALK_SPEED_PER_ROUND: f32 = 0.12;
const WALK_SPEED_MAX: f32 = 2.4;
/// A runner's speed (m/s) range when they first show up, climbing
/// [`RUN_SPEED_PER_ROUND`] a round up to [`RUN_SPEED_MAX`].
const RUN_SPEED_MIN: f32 = 4.2;
const RUN_SPEED_SPREAD: f32 = 0.8;
const RUN_SPEED_PER_ROUND: f32 = 0.1;
const RUN_SPEED_MAX: f32 = 6.5;
/// Round the first runners can show up in, and the round from which every
/// zombie runs (the share ramps up in between).
const FIRST_RUNNER_ROUND: u32 = 5;
const ALL_RUNNERS_ROUND: u32 = 10;

/// A zombie stops closing in this close (m, horizontally, feet to feet) to
/// who it's after, rather than walking into them...
pub const ZOMBIE_STOP_DIST: f32 = 1.0;
/// ...starts a swing at them from this close (and within
/// [`ZOMBIE_ATTACK_HEIGHT`] of their height)...
pub const ZOMBIE_ATTACK_RANGE: f32 = 1.5;
pub const ZOMBIE_ATTACK_HEIGHT: f32 = 1.5;
/// ...and the swing lands this far (s) into it — the arm takes a moment to
/// come round, so stepping out of [`ZOMBIE_ATTACK_REACH`] before then dodges
/// it. The whole swing (the client's `Attack` clip at its tuned speed) is
/// [`ZOMBIE_ATTACK_SECS`]; another can follow straight after.
pub const ZOMBIE_ATTACK_HIT_SECS: f32 = 0.5;
pub const ZOMBIE_ATTACK_SECS: f32 = 1.0;
pub const ZOMBIE_ATTACK_REACH: f32 = 2.0;
/// Health a landed swipe takes (of `health::FULL_HEALTH`, before perks).
pub const ZOMBIE_SWIPE_DAMAGE: f32 = 40.0;
/// A walker brings its arms up once it's this close (m) to who it's after.
pub const ZOMBIE_ARMS_UP_DIST: f32 = 8.0;

/// Highest starting round / starting points the party leader can pick for a
/// `Zombies` game (`Lobby::start_round` / `Lobby::start_points`) — the
/// server clamps to these.
pub const MAX_START_ROUND: u32 = 100;
pub const MAX_START_POINTS: u32 = 1_000_000;

/// The chance a zombie spawned in `round` is a runner.
pub fn runner_chance(round: u32) -> f32 {
    if round < FIRST_RUNNER_ROUND {
        return 0.0;
    }
    let span = (ALL_RUNNERS_ROUND - FIRST_RUNNER_ROUND + 1) as f32;
    ((round - FIRST_RUNNER_ROUND + 1) as f32 / span).min(1.0)
}

/// A zombie spawned in `round`: whether it runs, and its speed (m/s). `roll`
/// / `speed_roll` are uniform `0..1` rolls.
pub fn zombie_speed(round: u32, roll: f32, speed_roll: f32) -> (bool, f32) {
    let round = round.max(1);
    let runner = roll < runner_chance(round);
    let speed = if runner {
        let extra = (round - FIRST_RUNNER_ROUND.min(round)) as f32 * RUN_SPEED_PER_ROUND;
        (RUN_SPEED_MIN + RUN_SPEED_SPREAD * speed_roll + extra).min(RUN_SPEED_MAX)
    } else {
        let extra = (round - 1) as f32 * WALK_SPEED_PER_ROUND;
        (WALK_SPEED_MIN + WALK_SPEED_SPREAD * speed_roll + extra).min(WALK_SPEED_MAX)
    };
    (runner, speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_rounds_only_walk_slowly_and_late_rounds_all_run() {
        for roll in [0.0, 0.5, 0.999] {
            let (runner, speed) = zombie_speed(1, roll, roll);
            assert!(!runner);
            assert!((WALK_SPEED_MIN..=WALK_SPEED_MIN + WALK_SPEED_SPREAD).contains(&speed));
            let (runner, speed) = zombie_speed(ALL_RUNNERS_ROUND, roll, roll);
            assert!(runner);
            assert!((RUN_SPEED_MIN..=RUN_SPEED_MAX).contains(&speed));
        }
        assert_eq!(runner_chance(FIRST_RUNNER_ROUND - 1), 0.0);
        assert!(runner_chance(FIRST_RUNNER_ROUND) > 0.0 && runner_chance(FIRST_RUNNER_ROUND) < 1.0);
    }

    #[test]
    fn speeds_are_capped_however_late_the_round() {
        let (runner, run) = zombie_speed(1000, 0.0, 1.0);
        assert!(runner && run <= RUN_SPEED_MAX);
        let (runner, walk) = zombie_speed(ALL_RUNNERS_ROUND - 1, 0.999, 1.0);
        assert!(!runner && walk <= WALK_SPEED_MAX);
    }

    #[test]
    fn the_swing_can_be_dodged() {
        assert!(ZOMBIE_ATTACK_HIT_SECS > 0.2 && ZOMBIE_ATTACK_HIT_SECS < ZOMBIE_ATTACK_SECS);
        assert!(ZOMBIE_STOP_DIST < ZOMBIE_ATTACK_RANGE && ZOMBIE_ATTACK_RANGE < ZOMBIE_ATTACK_REACH);
    }
}
