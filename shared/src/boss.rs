//! `Zombies` bosses: a hulking brute (`models/characters/boss.glb`) that
//! joins some rounds ([`is_boss_round`]) on top of their zombies. Each is
//! announced by an orange lightning strike where it's about to appear (for
//! [`BOSS_PRE_SPAWN_SECS`] — the spawn sound's length), then walks at whoever
//! it's after with two attacks:
//!
//! * **Smash** — close in ([`MELEE_RANGE`]), it rears its arms up and back
//!   and slams them forward ([`ATTACK_SECS`] long); the hit lands only as the
//!   arms come all the way forward ([`MELEE_HIT_SECS`] in), on whoever's
//!   still within [`MELEE_REACH`], so it can be dodged.
//! * **Blast** — farther off ([`BLAST_MIN_RANGE`]..[`BLAST_MAX_RANGE`], and
//!   in sight), the same wind-up builds a ball of energy overhead that it
//!   hurls at them ([`BLAST_RELEASE_SECS`] in), every [`BLAST_COOLDOWN_SECS`]
//!   or so. The ball flies straight at [`BLAST_SPEED`] — quick, but a player
//!   on their toes can sidestep it — and blows up on the first thing it
//!   touches (a player, or the map), hurting everyone within
//!   [`BLAST_RADIUS`]: [`BLAST_DAMAGE`] at its heart, less the farther out,
//!   nothing past the edge ([`blast_damage`]).
//!
//! Shared so the server (`server::boss`, `server::ai`) and the client
//! (`client::boss`) agree on timing and size. A boss is a zombie to
//! everything else (scoring, "enemies left"), just a much tougher one
//! ([`boss_health`]) — though Insta-Kill and the Nuke don't touch it.

use bevy::math::Vec3;

use crate::hitbox::Capsule;
use crate::zombies::zombie_health;
use crate::ZombieAnim;

/// The first boss round, and every this-many rounds after ([`is_boss_round`]).
pub const BOSS_ROUND_EVERY: u32 = 7;

/// Whether `round` brings a boss along with its zombies: every seventh
/// round — never a dog round (those are hellhounds only).
pub fn is_boss_round(round: u32) -> bool {
    round > 0 && round % BOSS_ROUND_EVERY == 0 && !crate::dogs::is_dog_round(round)
}

/// How many bosses boss round `round` brings for `players` members: one, a
/// second with three or more players, and one more every third boss round.
pub fn bosses_in_round(round: u32, players: usize) -> u32 {
    let nth = round / BOSS_ROUND_EVERY;
    (1 + (players.max(1) as u32 - 1) / 2 + nth.saturating_sub(1) / 3).min(4)
}

/// Seconds into a boss round before its boss is struck in — the round's
/// first zombies get a head start.
pub const BOSS_ROUND_DELAY_SECS: f32 = 12.0;

/// Seconds its lightning strikes before it appears — the audible length of
/// its spawn sound (`audio/zombies/boss/spawn.mp3`).
pub const BOSS_PRE_SPAWN_SECS: f32 = 5.6;

/// A boss's health in `round`: as much as a crowd of that round's zombies,
/// more for every extra player.
pub fn boss_health(round: u32, players: usize) -> f32 {
    zombie_health(round) * (30.0 + 10.0 * (players.max(1) - 1) as f32)
}

/// Walking speed (m/s) — a slow, heavy stride.
pub const BOSS_SPEED: f32 = 1.7;

/// How tall (m) it stands, and its body's hit capsule radius / head radius.
pub const BOSS_HEIGHT: f32 = 2.7;
pub const BOSS_RADIUS: f32 = 0.6;
pub const BOSS_HEAD_RADIUS: f32 = 0.3;

/// The body and head a shot, stab or bolt has to hit on someone whose pose
/// is `anim`, standing at `feet` — a boss's, or a person-sized one
/// (`height`, `radius`, `head_radius`) for everyone else.
pub fn hit_capsules(feet: Vec3, anim: ZombieAnim, height: f32, radius: f32, head_radius: f32) -> (Capsule, Capsule) {
    if anim.is_boss() {
        (
            Capsule::standing(feet, BOSS_HEIGHT, BOSS_RADIUS),
            Capsule::head(feet, BOSS_HEIGHT, BOSS_HEAD_RADIUS),
        )
    } else {
        (Capsule::standing(feet, height, radius), Capsule::head(feet, height, head_radius))
    }
}

/// The attack clip's length (s): rear up, arms back, slam forward, recover.
pub const ATTACK_SECS: f32 = 2.267;
/// When (s into it) the arms are all the way forward — the smash lands...
pub const MELEE_HIT_SECS: f32 = 1.2;
/// ...and when the ball leaves its hands.
pub const BLAST_RELEASE_SECS: f32 = 1.15;
/// When (s into it) the ball starts forming overhead, as the arms go up.
pub const BLAST_CHARGE_START_SECS: f32 = 0.55;

/// It starts a smash at someone this close (m, feet to feet, horizontally;
/// within [`MELEE_HEIGHT`] of their height)...
pub const MELEE_RANGE: f32 = 2.4;
pub const MELEE_HEIGHT: f32 = 2.0;
/// ...stops walking in this close...
pub const STOP_DIST: f32 = 1.8;
/// ...and it lands on them if they're still within this when the arms come
/// forward, taking this much health (before perks).
pub const MELEE_REACH: f32 = 3.0;
pub const MELEE_DAMAGE: f32 = 70.0;

/// It hurls a blast at someone it can see between these distances (m)...
pub const BLAST_MIN_RANGE: f32 = 7.0;
pub const BLAST_MAX_RANGE: f32 = 45.0;
/// ...at most this often (s, plus up to [`BLAST_COOLDOWN_JITTER`] more)...
pub const BLAST_COOLDOWN_SECS: f32 = 6.0;
pub const BLAST_COOLDOWN_JITTER: f32 = 3.0;
/// ...and not in its first moments after appearing.
pub const BLAST_FIRST_DELAY_SECS: f32 = 3.0;
/// The ball's speed (m/s) and how long it flies before blowing up anyway.
pub const BLAST_SPEED: f32 = 17.0;
pub const BLAST_LIFETIME_SECS: f32 = 4.0;
/// The ball's size for hitting things (m): a player whose body comes within
/// this of its centre, or a wall it touches, sets it off.
pub const BLAST_HIT_RADIUS: f32 = 0.45;
/// Its explosion: this much health at its heart, falling off to nothing at
/// [`BLAST_RADIUS`] (m).
pub const BLAST_DAMAGE: f32 = 80.0;
pub const BLAST_RADIUS: f32 = 5.0;

/// Where the ball leaves its hands, from its feet facing `forward` (unit,
/// horizontal): a little in front of it, just over head height.
pub fn blast_origin(feet: Vec3, forward: Vec3) -> Vec3 {
    feet + forward * 1.0 + Vec3::Y * (BOSS_HEIGHT * 0.8)
}

/// What the blast takes off someone `distance` (m) from where it went off:
/// [`BLAST_DAMAGE`] at its heart, falling off straight to nothing at
/// [`BLAST_RADIUS`] — and nothing at all past it.
pub fn blast_damage(distance: f32) -> f32 {
    if distance >= BLAST_RADIUS {
        0.0
    } else {
        BLAST_DAMAGE * (1.0 - distance.max(0.0) / BLAST_RADIUS)
    }
}

/// Closest distance between the point `p` and the segment `a`..`b`.
pub fn segment_point_distance(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 1e-12 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bosses_come_every_seventh_round_but_never_on_a_dog_round() {
        assert!(!is_boss_round(0));
        assert!(!is_boss_round(6));
        assert!(is_boss_round(7));
        assert!(is_boss_round(14));
        assert!(!is_boss_round(35), "35 is a dog round");
        assert_eq!(bosses_in_round(7, 1), 1);
        assert!(bosses_in_round(7, 4) > 1);
        assert!(bosses_in_round(500, 8) <= 4);
    }

    #[test]
    fn the_blast_falls_off_to_nothing_at_its_edge() {
        assert_eq!(blast_damage(0.0), BLAST_DAMAGE);
        assert!(blast_damage(1.0) > blast_damage(3.0));
        assert!(blast_damage(BLAST_RADIUS - 0.1) > 0.0);
        assert_eq!(blast_damage(BLAST_RADIUS), 0.0);
        assert_eq!(blast_damage(BLAST_RADIUS + 10.0), 0.0);
    }

    #[test]
    fn the_smash_lands_as_the_arms_come_forward_and_reaches_past_where_it_starts() {
        assert!(MELEE_HIT_SECS > BLAST_CHARGE_START_SECS && MELEE_HIT_SECS < ATTACK_SECS);
        assert!(BLAST_RELEASE_SECS < ATTACK_SECS);
        assert!(MELEE_REACH > MELEE_RANGE && MELEE_RANGE > STOP_DIST);
        assert!(BLAST_MIN_RANGE > MELEE_REACH);
    }

    #[test]
    fn a_boss_is_much_bigger_to_hit_than_a_person() {
        let (body, head) = hit_capsules(Vec3::ZERO, ZombieAnim::BossIdle, 1.8, 0.35, 0.12);
        assert!(body.radius > 0.35 && head.a.y > 1.8);
        let (body, _) = hit_capsules(Vec3::ZERO, ZombieAnim::Walk, 1.8, 0.35, 0.12);
        assert_eq!(body.radius, 0.35);
    }

    #[test]
    fn a_segment_passing_by_a_point_is_as_close_as_its_nearest_spot() {
        let d = segment_point_distance(Vec3::ZERO, Vec3::X * 10.0, Vec3::new(5.0, 1.0, 0.0));
        assert!((d - 1.0).abs() < 1e-5);
        let d = segment_point_distance(Vec3::ZERO, Vec3::X, Vec3::new(3.0, 0.0, 0.0));
        assert!((d - 2.0).abs() < 1e-5);
    }
}
