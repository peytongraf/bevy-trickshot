//! The throwing knife's flight: a ballistic arc that bounces off walls and the
//! ground, loses energy on every bounce, comes to rest, and kills whoever it
//! hits while it's still moving fast.
//!
//! Pure math over a [`CollisionWorld`] — the server steps a [`KnifeBody`] every
//! tick (`server::knives`) and replicates the result as a
//! [`crate::ThrownKnife`]; nothing here touches the network or the ECS, so it's
//! unit-tested against a couple of trivial worlds below.

use bevy::math::{Mat3, Quat, Vec3};

use crate::ballistics::Target;
use crate::hitbox::{ray_capsule, Capsule};
use crate::map::CollisionWorld;

/// Most knives one player can have in the world at once (also how many a
/// kill-cam frame carries — see [`crate::KillCamSample::thrown_knives`]).
pub const MAX_KNIVES_PER_PLAYER: usize = 3;
/// Muzzle speed of a throw (m/s).
pub const THROW_SPEED: f32 = 40.0;
/// Downward acceleration (m/s²). Well under real gravity — a Call-of-Duty
/// throwing knife flies nearly straight for the first few dozen metres.
pub const GRAVITY: f32 = 6.0;
/// Collision radius of the knife (m): it's swept through the map as a small
/// sphere, so it can't squeeze through a crack a real knife wouldn't fit.
pub const KNIFE_RADIUS: f32 = 0.06;
/// Fraction of the speed *into* a surface kept (going back out) on a bounce.
pub const RESTITUTION: f32 = 0.5;
/// Fraction of the speed *along* a surface kept on a bounce.
pub const TANGENT_KEEP: f32 = 0.8;
/// Extra fraction of the along-surface speed kept when the surface is a floor
/// (on top of [`TANGENT_KEEP`]), so a knife skidding over the ground bleeds off
/// speed faster than one glancing off a wall.
pub const FLOOR_FRICTION: f32 = 0.65;
/// Below this speed (m/s) after a bounce the knife just stops.
pub const REST_SPEED: f32 = 1.5;
/// A knife that has bounced this many times stops regardless — a backstop for
/// a pathological wedge, the energy loss normally ends it long before.
pub const MAX_BOUNCES: u32 = 10;
/// The knife only kills while moving at least this fast (m/s) — a slow
/// tumble off a wall is harmless.
pub const LETHAL_SPEED: f32 = 4.0;
/// How long (s) a stopped knife lies there before it's removed.
pub const REST_LINGER_SECS: f32 = 1.0;
/// A knife still moving this long (s) after being thrown is removed anyway.
pub const MAX_FLIGHT_SECS: f32 = 10.0;
/// A knife that falls below this height has left the map — removed.
pub const KILL_FLOOR_Y: f32 = -100.0;
/// End-over-end tumble rate in flight (rad/s).
pub const SPIN_RATE: f32 = 22.0;
/// A tick can contain several bounces (a tight corner); more than this many
/// in one tick are dropped rather than looped on.
const MAX_SUBSTEPS: usize = 4;
/// Distance (m) a knife is nudged off the surface after a bounce, so the next
/// sweep doesn't immediately re-hit it.
const SKIN: f32 = 0.004;

/// A throwing knife's full simulation state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnifeBody {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Orientation: `-Z` is the blade tip, `Y` the flat face's normal (see
    /// [`crate::ThrownKnife`]).
    pub rot: Quat,
    /// Angular velocity, world space (rad/s).
    pub spin: Vec3,
    pub bounces: u32,
    /// Seconds since it was thrown.
    pub age: f32,
    pub resting: bool,
    /// Seconds spent resting.
    pub rest_secs: f32,
    /// Where the orientation eases to once resting on a floor (blade flat).
    rest_target: Option<Quat>,
    /// The first surface the knife struck during the most recent
    /// [`KnifeBody::step`], if any — reset at the start of every step. For
    /// the server to announce an impact sound.
    pub impact: Option<KnifeImpact>,
}

/// A knife striking a surface (not a target).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnifeImpact {
    pub point: Vec3,
    /// How fast the knife was moving *into* the surface (m/s) — a knife
    /// skidding along the ground barely touches it.
    pub speed: f32,
}

/// What a step ran into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnifeHit {
    /// The [`Target::id`] that was hit.
    pub target: u64,
    pub point: Vec3,
}

impl KnifeBody {
    /// A knife leaving `origin` along `dir` (need not be normalised) at
    /// [`THROW_SPEED`], tumbling tip-over-handle.
    pub fn thrown(origin: Vec3, dir: Vec3) -> Self {
        let dir = dir.normalize_or_zero();
        let dir = if dir == Vec3::ZERO { Vec3::NEG_Z } else { dir };
        let rot = look_rotation(dir);
        Self {
            pos: origin,
            vel: dir * THROW_SPEED,
            rot,
            // Tip drops toward the target: a negative spin about the knife's
            // own right axis.
            spin: -(rot * Vec3::X) * SPIN_RATE,
            bounces: 0,
            age: 0.0,
            resting: false,
            rest_secs: 0.0,
            rest_target: None,
            impact: None,
        }
    }

    /// Nothing more to simulate — the caller should remove the knife.
    pub fn finished(&self) -> bool {
        self.rest_secs >= REST_LINGER_SECS
            || self.age >= MAX_FLIGHT_SECS
            || self.pos.y < KILL_FLOOR_Y
    }

    /// Advance the knife `dt` seconds through `world`, testing `targets`'
    /// body capsules along the way (callers pass only who this knife is
    /// allowed to hit — never its thrower, never a mode's off-limits side).
    /// Returns the first target struck, nearest-first; the knife stops being
    /// meaningful after that, so the caller should remove it.
    pub fn step(
        &mut self,
        dt: f32,
        world: &dyn CollisionWorld,
        targets: &[Target],
    ) -> Option<KnifeHit> {
        self.age += dt;
        self.impact = None;
        if self.resting {
            self.rest_secs += dt;
            if let Some(target) = self.rest_target {
                let k = 1.0 - (-12.0 * dt).exp();
                self.rot = self.rot.slerp(target, k).normalize();
            }
            return None;
        }

        self.vel.y -= GRAVITY * dt;
        let mut remaining = dt;
        for _ in 0..MAX_SUBSTEPS {
            let travel = self.vel * remaining;
            let len = travel.length();
            if len < 1e-6 {
                break;
            }
            let dir = travel / len;
            let to = self.pos + travel;

            let world_hit = world.sweep_sphere(self.pos, to, KNIFE_RADIUS);
            let world_dist = world_hit.map_or(f32::INFINITY, |h| h.fraction * len);

            if self.vel.length() >= LETHAL_SPEED {
                let nearest = targets
                    .iter()
                    .filter_map(|t| {
                        let fat = Capsule {
                            radius: t.body.radius + KNIFE_RADIUS,
                            ..t.body
                        };
                        ray_capsule(self.pos, dir, &fat)
                            .filter(|&d| d <= len)
                            .map(|d| (d, t.id))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((d, id)) = nearest {
                    if d <= world_dist {
                        return Some(KnifeHit {
                            target: id,
                            point: self.pos + dir * d,
                        });
                    }
                }
            }

            let Some(hit) = world_hit else {
                self.pos = to;
                break;
            };

            // Move to the contact, then bounce.
            self.pos += dir * world_dist;
            let n = hit.normal;
            let vn = self.vel.dot(n);
            if self.impact.is_none() {
                self.impact = Some(KnifeImpact {
                    point: self.pos,
                    speed: (-vn).max(0.0),
                });
            }
            if vn < 0.0 {
                let v_n = n * vn;
                let mut v_t = (self.vel - v_n) * TANGENT_KEEP;
                let floor = n.y > 0.7;
                if floor {
                    v_t *= FLOOR_FRICTION;
                }
                self.vel = v_t - v_n * RESTITUTION;
            }
            self.pos += n * SKIN;
            self.bounces += 1;
            // Each bounce knocks the tumble about: slower, and a little
            // different every time.
            let wobble = hash_dir(self.bounces, self.pos);
            self.spin = self.spin * 0.55 + wobble * 4.0;

            if self.vel.length() < REST_SPEED || self.bounces >= MAX_BOUNCES {
                self.resting = true;
                self.vel = Vec3::ZERO;
                self.spin = Vec3::ZERO;
                self.rest_target = (n.y > 0.7).then(|| flat_rotation(self.rot));
                return None;
            }
            remaining *= 1.0 - hit.fraction;
        }

        self.rot = (Quat::from_scaled_axis(self.spin * dt) * self.rot).normalize();
        None
    }
}

/// The orientation with the blade tip along `dir` and the flat face
/// upward-ish — see [`crate::ThrownKnife`]'s rotation frame.
fn look_rotation(dir: Vec3) -> Quat {
    let f = dir.normalize_or_zero();
    let mut right = f.cross(Vec3::Y);
    if right.length_squared() < 1e-6 {
        right = Vec3::X; // looking straight up / down
    }
    let right = right.normalize();
    let up = right.cross(f);
    Quat::from_mat3(&Mat3::from_cols(right, up, -f))
}

/// `rot` laid flat: the tip's heading kept, projected onto the ground, with
/// the flat face up.
fn flat_rotation(rot: Quat) -> Quat {
    let fwd = rot * Vec3::NEG_Z;
    let mut h = Vec3::new(fwd.x, 0.0, fwd.z);
    if h.length_squared() < 1e-6 {
        // Pointing straight up / down: fall back to the face's heading.
        let up = rot * Vec3::Y;
        h = Vec3::new(up.x, 0.0, up.z);
    }
    if h.length_squared() < 1e-6 {
        h = Vec3::NEG_Z;
    }
    look_rotation(h.normalize())
}

/// A deterministic pseudo-random unit-ish vector from a bounce count and
/// position — no `rand` dependency, and the same on every machine.
fn hash_dir(n: u32, p: Vec3) -> Vec3 {
    let mut h = n.wrapping_mul(0x9e37_79b9)
        ^ (p.x.to_bits()).wrapping_mul(0x85eb_ca6b)
        ^ (p.y.to_bits()).wrapping_mul(0xc2b2_ae35)
        ^ (p.z.to_bits()).wrapping_mul(0x27d4_eb2f);
    let mut next = || {
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297a_2d39);
        h ^= h >> 15;
        (h as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    Vec3::new(next(), next(), next()).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{EmptyWorld, WorldHit};

    const DT: f32 = 1.0 / 64.0;

    /// A flat floor at `y = 0` and, optionally, a wall at `x = wall_x`
    /// (solid beyond it).
    struct TestWorld {
        wall_x: Option<f32>,
    }

    impl CollisionWorld for TestWorld {
        fn segment_blocked(&self, _a: Vec3, _b: Vec3) -> bool {
            false
        }

        fn raycast(&self, _origin: Vec3, _dir: Vec3, _max_dist: f32) -> Option<crate::map::RayHit> {
            None
        }

        fn sweep_sphere(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
            let mut best: Option<WorldHit> = None;
            let mut consider = |h: WorldHit| {
                if best.is_none_or(|b| h.fraction < b.fraction) {
                    best = Some(h);
                }
            };
            // Floor.
            let (a, b) = (from.y - radius, to.y - radius);
            if b < 0.0 && a > b {
                consider(WorldHit {
                    fraction: (a / (a - b)).clamp(0.0, 1.0),
                    normal: Vec3::Y,
                });
            }
            // Wall.
            if let Some(wx) = self.wall_x {
                let (a, b) = (wx - (from.x + radius), wx - (to.x + radius));
                if b < 0.0 && a > b {
                    consider(WorldHit {
                        fraction: (a / (a - b)).clamp(0.0, 1.0),
                        normal: Vec3::NEG_X,
                    });
                }
            }
            best
        }
    }

    fn run(body: &mut KnifeBody, world: &dyn CollisionWorld, secs: f32) {
        for _ in 0..(secs / DT) as usize {
            assert!(body.step(DT, world, &[]).is_none());
        }
    }

    #[test]
    fn falls_under_gravity() {
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 50.0, 0.0), Vec3::X);
        run(&mut k, &EmptyWorld, 1.0);
        // Dropped ~ ½ g t² (a touch more — semi-implicit Euler).
        let drop = 50.0 - k.pos.y;
        assert!((drop - 0.5 * GRAVITY).abs() < 0.2, "dropped {drop}");
        assert!((k.pos.x - THROW_SPEED).abs() < 0.5);
    }

    #[test]
    fn bounces_off_the_ground_losing_energy_then_stops() {
        let world = TestWorld { wall_x: None };
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, -0.3, 0.0));
        let mut peak_after_first_bounce = 0.0f32;
        for _ in 0..(8.0 / DT) as usize {
            k.step(DT, &world, &[]);
            if k.bounces >= 1 {
                peak_after_first_bounce = peak_after_first_bounce.max(k.vel.length());
            }
            if k.resting {
                break;
            }
        }
        assert!(k.resting, "knife never came to rest");
        assert!(k.bounces >= 1 && k.bounces <= MAX_BOUNCES);
        // Energy was lost: nothing after the first bounce is as fast as the throw.
        assert!(peak_after_first_bounce < THROW_SPEED * 0.9);
        // It stopped on the floor (centre one radius up), lying flat.
        assert!((k.pos.y - KNIFE_RADIUS).abs() < 0.05, "y = {}", k.pos.y);
        for _ in 0..40 {
            k.step(DT, &world, &[]);
        }
        let face = k.rot * Vec3::Y;
        assert!(face.y > 0.95, "not flat: face normal {face}");
    }

    #[test]
    fn reports_where_and_how_hard_it_struck_a_surface() {
        let world = TestWorld { wall_x: Some(5.0) };
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 5.0, 0.0), Vec3::X);
        let mut seen = None;
        for _ in 0..(1.0 / DT) as usize {
            k.step(DT, &world, &[]);
            if let Some(i) = k.impact {
                seen = Some(i);
                break;
            }
            assert!(k.impact.is_none());
        }
        let i = seen.expect("never struck the wall");
        assert!((i.point.x - (5.0 - KNIFE_RADIUS)).abs() < 0.05, "struck at {}", i.point);
        assert!(i.speed > THROW_SPEED * 0.9, "impact speed {}", i.speed);
        // Cleared again on the next step.
        k.step(DT, &world, &[]);
        assert!(k.impact.is_none());
    }

    #[test]
    fn removed_a_moment_after_stopping() {
        let world = TestWorld { wall_x: None };
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, -1.0, 0.0));
        let mut steps = 0;
        while !k.finished() {
            k.step(DT, &world, &[]);
            steps += 1;
            assert!(steps < (MAX_FLIGHT_SECS / DT) as usize + 10, "never finished");
        }
        assert!(k.resting || k.age >= MAX_FLIGHT_SECS);
    }

    #[test]
    fn bounces_back_off_a_wall() {
        let world = TestWorld { wall_x: Some(5.0) };
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 5.0, 0.0), Vec3::X);
        for _ in 0..(1.0 / DT) as usize {
            k.step(DT, &world, &[]);
            if k.bounces > 0 {
                break;
            }
        }
        assert_eq!(k.bounces, 1);
        assert!(k.vel.x < 0.0, "did not reflect: {}", k.vel);
        // Kept RESTITUTION of the way in, not more.
        assert!(k.vel.x.abs() < THROW_SPEED * RESTITUTION * 1.05);
        assert!(k.pos.x < 5.0);
    }

    fn capsule_at(x: f32, id: u64) -> Target {
        let feet = Vec3::new(x, 0.0, 0.0);
        Target {
            id,
            body: Capsule::standing(feet, 1.8, 0.4),
            head: Capsule::head(feet, 1.8, 0.14),
        }
    }

    #[test]
    fn hits_the_nearest_target_in_its_path() {
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 1.0, 0.0), Vec3::X);
        let targets = [capsule_at(20.0, 2), capsule_at(10.0, 1)];
        let mut hit = None;
        for _ in 0..(2.0 / DT) as usize {
            if let Some(h) = k.step(DT, &EmptyWorld, &targets) {
                hit = Some(h);
                break;
            }
        }
        let hit = hit.expect("knife missed");
        assert_eq!(hit.target, 1);
        assert!((hit.point.x - 9.5).abs() < 0.5, "hit at {}", hit.point);
    }

    #[test]
    fn a_wall_in_front_protects_the_target() {
        let world = TestWorld { wall_x: Some(5.0) };
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 1.0, 0.0), Vec3::X);
        let targets = [capsule_at(10.0, 1)];
        for _ in 0..(0.5 / DT) as usize {
            assert!(k.step(DT, &world, &targets).is_none());
        }
        assert!(k.bounces >= 1);
    }

    #[test]
    fn a_slow_knife_is_harmless() {
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 1.0, 0.0), Vec3::X);
        k.vel = Vec3::new(2.0, 0.0, 0.0); // under LETHAL_SPEED
        let targets = [capsule_at(1.0, 1)];
        // Just the first stretch through the target, before gravity has
        // sped it up past the lethal threshold.
        for _ in 0..(0.4 / DT) as usize {
            assert!(k.step(DT, &EmptyWorld, &targets).is_none());
        }
        assert!(k.pos.x > 0.6, "should have been inside the target's reach");
    }

    #[test]
    fn spin_tumbles_the_tip_downward_in_flight() {
        let mut k = KnifeBody::thrown(Vec3::new(0.0, 80.0, 0.0), Vec3::X);
        let tip0 = k.rot * Vec3::NEG_Z;
        for _ in 0..4 {
            k.step(DT, &EmptyWorld, &[]);
        }
        let tip1 = k.rot * Vec3::NEG_Z;
        assert!(tip1.y < tip0.y - 0.05, "tip did not drop: {tip0} -> {tip1}");
    }
}
