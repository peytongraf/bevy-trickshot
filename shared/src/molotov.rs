//! The molotov cocktail (`Zombies` only): a lobbed bottle that shatters on
//! the first thing it touches and leaves a patch of fire on the surfaces
//! around that spot. The fire burns zombies (and the player who threw it —
//! never anyone else in the lobby) with small, quick ticks of damage.
//!
//! Pure math over a [`CollisionWorld`], like [`crate::throwing_knife`]: the
//! server steps a [`MolotovBody`] every tick (`server::molotovs`), turns the
//! shatter point into fire spots with [`fire_spots`] and checks who stands in
//! them with [`in_fire`]; clients only draw the result.

use bevy::math::{Quat, Vec3};

use crate::ballistics::Target;
use crate::hitbox::{ray_capsule, Capsule};
use crate::map::CollisionWorld;

/// Most molotovs a player can carry (the same cap as
/// [`crate::throwing_knife::MAX_CARRIED`]).
pub const MAX_MOLOTOVS: u32 = 4;
/// Launch speed (m/s) along the aim.
pub const THROW_SPEED: f32 = 19.0;
/// Extra upward launch speed (m/s), so a throw at the horizon still lobs.
pub const THROW_LIFT: f32 = 2.5;
/// Downward acceleration (m/s²) — a real arc, unlike the throwing knife.
pub const GRAVITY: f32 = 9.8;
/// Collision radius of the bottle (m).
pub const RADIUS: f32 = 0.07;
/// End-over-end tumble rate in flight (rad/s).
pub const SPIN_RATE: f32 = 9.0;
/// A bottle still flying this long (s) is removed anyway.
pub const MAX_FLIGHT_SECS: f32 = 8.0;
/// A bottle that falls below this height has left the map.
pub const KILL_FLOOR_Y: f32 = -100.0;

/// How far (m) from where the fuel lands the fire spreads.
pub const FIRE_RADIUS: f32 = 3.0;
/// How long (s) a fire burns.
pub const FIRE_SECS: f32 = 8.0;
/// Seconds between damage ticks.
pub const FIRE_TICK_SECS: f32 = 0.2;
/// Damage each tick does to a zombie standing in the fire.
pub const ZOMBIE_TICK_DAMAGE: f32 = 14.0;
/// Damage each tick does to the thrower standing in their own fire.
pub const SELF_TICK_DAMAGE: f32 = 4.0;
/// How close (m, across the ground) to a fire spot a body has to be to burn.
pub const SPOT_REACH: f32 = 0.75;
/// Feet up to this far (m) above a fire spot burn (and a little below it).
pub const BURN_HEIGHT: f32 = 1.4;
/// A fire spot more than this far (m) above or below where the fuel landed
/// is dropped — fire doesn't pour down a cliff or climb onto a roof.
const MAX_SPOT_STEP: f32 = 1.2;

/// The chance (0–1) a zombie a player kills drops a molotov.
pub const DROP_CHANCE: f32 = 0.04;
/// How long (s) a dropped molotov lies there before it's gone.
pub const DROP_LINGER_SECS: f32 = 60.0;
/// A dropped molotov lies this far (m) from the dead zombie's feet, clear of
/// the body (and of a power-up dropped at the feet).
pub const DROP_GAP: f32 = 1.0;

/// A molotov's flight state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MolotovBody {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Orientation: `Y` runs from the bottle's base to its neck.
    pub rot: Quat,
    /// Angular velocity, world space (rad/s).
    pub spin: Vec3,
    /// Seconds since it was thrown.
    pub age: f32,
}

/// Where a bottle broke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shatter {
    pub point: Vec3,
    /// Surface normal there (for a body hit, back toward where it came from).
    pub normal: Vec3,
}

impl MolotovBody {
    /// A bottle leaving `origin` thrown along `dir` (need not be normalised),
    /// neck first, tumbling end over end.
    pub fn thrown(origin: Vec3, dir: Vec3) -> Self {
        let dir = dir.normalize_or_zero();
        let dir = if dir == Vec3::ZERO { Vec3::NEG_Z } else { dir };
        let rot = Quat::from_rotation_arc(Vec3::Y, dir);
        let side = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        Self {
            pos: origin,
            vel: dir * THROW_SPEED + Vec3::Y * THROW_LIFT,
            rot,
            spin: side * SPIN_RATE,
            age: 0.0,
        }
    }

    /// Flown too long or fallen out of the map — the caller removes it.
    pub fn finished(&self) -> bool {
        self.age >= MAX_FLIGHT_SECS || self.pos.y < KILL_FLOOR_Y
    }

    /// Advance `dt` seconds. Returns where it broke if it touched the map or
    /// one of `targets`' bodies this step.
    pub fn step(&mut self, dt: f32, world: &dyn CollisionWorld, targets: &[Target]) -> Option<Shatter> {
        self.age += dt;
        self.vel.y -= GRAVITY * dt;
        let travel = self.vel * dt;
        let len = travel.length();
        self.rot = (Quat::from_scaled_axis(self.spin * dt) * self.rot).normalize();
        if len < 1e-6 {
            return None;
        }
        let dir = travel / len;
        let to = self.pos + travel;

        let world_hit = world.sweep_sphere(self.pos, to, RADIUS);
        let world_dist = world_hit.map_or(f32::INFINITY, |h| h.fraction * len);

        let body = targets
            .iter()
            .filter_map(|t| {
                let fat = Capsule {
                    radius: t.body.radius + RADIUS,
                    ..t.body
                };
                ray_capsule(self.pos, dir, &fat).filter(|&d| d <= len)
            })
            .min_by(|a, b| a.total_cmp(b));
        if let Some(d) = body.filter(|&d| d <= world_dist) {
            return Some(Shatter {
                point: self.pos + dir * d,
                normal: -dir,
            });
        }
        if let Some(hit) = world_hit {
            return Some(Shatter {
                point: self.pos + dir * world_dist,
                normal: hit.normal,
            });
        }
        self.pos = to;
        None
    }
}

/// Where the fire burns after a bottle broke at `shatter`: the fuel lands on
/// the floor there (or, off a wall or a body, on the ground under it) and
/// spreads up to [`FIRE_RADIUS`] across the surfaces around — not through
/// walls, and not down drops or up onto ledges. Each returned point sits just
/// on a surface. Never empty: at worst the fire is just where it broke.
pub fn fire_spots(shatter: Shatter, world: &dyn CollisionWorld) -> Vec<Vec3> {
    let point = shatter.point;
    let floor_hit = shatter.normal.y > 0.6;
    let base = if floor_hit {
        point - Vec3::Y * RADIUS
    } else {
        let start = point + shatter.normal * 0.3;
        match world.raycast(start, Vec3::NEG_Y, 6.0) {
            Some(h) => start - Vec3::Y * h.distance,
            None => point,
        }
    };

    let mut spots = Vec::new();
    // A splash burning on the wall / whatever it broke against.
    if !floor_hit {
        spots.push(point + shatter.normal * 0.05);
    }
    // Rings of candidate spots around where the fuel lands.
    const RINGS: [(f32, usize); 4] = [(0.0, 1), (0.9, 6), (1.9, 11), (2.85, 16)];
    let above = |v: Vec3| v + Vec3::Y * 0.6;
    for (ring, (r, count)) in RINGS.into_iter().enumerate() {
        let r = r / 2.85 * FIRE_RADIUS;
        for i in 0..count {
            let a = (i as f32 + 0.5 * ring as f32) / count as f32 * std::f32::consts::TAU;
            let offset = Vec3::new(a.cos(), 0.0, a.sin()) * r;
            if r > 0.0 && world.segment_blocked(above(base), above(base + offset)) {
                continue;
            }
            let probe = base + offset + Vec3::Y * 1.0;
            let Some(hit) = world.raycast(probe, Vec3::NEG_Y, 1.0 + MAX_SPOT_STEP) else {
                continue;
            };
            if hit.normal.y < 0.5 {
                continue;
            }
            let ground = probe - Vec3::Y * hit.distance;
            if (ground.y - base.y).abs() > MAX_SPOT_STEP {
                continue;
            }
            spots.push(ground + Vec3::Y * 0.02);
        }
    }
    if spots.is_empty() {
        spots.push(base);
    }
    spots
}

/// Whether a body standing with its feet at `feet` is in a fire burning at
/// `spots`.
pub fn in_fire(feet: Vec3, spots: &[Vec3]) -> bool {
    spots.iter().any(|s| {
        let dy = feet.y - s.y;
        let flat = Vec3::new(feet.x - s.x, 0.0, feet.z - s.z).length();
        flat <= SPOT_REACH && (-0.5..=BURN_HEIGHT).contains(&dy)
    })
}

/// Where a molotov a zombie dropped lies: [`DROP_GAP`] off to one side of
/// its `feet` (the side picked by `seed`, trying others if one's behind a
/// wall or over a drop), on the ground. At worst, at the feet.
pub fn drop_spot(feet: Vec3, seed: f32, world: &dyn CollisionWorld) -> Vec3 {
    let a0 = seed * std::f32::consts::TAU;
    let chest = feet + Vec3::Y;
    (0..6)
        .find_map(|i| {
            let a = a0 + i as f32 * std::f32::consts::TAU / 6.0;
            let d = Vec3::new(a.cos(), 0.0, a.sin());
            let above = feet + d * DROP_GAP + Vec3::Y;
            if world.segment_blocked(chest, above) {
                return None;
            }
            let ground = world.raycast(above, Vec3::NEG_Y, 3.0)?;
            Some(above - Vec3::Y * ground.distance)
        })
        .unwrap_or(feet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{RayHit, WorldHit};

    /// A floor at `y = 0`, and optionally a wall filling `x >= wall`.
    struct World {
        wall: Option<f32>,
    }

    impl CollisionWorld for World {
        fn segment_blocked(&self, a: Vec3, b: Vec3) -> bool {
            if a.y < 0.0 || b.y < 0.0 {
                return true;
            }
            self.wall.is_some_and(|w| a.x.max(b.x) >= w)
        }

        fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
            if self.wall.is_some_and(|w| origin.x >= w) {
                return None;
            }
            if dir.y < 0.0 && origin.y >= 0.0 {
                let d = origin.y / -dir.y;
                if d <= max_dist {
                    return Some(RayHit {
                        distance: d,
                        normal: Vec3::Y,
                    });
                }
            }
            None
        }

        fn sweep_sphere(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
            let mut best: Option<WorldHit> = None;
            if to.y - radius < 0.0 && from.y - radius >= 0.0 {
                let f = (from.y - radius) / (from.y - to.y);
                best = Some(WorldHit {
                    fraction: f,
                    normal: Vec3::Y,
                });
            }
            if let Some(w) = self.wall {
                if to.x + radius > w && from.x + radius <= w {
                    let f = (w - radius - from.x) / (to.x - from.x);
                    if best.is_none_or(|b| f < b.fraction) {
                        best = Some(WorldHit {
                            fraction: f,
                            normal: Vec3::NEG_X,
                        });
                    }
                }
            }
            best
        }
    }

    fn fly(body: &mut MolotovBody, world: &World) -> Option<Shatter> {
        for _ in 0..2000 {
            if let Some(s) = body.step(1.0 / 64.0, world, &[]) {
                return Some(s);
            }
        }
        None
    }

    #[test]
    fn a_throw_arcs_down_and_breaks_on_the_floor() {
        let world = World { wall: None };
        let mut body = MolotovBody::thrown(Vec3::new(0.0, 1.7, 0.0), Vec3::NEG_Z);
        let s = fly(&mut body, &world).expect("it lands");
        assert!(s.point.z < -5.0, "flew forward: {s:?}");
        assert!(s.point.y.abs() < 0.2, "on the floor: {s:?}");
        assert_eq!(s.normal, Vec3::Y);
    }

    #[test]
    fn a_throw_into_a_wall_burns_on_the_ground_below_it() {
        let world = World { wall: Some(4.0) };
        let mut body = MolotovBody::thrown(Vec3::new(0.0, 1.7, 0.0), Vec3::X);
        let s = fly(&mut body, &world).expect("it hits the wall");
        assert_eq!(s.normal, Vec3::NEG_X);
        let spots = fire_spots(s, &world);
        assert!(spots.iter().all(|p| p.x < 4.0), "nothing through the wall");
        assert!(spots.iter().filter(|p| p.y < 0.1).count() > 5, "a fire on the ground");
    }

    #[test]
    fn the_fire_spreads_round_where_it_landed_and_burns_who_stands_in_it() {
        let world = World { wall: None };
        let at = Vec3::new(3.0, 0.0, -2.0);
        let spots = fire_spots(
            Shatter {
                point: at + Vec3::Y * RADIUS,
                normal: Vec3::Y,
            },
            &world,
        );
        assert!(spots.len() > 20);
        assert!(spots.iter().all(|p| p.distance(at) <= FIRE_RADIUS + 0.05));
        assert!(in_fire(at, &spots));
        assert!(in_fire(at + Vec3::new(2.0, 0.0, 0.0), &spots));
        assert!(!in_fire(at + Vec3::new(FIRE_RADIUS + 1.5, 0.0, 0.0), &spots));
        assert!(!in_fire(at + Vec3::Y * 3.0, &spots), "not from well above");
    }

    #[test]
    fn a_bottle_breaks_on_a_body_in_its_way() {
        let world = World { wall: None };
        let feet = Vec3::new(0.0, 0.0, -3.0);
        let target = Target {
            id: 1,
            body: Capsule::standing(feet, 1.8, 0.4),
            head: Capsule::head(feet, 1.8, 0.12),
        };
        let mut body = MolotovBody::thrown(Vec3::new(0.0, 1.2, 0.0), Vec3::NEG_Z);
        let mut shatter = None;
        for _ in 0..200 {
            if let Some(s) = body.step(1.0 / 64.0, &world, std::slice::from_ref(&target)) {
                shatter = Some(s);
                break;
            }
        }
        let s = shatter.expect("it hits the zombie");
        assert!((s.point.z - (-3.0 + 0.4 + RADIUS)).abs() < 0.3, "{s:?}");
    }

    #[test]
    fn a_dropped_molotov_lies_clear_of_the_body_on_the_ground() {
        let world = World { wall: None };
        let feet = Vec3::new(1.0, 0.0, 1.0);
        let spot = drop_spot(feet, 0.3, &world);
        assert!((spot.distance(feet) - DROP_GAP).abs() < 0.01);
        assert!(spot.y.abs() < 0.01);
    }
}
