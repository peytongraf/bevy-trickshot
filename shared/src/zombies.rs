//! `Zombies` zombies' body: how fast they move each round, and their melee
//! swipe — shared so the client's animation and the server's AI agree.
//! (Rounds, spawning and the AI itself are `server::zombies` / `server::ai`.)

/// Seconds a new zombie takes to climb out of the ground...
pub const ZOMBIE_RISE_SECS: f32 = 1.6;
/// ...from this far (m) under it — a whole body. (The client's ground-breaking
/// effect reads both to tell how far along the climb is.)
pub const ZOMBIE_RISE_DEPTH: f32 = 1.9;

/// Slowest a zombie ever moves (m/s) — round 1's shamblers...
pub const ZOMBIE_MIN_SPEED: f32 = 1.0;
/// ...and fastest: a sprinting player with Nitro Brew (the client's
/// `DEFAULT_SPRINT_SPEED` 9 m/s × `NitroBrew::move_mult` 1.15), reached at
/// [`ZOMBIE_MAX_SPEED_ROUND`].
pub const ZOMBIE_MAX_SPEED: f32 = 9.0 * 1.15;
pub const ZOMBIE_MAX_SPEED_ROUND: u32 = 50;
/// How the speed climbs from round 1 to [`ZOMBIE_MAX_SPEED_ROUND`] (below 1:
/// quicker early, easing off toward the top) — about 1.3 m/s in round 2,
/// 2.1 in round 5, 3.2 in round 10, 4.9 in round 20, 6.5 in round 30.
const SPEED_CURVE: f32 = 0.85;
/// Each zombie's own speed is its round's ± this fraction (then kept within
/// [`ZOMBIE_MIN_SPEED`]..=[`ZOMBIE_MAX_SPEED`]).
pub const ZOMBIE_SPEED_VARIANCE: f32 = 0.12;
/// A zombie this fast or faster runs (arms up) rather than walks — about
/// round 10, with the variance mixing walkers and runners either side.
pub const ZOMBIE_RUN_SPEED: f32 = 3.0;

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

/// The last round a plain (un-Pack-a-Punched) knife stab kills a zombie in
/// one — Cold War's knife gives out around round 13 too. Zombie health
/// ([`zombie_health`]) reaches [`crate::melee::KNIFE_DAMAGE`] here.
pub const ZOMBIE_KNIFE_ONE_HIT_ROUND: u32 = 13;
/// Most health a zombie ever has, reached with its top speed at
/// [`ZOMBIE_MAX_SPEED_ROUND`]. A fully packed knife (×8) still kills in one;
/// a plain sniper takes several shots even to the head.
pub const ZOMBIE_MAX_HEALTH: f32 = 5_000.0;

/// How much health round `round`'s zombies spawn with: a player's
/// `health::FULL_HEALTH` in round 1, growing by the same factor every round
/// up to [`crate::melee::KNIFE_DAMAGE`] at [`ZOMBIE_KNIFE_ONE_HIT_ROUND`]
/// (about 180 in round 4, 380 in round 8 — so a plain sniper's body shot
/// stops killing in one after round 4, its headshot after round 8), then
/// climbing steadily to [`ZOMBIE_MAX_HEALTH`] at [`ZOMBIE_MAX_SPEED_ROUND`].
pub fn zombie_health(round: u32) -> f32 {
    let round = round.max(1);
    let base = crate::health::FULL_HEALTH;
    let knife = crate::melee::KNIFE_DAMAGE;
    let knife_round = ZOMBIE_KNIFE_ONE_HIT_ROUND;
    if round <= knife_round {
        let t = (round - 1) as f32 / (knife_round - 1) as f32;
        // (Exactly the knife's damage at `knife_round`, so it still kills.)
        (base * (knife / base).powf(t)).min(knife)
    } else {
        let t = (round - knife_round) as f32 / (ZOMBIE_MAX_SPEED_ROUND - knife_round) as f32;
        knife + (ZOMBIE_MAX_HEALTH - knife) * t.min(1.0)
    }
}

/// Round `round`'s typical zombie speed (m/s), before each one's variance.
pub fn round_speed(round: u32) -> f32 {
    let t = (round.max(1) - 1) as f32 / (ZOMBIE_MAX_SPEED_ROUND - 1) as f32;
    ZOMBIE_MIN_SPEED + (ZOMBIE_MAX_SPEED - ZOMBIE_MIN_SPEED) * t.clamp(0.0, 1.0).powf(SPEED_CURVE)
}

/// `speed` kept within [`ZOMBIE_MIN_SPEED`]..=[`ZOMBIE_MAX_SPEED`].
pub fn clamp_speed(speed: f32) -> f32 {
    speed.clamp(ZOMBIE_MIN_SPEED, ZOMBIE_MAX_SPEED)
}

/// A zombie spawned in `round`: whether it runs, and its speed (m/s).
/// `roll` is a uniform `0..1` roll for its variance.
pub fn zombie_speed(round: u32, roll: f32) -> (bool, f32) {
    let speed = clamp_speed(round_speed(round) * (1.0 + (roll * 2.0 - 1.0) * ZOMBIE_SPEED_VARIANCE));
    (speed >= ZOMBIE_RUN_SPEED, speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_climbs_from_the_min_to_a_nitro_sprinter_at_round_50_and_stays_in_bounds() {
        assert_eq!(round_speed(1), ZOMBIE_MIN_SPEED);
        assert!((round_speed(ZOMBIE_MAX_SPEED_ROUND) - ZOMBIE_MAX_SPEED).abs() < 1e-4);
        assert_eq!(round_speed(500), ZOMBIE_MAX_SPEED);
        for r in 1..ZOMBIE_MAX_SPEED_ROUND {
            assert!(round_speed(r + 1) > round_speed(r));
        }
        for round in [1, 2, 10, 49, 50, 80] {
            for roll in [0.0, 0.3, 0.5, 0.999] {
                let (_, s) = zombie_speed(round, roll);
                assert!((ZOMBIE_MIN_SPEED..=ZOMBIE_MAX_SPEED).contains(&s), "round {round}: {s}");
            }
        }
    }

    #[test]
    fn early_rounds_walk_and_late_rounds_run() {
        for roll in [0.0, 0.5, 0.999] {
            assert!(!zombie_speed(1, roll).0);
            assert!(!zombie_speed(5, roll).0);
            assert!(zombie_speed(20, roll).0);
        }
    }

    #[test]
    fn health_climbs_to_the_max_with_the_speed_and_the_knife_gives_out_after_round_13() {
        use crate::melee::KNIFE_DAMAGE;
        assert_eq!(zombie_health(1), crate::health::FULL_HEALTH);
        for r in 1..ZOMBIE_MAX_SPEED_ROUND {
            assert!(zombie_health(r + 1) > zombie_health(r), "round {r}");
        }
        assert_eq!(zombie_health(ZOMBIE_MAX_SPEED_ROUND), ZOMBIE_MAX_HEALTH);
        assert_eq!(zombie_health(500), ZOMBIE_MAX_HEALTH);
        // A stab kills when it takes health to 0.
        assert!(KNIFE_DAMAGE >= zombie_health(ZOMBIE_KNIFE_ONE_HIT_ROUND));
        assert!(KNIFE_DAMAGE < zombie_health(ZOMBIE_KNIFE_ONE_HIT_ROUND + 1));
        // A fully packed knife still one-shots at the top; a plain sniper
        // headshot never does there.
        assert!(KNIFE_DAMAGE * crate::pap::damage_mult(crate::pap::MAX_LEVEL) >= ZOMBIE_MAX_HEALTH);
        assert!(200.0 * 2.0 * 2.0 < ZOMBIE_MAX_HEALTH);
    }

    #[test]
    fn the_swing_can_be_dodged() {
        assert!(ZOMBIE_ATTACK_HIT_SECS > 0.2 && ZOMBIE_ATTACK_HIT_SECS < ZOMBIE_ATTACK_SECS);
        assert!(ZOMBIE_STOP_DIST < ZOMBIE_ATTACK_RANGE && ZOMBIE_ATTACK_RANGE < ZOMBIE_ATTACK_REACH);
    }
}
