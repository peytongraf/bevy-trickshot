//! `Zombies` hellhound rounds: every fifth round ([`is_dog_round`]) sends
//! only dogs — a few at first, more each dog round ([`dogs_in_round`]). Each
//! one is announced by a lightning strike where it's about to appear (for
//! [`DOG_PRE_SPAWN_SECS`]), then appears there in a flash. A dog runs fast
//! from the start but never past a sprinting player ([`dog_speed`]) and,
//! instead of swiping, blows up once it's within [`DOG_EXPLODE_RANGE`] of a player,
//! hurting everyone within [`DOG_BLAST_RADIUS`]. Killed, it blows up too
//! (harmlessly) — either way it's gone on the spot.
//!
//! Shared so the server's rounds / AI (`server::zombies`, `server::ai`) and
//! the client's effects agree. Dogs are zombies as far as everything else
//! is concerned (scoring, power-up drops, "enemies left").

use crate::zombies::{round_speed, zombie_health};

/// Every this-many rounds is a dog round.
pub const DOG_ROUND_EVERY: u32 = 5;

/// Whether `round` sends dogs (and only dogs).
pub fn is_dog_round(round: u32) -> bool {
    round > 0 && round % DOG_ROUND_EVERY == 0
}

/// How many dogs dog round `round` sends, for `players` members: 4 on round
/// 5 alone, two more every dog round after and two more per extra player.
pub fn dogs_in_round(round: u32, players: usize) -> u32 {
    let nth = (round / DOG_ROUND_EVERY).max(1);
    (2 + 2 * nth + 2 * (players.max(1) as u32 - 1)).min(40)
}

/// Seconds between one dog's lightning and the next in dog round `round`:
/// a steady stream through the round — 2.5 on the first, a little quicker
/// each dog round after, never under 1.5 — rather than the round's zombie
/// pace, which by the later rounds strikes the whole pack in within a few
/// seconds (and leaves nothing coming once it's dead).
pub fn dog_spawn_interval(round: u32) -> f32 {
    let nth = (round / DOG_ROUND_EVERY).max(1);
    (2.5 - 0.25 * (nth - 1) as f32).max(1.5)
}

/// Most hellhounds out (or struck in and on their way) at once in a dog
/// round; the rest wait their turn.
pub const DOG_MAX_AT_ONCE: usize = 12;

/// Seconds a dog's lightning strike lasts before it appears — the audible
/// length of the pre-spawn sound (`audio/zombies/dogs/pre_spawn.mp3`).
pub const DOG_PRE_SPAWN_SECS: f32 = 5.35;

/// A player's sprint speed (m/s) with no perks — the client's
/// `DEFAULT_SPRINT_SPEED`.
pub const PLAYER_SPRINT_SPEED: f32 = 9.0;
/// Slowest and fastest (m/s) a dog runs: from the very first dog round a
/// good deal quicker than a zombie runner (`ZOMBIE_RUN_SPEED`), but never faster than a
/// sprinting player — sprinting always keeps pace, and perks pull away.
pub const DOG_MIN_SPEED: f32 = 7.0;
pub const DOG_MAX_SPEED: f32 = PLAYER_SPRINT_SPEED;
/// How much faster than the round's zombies a dog is (until it tops out).
pub const DOG_SPEED_MULT: f32 = 1.725;
/// Each dog's own speed is its round's ± this fraction.
pub const DOG_SPEED_VARIANCE: f32 = 0.08;

/// A dog spawned in `round`: its speed (m/s). `roll` is a uniform `0..1`.
pub fn dog_speed(round: u32, roll: f32) -> f32 {
    let base = (round_speed(round) * DOG_SPEED_MULT).max(DOG_MIN_SPEED);
    (base * (1.0 + (roll * 2.0 - 1.0) * DOG_SPEED_VARIANCE)).clamp(DOG_MIN_SPEED, DOG_MAX_SPEED)
}

/// A dog's health in `round` — a good deal less than a zombie's.
pub fn dog_health(round: u32) -> f32 {
    zombie_health(round) * 0.6
}

/// A dog blows up once it's this close (m, horizontally, feet to feet) to a
/// player (and within [`DOG_EXPLODE_HEIGHT`] of their height)...
pub const DOG_EXPLODE_RANGE: f32 = 1.6;
pub const DOG_EXPLODE_HEIGHT: f32 = 1.5;
/// ...taking this much health (before perks) from every player within
/// [`DOG_BLAST_RADIUS`] (m) of it.
pub const DOG_BLAST_DAMAGE: f32 = 50.0;
pub const DOG_BLAST_RADIUS: f32 = 2.5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zombies::ZOMBIE_RUN_SPEED;

    #[test]
    fn every_fifth_round_is_a_dog_round() {
        assert!(!is_dog_round(0));
        assert!(!is_dog_round(4));
        assert!(is_dog_round(5));
        assert!(!is_dog_round(6));
        assert!(is_dog_round(10));
        assert!(is_dog_round(25));
    }

    #[test]
    fn dog_rounds_start_small_and_grow() {
        assert_eq!(dogs_in_round(5, 1), 4);
        assert!(dogs_in_round(10, 1) > dogs_in_round(5, 1));
        assert!(dogs_in_round(20, 1) > dogs_in_round(10, 1));
        assert!(dogs_in_round(5, 3) > dogs_in_round(5, 1));
        assert!(dogs_in_round(500, 4) <= 40);
    }

    #[test]
    fn dogs_come_in_a_steady_stream_quickening_a_little() {
        assert_eq!(dog_spawn_interval(5), 2.5);
        assert!(dog_spawn_interval(15) < dog_spawn_interval(5));
        assert_eq!(dog_spawn_interval(500), 1.5);
        // (A first dog round's whole pack is in within ~10 s.)
        assert!(dog_spawn_interval(5) * dogs_in_round(5, 1) as f32 <= 10.0);
    }

    #[test]
    fn dogs_start_fast_but_never_outrun_a_sprinting_player() {
        assert!(DOG_MIN_SPEED > ZOMBIE_RUN_SPEED);
        assert!(DOG_MAX_SPEED <= PLAYER_SPRINT_SPEED);
        for round in [5, 10, 20, 35, 50, 100] {
            for roll in [0.0, 0.5, 0.999] {
                let s = dog_speed(round, roll);
                assert!((DOG_MIN_SPEED..=DOG_MAX_SPEED).contains(&s), "round {round}: {s}");
            }
            assert!(dog_speed(round, 0.5) >= round_speed(round).min(DOG_MAX_SPEED), "round {round}");
        }
    }

    #[test]
    fn a_dog_explodes_before_it_could_swipe_and_its_blast_reaches_past_that() {
        assert!(DOG_BLAST_RADIUS > DOG_EXPLODE_RANGE);
        assert!(dog_health(5) < zombie_health(5));
    }
}
