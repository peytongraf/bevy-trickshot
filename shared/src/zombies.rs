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

/// Longest pre-game countdown (s) the leader can pick
/// (`Lobby::countdown_secs`): the party roams, buys and turns the power on
/// while no zombies come, until it runs out and the first round begins.
pub const MAX_COUNTDOWN_SECS: u32 = 120;

/// The last round a plain (un-Pack-a-Punched) stab kills a zombie in one
/// (a thrown knife always does: [`ZOMBIE_THROWING_KNIFE_DAMAGE`]); Cold
/// War's knife gives out around round 13 too.
/// See [`ZOMBIE_KNIFE_DAMAGE`].
pub const ZOMBIE_KNIFE_ONE_HIT_ROUND: u32 = 13;
/// Most health a zombie ever has, reached with its top speed at
/// [`ZOMBIE_MAX_SPEED_ROUND`] — just under a fully packed (×8) sniper
/// headshot, so that still kills in one there.
pub const ZOMBIE_MAX_HEALTH: f32 = 3_000.0;

/// What a knife stab does to a zombie before Pack-a-Punch — just
/// over [`zombie_health`] at [`ZOMBIE_KNIFE_ONE_HIT_ROUND`], so it stops
/// killing in one right after. (Against players a knife always kills:
/// [`crate::melee::KNIFE_DAMAGE`].)
pub const ZOMBIE_KNIFE_DAMAGE: f32 = 231.0;
/// What a thrown knife does to a zombie: enough to kill one in any round,
/// wherever it hits.
pub const ZOMBIE_THROWING_KNIFE_DAMAGE: f32 = ZOMBIE_MAX_HEALTH;

/// How much health round `round`'s zombies spawn with: a player's
/// `health::FULL_HEALTH` in round 1, growing by the same factor every round
/// (~7%) up to [`ZOMBIE_MAX_HEALTH`] at [`ZOMBIE_MAX_SPEED_ROUND`]. It
/// doubles about every 10 rounds, as does each Pack-a-Punch level's damage,
/// so a sniper body shot (200 plain) kills in one through about round 10
/// unpacked, 20 at level I, 30 at level II and 40 at level III — and a
/// headshot (×2) ten rounds beyond each.
pub fn zombie_health(round: u32) -> f32 {
    let base = crate::health::FULL_HEALTH;
    let t = (round.max(1) - 1) as f32 / (ZOMBIE_MAX_SPEED_ROUND - 1) as f32;
    (base * (ZOMBIE_MAX_HEALTH / base).powf(t.min(1.0))).min(ZOMBIE_MAX_HEALTH)
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

/// How many zombies round `round` sends, for `players` members.
pub fn zombies_in_round(round: u32, players: usize) -> u32 {
    let round = round.max(1);
    (4 + 2 * (round - 1) + 2 * (players.max(1) as u32 - 1)).min(60)
}

/// The share of kills an estimate counts as critical (a headshot or a knife
/// kill, [`crate::ZOMBIE_CRITICAL_POINTS`] more) — a sniper game, so plenty.
const ESTIMATE_CRITICAL_SHARE: f32 = 0.4;
/// ...and on dog rounds (they're harder to headshot).
const ESTIMATE_DOG_CRITICAL_SHARE: f32 = 0.2;
/// The share of power-up drops an estimate counts as picked up.
const ESTIMATE_PICKUP_SHARE: f32 = 0.8;
/// How many of a round's kills one Double Points covers, at most (its
/// [`crate::power_ups::TIMED_SECS`] of killing)...
const ESTIMATE_DOUBLE_POINTS_KILLS: f32 = 15.0;

/// Roughly how many points a player would have banked by the start of
/// `round` with `players` in the party, had they played every round before
/// it and bought nothing — for the lobby's starting points to match a later
/// starting round. Their share of each round's kills (with a share of them
/// critical), plus what that many kills' power-ups are worth on average:
/// Bonus Points (split across the party), Nukes (everyone's), and the kills
/// Double Points doubles. Dog rounds drop nothing. Power-ups are random, so
/// this is a ballpark, rounded to 50.
pub fn estimated_points(round: u32, players: usize) -> u32 {
    use crate::power_ups::{PowerUp, BONUS_POINTS, NUKE_POINTS};
    let players = players.max(1) as f32;
    let kill = crate::ZOMBIE_KILL_POINTS as f32;
    let critical = crate::ZOMBIE_CRITICAL_POINTS as f32;
    let drops = |p: PowerUp, kills: f32| kills * p.drop_chance() * ESTIMATE_PICKUP_SHARE;
    let mut points = 0.0;
    for r in 1..round.max(1) {
        if crate::dogs::is_dog_round(r) {
            let dogs = crate::dogs::dogs_in_round(r, players as usize) as f32;
            points += dogs / players * (kill + critical * ESTIMATE_DOG_CRITICAL_SHARE);
            continue;
        }
        let team_kills = zombies_in_round(r, players as usize) as f32;
        let per_kill = kill + critical * ESTIMATE_CRITICAL_SHARE;
        points += team_kills / players * per_kill;
        points += drops(PowerUp::BonusPoints, team_kills) * BONUS_POINTS as f32 / players;
        points += drops(PowerUp::Nuke, team_kills) * NUKE_POINTS as f32;
        let doubled = (team_kills * 0.5).min(ESTIMATE_DOUBLE_POINTS_KILLS);
        points += drops(PowerUp::DoublePoints, team_kills) * doubled / players * per_kill;
    }
    let rounded = ((points / 50.0).round() * 50.0) as u32;
    rounded.min(MAX_START_POINTS)
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
    fn health_climbs_to_the_max_with_the_speed_and_one_shots_last_with_the_right_pap() {
        use crate::melee::KNIFE_DAMAGE;
        assert_eq!(zombie_health(1), crate::health::FULL_HEALTH);
        for r in 1..ZOMBIE_MAX_SPEED_ROUND {
            assert!(zombie_health(r + 1) > zombie_health(r), "round {r}");
        }
        assert_eq!(zombie_health(ZOMBIE_MAX_SPEED_ROUND), ZOMBIE_MAX_HEALTH);
        assert_eq!(zombie_health(500), ZOMBIE_MAX_HEALTH);
        // A stab kills when it takes health to 0.
        assert!(ZOMBIE_KNIFE_DAMAGE >= zombie_health(ZOMBIE_KNIFE_ONE_HIT_ROUND));
        assert!(ZOMBIE_KNIFE_DAMAGE < zombie_health(ZOMBIE_KNIFE_ONE_HIT_ROUND + 1));
        // A thrown knife always kills.
        assert!(ZOMBIE_THROWING_KNIFE_DAMAGE >= zombie_health(500));
        // Against players the knife still always kills.
        assert!(KNIFE_DAMAGE >= crate::health::FULL_HEALTH);
        // Sniper body shot (200): one-shots through ~round 10 plain, and ten
        // rounds further per Pack-a-Punch level.
        let body = 200.0;
        for (level, round) in [(0u8, 10u32), (1, 20), (2, 30), (3, 40)] {
            let dmg = body * crate::pap::damage_mult(level);
            assert!(dmg >= zombie_health(round), "level {level} round {round}");
            assert!(dmg < zombie_health(round + 2), "level {level} round {}", round + 2);
        }
        // A fully packed headshot one-shots even at the top.
        assert!(body * 2.0 * crate::pap::damage_mult(crate::pap::MAX_LEVEL) >= ZOMBIE_MAX_HEALTH);
    }

    #[test]
    fn the_swing_can_be_dodged() {
        assert!(ZOMBIE_ATTACK_HIT_SECS > 0.2 && ZOMBIE_ATTACK_HIT_SECS < ZOMBIE_ATTACK_SECS);
        assert!(ZOMBIE_STOP_DIST < ZOMBIE_ATTACK_RANGE && ZOMBIE_ATTACK_RANGE < ZOMBIE_ATTACK_REACH);
    }

    #[test]
    fn estimated_points_grow_with_the_round_and_stay_in_the_ballpark() {
        assert_eq!(estimated_points(1, 1), 0);
        assert!(estimated_points(2, 1) > 0);
        for r in 2..60 {
            assert!(estimated_points(r + 1, 1) >= estimated_points(r, 1), "round {r}");
        }
        // Solo to round 10: about 90 kills — around 11-14k.
        let ten = estimated_points(10, 1);
        assert!((10_000..16_000).contains(&ten), "{ten}");
        // A bigger party shares the kills.
        assert!(estimated_points(10, 4) < ten);
        assert!(estimated_points(500, 1) <= MAX_START_POINTS);
        assert_eq!(estimated_points(20, 1) % 50, 0);
    }
}
