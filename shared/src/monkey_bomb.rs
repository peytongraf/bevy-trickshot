//! The monkey bomb (`Zombies` only), Call of Duty's: a wind-up cymbal
//! monkey thrown like a molotov. It lobs, bounces off walls and settles on
//! the first floor it lands on; then it claps its cymbals and sings
//! ([`SONG_SECS`]) while every zombie near enough ([`LURE_RADIUS`]) forgets
//! the players and crowds round it — until, with a "bye bye", it blows up
//! like a Bomb Shot.
//!
//! Before it's thrown it's primed: once the lethal key's released the
//! player holds on to it while its prime plays ([`PRIME_SECS`]), and
//! there's no taking it back.
//!
//! Pure math over a [`CollisionWorld`], like [`crate::molotov`]: the server
//! steps a [`MonkeyBody`] every tick (`server::monkey_bombs`), clients draw
//! it.

use bevy::math::{Quat, Vec3};

use crate::map::CollisionWorld;

/// Most monkey bombs a player can carry.
pub const MAX_MONKEYS: u32 = 3;
/// Launch speed (m/s) along the aim, and extra upward speed so a throw at
/// the horizon still lobs.
pub const THROW_SPEED: f32 = 16.0;
pub const THROW_LIFT: f32 = 3.0;
/// Downward acceleration (m/s²).
pub const GRAVITY: f32 = 9.8;
/// Collision radius (m).
pub const RADIUS: f32 = 0.12;
/// A surface this upright (its normal's `y`) or more is a floor it settles on;
/// anything steeper it bounces off.
const FLOOR_NORMAL_Y: f32 = 0.6;
/// The speed kept bouncing off a wall.
const BOUNCE_KEEP: f32 = 0.45;
/// Tumble rate in flight (rad/s).
pub const SPIN_RATE: f32 = 7.0;
/// Bounces before it just drops where it is.
const MAX_BOUNCES: u32 = 6;
/// A monkey still flying this long (s) settles where it is.
pub const MAX_FLIGHT_SECS: f32 = 8.0;
/// Fallen below this, it's left the map — it's removed.
pub const KILL_FLOOR_Y: f32 = -100.0;

/// How long (s) it's held, primed, after the lethal key's released, before
/// it's thrown (its prime sound plays on, `client::monkey_bomb`).
pub const PRIME_SECS: f32 = 3.0;
/// Once down: how long (s) until its song starts (its land sound plays at
/// once)...
pub const SONG_DELAY_SECS: f32 = 0.4;
/// ...and its sounds' lengths: the song plays on under the "bye bye
/// zombies" until it blows up.
pub const PRIME_SOUND_SECS: f32 = 4.2;
pub const LAND_SECS: f32 = 0.288;
pub const SONG_SECS: f32 = 10.632;
pub const VOX_SECS: f32 = 1.392;
/// From landing to the explosion, by default (the debug panel can change it
/// for a lobby, `SetMonkeyFuse`) — the "bye bye" starts [`VOX_SECS`] before.
pub const FUSE_SECS: f32 = 9.0;
/// The shortest and longest fuse the debug panel can set.
pub const MIN_FUSE_SECS: f32 = 1.0;
pub const MAX_FUSE_SECS: f32 = 30.0;

/// How far (m) a zombie can be from a landed monkey and still be drawn to
/// it.
pub const LURE_RADIUS: f32 = 30.0;

/// The chance (0–1) a zombie a player kills drops a monkey bomb — rarer
/// than a molotov ([`crate::molotov::DROP_CHANCE`]).
pub const DROP_CHANCE: f32 = 0.015;

/// A monkey bomb's flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonkeyBody {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub spin: Vec3,
    pub age: f32,
    pub bounces: u32,
}

impl MonkeyBody {
    /// One leaving `origin` thrown along `dir` (need not be normalised).
    pub fn thrown(origin: Vec3, dir: Vec3) -> Self {
        let dir = dir.normalize_or_zero();
        let dir = if dir == Vec3::ZERO { Vec3::NEG_Z } else { dir };
        let side = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        Self {
            pos: origin,
            vel: dir * THROW_SPEED + Vec3::Y * THROW_LIFT,
            rot: Quat::from_rotation_y(f32::atan2(-dir.x, -dir.z)),
            spin: side * SPIN_RATE,
            age: 0.0,
            bounces: 0,
        }
    }

    /// Fallen out of the map — the caller removes it.
    pub fn lost(&self) -> bool {
        self.pos.y < KILL_FLOOR_Y
    }

    /// Advance `dt` seconds: off any wall it bounces; on a floor (or flown
    /// too long, or bounced too often) it's down — the spot on the ground it
    /// stands on, and which way it faces (a turn about the vertical) — is
    /// returned.
    pub fn step(&mut self, dt: f32, world: &dyn CollisionWorld) -> Option<(Vec3, Quat)> {
        self.age += dt;
        self.vel.y -= GRAVITY * dt;
        self.rot = (Quat::from_scaled_axis(self.spin * dt) * self.rot).normalize();
        let travel = self.vel * dt;
        let len = travel.length();
        let facing = || {
            let flat = Vec3::new(self.vel.x, 0.0, self.vel.z);
            Quat::from_rotation_y(if flat.length_squared() > 1e-6 { f32::atan2(-flat.x, -flat.z) } else { 0.0 })
        };
        if len > 1e-6 {
            if let Some(hit) = world.sweep_sphere(self.pos, self.pos + travel, RADIUS) {
                let at = self.pos + travel * hit.fraction;
                if hit.normal.y >= FLOOR_NORMAL_Y || self.bounces >= MAX_BOUNCES {
                    return Some((at - Vec3::Y * RADIUS, facing()));
                }
                // Off a wall: back out along its normal, some speed lost.
                self.bounces += 1;
                let n = hit.normal;
                self.vel = (self.vel - n * 2.0 * self.vel.dot(n)) * BOUNCE_KEEP;
                self.pos = at + n * 0.01;
                return None;
            }
            self.pos += travel;
        }
        if self.age >= MAX_FLIGHT_SECS {
            // (Settles wherever it's got to.)
            return Some((self.pos - Vec3::Y * RADIUS, facing()));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{CollisionWorld, RayHit, WorldHit};

    /// A floor at y = 0, and a wall at x = 5 facing -X.
    struct Room;

    impl CollisionWorld for Room {
        fn sweep_sphere(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
            let mut best: Option<WorldHit> = None;
            let mut consider = |fraction: f32, normal: Vec3| {
                if (0.0..=1.0).contains(&fraction) && best.is_none_or(|b| fraction < b.fraction) {
                    best = Some(WorldHit { fraction, normal });
                }
            };
            if to.y - radius < 0.0 && from.y - radius >= 0.0 {
                consider((from.y - radius) / (from.y - to.y), Vec3::Y);
            }
            if to.x + radius > 5.0 && from.x + radius <= 5.0 {
                consider((5.0 - radius - from.x) / (to.x - from.x), Vec3::NEG_X);
            }
            best
        }

        fn segment_blocked(&self, _a: Vec3, _b: Vec3) -> bool {
            false
        }

        fn raycast(&self, _origin: Vec3, _dir: Vec3, _max_dist: f32) -> Option<RayHit> {
            None
        }
    }

    fn fly(mut body: MonkeyBody) -> (Vec3, MonkeyBody) {
        for _ in 0..2000 {
            if let Some((at, _)) = body.step(1.0 / 64.0, &Room) {
                return (at, body);
            }
        }
        panic!("never landed");
    }

    #[test]
    fn it_lobs_and_settles_on_the_floor() {
        let (at, body) = fly(MonkeyBody::thrown(Vec3::new(0.0, 1.6, 0.0), Vec3::NEG_Z));
        assert!(at.y.abs() < 0.05, "{at}");
        assert!(at.z < -3.0);
        assert_eq!(body.bounces, 0);
    }

    #[test]
    fn it_bounces_off_a_wall_before_settling() {
        let (at, body) = fly(MonkeyBody::thrown(Vec3::new(3.0, 1.6, 0.0), Vec3::X));
        assert_eq!(body.bounces, 1);
        assert!(at.x < 5.0 && at.y.abs() < 0.05, "{at}");
    }

    #[test]
    fn the_bye_bye_starts_after_the_song_and_the_song_lasts_till_the_end() {
        assert!(FUSE_SECS - VOX_SECS > SONG_DELAY_SECS);
        assert!(SONG_DELAY_SECS + SONG_SECS >= FUSE_SECS);
        assert!(DROP_CHANCE < crate::molotov::DROP_CHANCE);
    }
}
