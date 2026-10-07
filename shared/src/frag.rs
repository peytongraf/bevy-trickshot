//! The frag grenade (`Zombies` only), Call of Duty's: pressing the lethal key
//! pulls the pin — there's no putting it back, so the throw can't be
//! cancelled — and from then its fuse ([`FUSE_SECS`]) is running: holding the
//! key cooks it, releasing throws it. Cooked too long, it goes off in the
//! hand. Thrown, it bounces off walls and floors, rolls to a stop, and blows
//! up when its fuse runs out wherever it is — hurting every zombie in reach
//! ([`zombie_damage`]) and its thrower too ([`self_damage`]), but never
//! another player.
//!
//! Pure math over a [`CollisionWorld`], like [`crate::monkey_bomb`]: the
//! server steps a [`FragBody`] every tick (`server::frags`), clients draw it.

use bevy::math::{Quat, Vec3};

use crate::map::CollisionWorld;

/// Most frags a player can carry.
pub const MAX_FRAGS: u32 = 4;
/// Seconds from the pin coming out to the explosion.
pub const FUSE_SECS: f32 = 3.5;
/// Launch speed (m/s) along the aim, and extra upward speed so a throw at
/// the horizon still lobs.
pub const THROW_SPEED: f32 = 18.0;
pub const THROW_LIFT: f32 = 2.5;
/// Downward acceleration (m/s²).
pub const GRAVITY: f32 = 9.8;
/// Collision radius (m).
pub const RADIUS: f32 = 0.06;
/// Of its speed into a surface, how much it bounces back with...
const BOUNCE_KEEP: f32 = 0.35;
/// ...and of its speed along it, how much it keeps.
const SLIDE_KEEP: f32 = 0.7;
/// Rolling along a floor, how much speed it loses each second.
const ROLL_DRAG: f32 = 2.5;
/// A surface this upright (its normal's `y`) or more is a floor it can come
/// to rest on.
const FLOOR_NORMAL_Y: f32 = 0.6;
/// On a floor and slower than this (m/s), it stops.
const REST_SPEED: f32 = 0.6;
/// Tumble rate in flight (rad/s).
pub const SPIN_RATE: f32 = 9.0;
/// Fallen below this, it's left the map — it's removed.
pub const KILL_FLOOR_Y: f32 = -100.0;

/// Within this (m) of the blast, a zombie takes [`ZOMBIE_DAMAGE_MAX`]...
pub const ZOMBIE_KILL_RADIUS: f32 = 2.0;
/// ...falling off to [`ZOMBIE_DAMAGE_MIN`] at the edge of the blast, and
/// nothing past it.
pub const DAMAGE_RADIUS: f32 = 6.0;
pub const ZOMBIE_DAMAGE_MAX: f32 = 1_000.0;
pub const ZOMBIE_DAMAGE_MIN: f32 = 150.0;
/// The thrower's own: this much right on top of it (more than full health
/// — cooked off in the hand, it puts them down), to nothing at
/// [`SELF_DAMAGE_RADIUS`].
pub const SELF_DAMAGE_MAX: f32 = 150.0;
pub const SELF_DAMAGE_RADIUS: f32 = 5.0;

/// The chance (0–1) a zombie a player kills drops a frag — between the
/// molotov's and the monkey bomb's.
pub const DROP_CHANCE: f32 = 0.025;

/// What the blast does to a zombie whose middle is `distance` m from it.
pub fn zombie_damage(distance: f32) -> f32 {
    if distance <= ZOMBIE_KILL_RADIUS {
        ZOMBIE_DAMAGE_MAX
    } else if distance <= DAMAGE_RADIUS {
        let t = (distance - ZOMBIE_KILL_RADIUS) / (DAMAGE_RADIUS - ZOMBIE_KILL_RADIUS);
        ZOMBIE_DAMAGE_MAX + (ZOMBIE_DAMAGE_MIN - ZOMBIE_DAMAGE_MAX) * t
    } else {
        0.0
    }
}

/// What the blast does to its thrower, their middle `distance` m from it.
pub fn self_damage(distance: f32) -> f32 {
    let t = (distance / SELF_DAMAGE_RADIUS).clamp(0.0, 1.0);
    SELF_DAMAGE_MAX * (1.0 - t) * (1.0 - t)
}

/// A frag's flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FragBody {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub spin: Vec3,
    /// Stopped on a floor: it stays put until it blows.
    pub resting: bool,
}

impl FragBody {
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
            resting: false,
        }
    }

    /// One that never left the hand (cooked off): it goes off right there.
    pub fn in_hand(at: Vec3) -> Self {
        Self {
            pos: at,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            spin: Vec3::ZERO,
            resting: true,
        }
    }

    /// Fallen out of the map — the caller removes it.
    pub fn lost(&self) -> bool {
        self.pos.y < KILL_FLOOR_Y
    }

    /// Advance `dt` seconds: bounce off whatever it hits, roll along a floor
    /// and stop once it's slow enough.
    pub fn step(&mut self, dt: f32, world: &dyn CollisionWorld) {
        if self.resting {
            return;
        }
        self.vel.y -= GRAVITY * dt;
        self.rot = (Quat::from_scaled_axis(self.spin * dt) * self.rot).normalize();
        let travel = self.vel * dt;
        if travel.length_squared() < 1e-12 {
            return;
        }
        let Some(hit) = world.sweep_sphere(self.pos, self.pos + travel, RADIUS) else {
            self.pos += travel;
            return;
        };
        let n = hit.normal;
        self.pos += travel * hit.fraction + n * 0.005;
        // Off the surface: what went into it bounces back (less), what ran
        // along it slides on (less).
        let into = self.vel.dot(n);
        let along = self.vel - n * into;
        self.vel = along * SLIDE_KEEP - n * into.min(0.0) * BOUNCE_KEEP;
        self.spin *= 0.6;
        if n.y >= FLOOR_NORMAL_Y {
            self.vel *= (1.0 - ROLL_DRAG * dt).max(0.0);
            if self.vel.length() < REST_SPEED {
                self.vel = Vec3::ZERO;
                self.spin = Vec3::ZERO;
                self.resting = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{RayHit, WorldHit};

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

    fn settle(mut body: FragBody) -> FragBody {
        for _ in 0..(10 * 64) {
            body.step(1.0 / 64.0, &Room);
            if body.resting {
                return body;
            }
        }
        panic!("never came to rest: {body:?}");
    }

    #[test]
    fn it_lobs_bounces_and_rolls_to_a_stop_on_the_floor() {
        let body = settle(FragBody::thrown(Vec3::new(0.0, 1.6, 0.0), Vec3::NEG_Z));
        assert!(body.pos.y.abs() < 0.1, "{:?}", body.pos);
        assert!(body.pos.z < -5.0, "{:?}", body.pos);
    }

    #[test]
    fn it_bounces_back_off_a_wall() {
        let body = settle(FragBody::thrown(Vec3::new(3.0, 1.6, 0.0), Vec3::X));
        assert!(body.pos.x < 5.0, "{:?}", body.pos);
    }

    #[test]
    fn the_blast_falls_off_and_a_cooked_off_one_puts_its_thrower_down() {
        assert_eq!(zombie_damage(0.5), ZOMBIE_DAMAGE_MAX);
        assert!(zombie_damage(4.0) < ZOMBIE_DAMAGE_MAX && zombie_damage(4.0) > ZOMBIE_DAMAGE_MIN);
        assert_eq!(zombie_damage(DAMAGE_RADIUS + 0.1), 0.0);
        assert!(self_damage(0.0) > crate::health::FULL_HEALTH);
        assert_eq!(self_damage(SELF_DAMAGE_RADIUS), 0.0);
        assert!(DROP_CHANCE < crate::molotov::DROP_CHANCE && DROP_CHANCE > crate::monkey_bomb::DROP_CHANCE);
    }
}
