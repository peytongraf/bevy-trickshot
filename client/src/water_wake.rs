//! The water stirred up by whatever moves through it (`water` draws it):
//! another player, a zombie, a hellhound or the boss wading through a pool
//! kicks spray up ahead of its legs and leaves rings running out behind it,
//! more the faster it goes; one standing still sends out a faint ring now
//! and then. Landing in it from a jump throws it up all round. A zombie
//! dying in it splashes where it stood, then again a moment later where its
//! body hits the water. Our own legs do the same — what we see of it kicked
//! up ahead of us looking down, and a slide through it throws a sheet of
//! it. And a blast going off in or just over a pool throws up a great
//! column of it ([`spawn_big_splash`]).
//!
//! Everything here is particles (`StateScoped(InGame)`) and ripples
//! (`WaterRipples`, cleared leaving a game), plus each avatar's own [`Wake`],
//! which goes with it, and ours ([`LocalWake`], cleared leaving a game).

use bevy::prelude::*;
use shared::{PlayerPose, ZombieAnim};

use crate::net::RemoteAvatar;
use crate::player::{Player, PlayerPhysics, Slide, Stance, EYE_HEIGHT};
use crate::util::rand01;
use crate::vfx::Explosion;
use crate::water::{spawn_big_splash, spawn_splash, surface_near, surface_over, SplashParticle, WaterAssets, WaterRipples};
use crate::{AppState, CurrentMap};

/// Metres waded between the rings a walker leaves (alternating legs).
const RING_STRIDE: f32 = 0.42;
/// Splashes kicked up per metre waded.
const SPLASHES_PER_M: f32 = 2.4;
/// Seconds between the faint rings off someone standing still in it.
const IDLE_RING_SECS: f32 = 1.7;
/// How long after a zombie drops its body hits the water.
const BODY_FALL_SECS: f32 = 0.65;
/// Moved further than this (m) in a frame: teleported, not waded.
const TELEPORT_M: f32 = 3.0;
/// How far (m) over a pool a blast still throws it up.
const BLAST_REACH: f32 = 2.5;

pub(crate) struct WaterWakePlugin;

impl Plugin for WaterWakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalWake>()
            .add_systems(OnExit(AppState::InGame), |mut wake: ResMut<LocalWake>| *wake = LocalWake::default())
            .add_systems(
                Update,
                (avatar_wakes, local_wake, blast_splashes, shot_splashes).run_if(in_state(AppState::InGame)),
            );
    }
}

/// How one wader's been stirring the water.
#[derive(Component)]
pub(crate) struct Wake {
    /// Where its feet were last frame.
    prev: Vec3,
    /// Metres waded toward the next ring, and the next splash.
    ring: f32,
    splash: f32,
    /// Seconds stood still since the last ring.
    idle: f32,
    /// Which leg the next ring comes off.
    left: bool,
    /// Last frame: alive, and in the air.
    alive: bool,
    airborne: bool,
    /// When its body hits the water, having dropped in it.
    body_falls_at: Option<f32>,
}

impl Wake {
    fn new(feet: Vec3, alive: bool, airborne: bool) -> Self {
        Self {
            prev: feet,
            ring: 0.0,
            splash: 0.0,
            idle: 0.0,
            left: false,
            alive,
            airborne,
            body_falls_at: None,
        }
    }
}

/// Our own [`Wake`] (none before we've been seen).
#[derive(Resource, Default)]
pub(crate) struct LocalWake(Option<Wake>);

/// What stirring the water needs.
struct Stir<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    assets: &'a WaterAssets,
    ripples: &'a mut WaterRipples,
    /// Splash particles out (so far this frame too).
    splashed: usize,
    now: f32,
    seq: u32,
}

impl Stir<'_, '_, '_> {
    fn roll(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq.wrapping_mul(2_654_435_761) ^ self.now.to_bits()
    }

    fn splash(&mut self, at: Vec3, power: f32, push: Vec3) {
        let seed = self.roll();
        self.splashed += spawn_splash(self.commands, self.assets, self.splashed, at, power, push, seed);
    }

    /// A burst all round `at`, `radius` across.
    fn burst(&mut self, at: Vec3, radius: f32, count: u32, power: f32) {
        for _ in 0..count {
            let s = self.roll();
            let angle = rand01(s ^ 1) * core::f32::consts::TAU;
            let r = radius * rand01(s ^ 2).sqrt();
            self.splash(at + Vec3::new(angle.cos(), 0.0, angle.sin()) * r, power, Vec3::ZERO);
        }
    }

    /// Wade `moved` (flat) this frame with feet on the surface at `at`:
    /// spray kicked up ahead of the legs, rings off them, a faint ring now
    /// and then standing still. `size` is how big the wader is (1 = a
    /// person), `kick` how hard it's going at it (a slide's more), and
    /// `ahead` how far ahead of its feet the spray goes up.
    #[allow(clippy::too_many_arguments)]
    fn wade(&mut self, wake: &mut Wake, at: Vec3, moved: Vec3, dt: f32, size: f32, kick: f32, ahead: f32) {
        let dist = moved.length();
        let speed = dist / dt;
        if speed < 0.3 {
            wake.idle += dt;
            if wake.idle >= IDLE_RING_SECS {
                wake.idle = 0.0;
                self.ripples.add(at, self.now, 0.12 * size);
            }
            return;
        }
        wake.idle = 0.0;
        let dir = moved / dist;
        let side = Vec3::new(-dir.z, 0.0, dir.x);

        wake.ring += dist;
        if wake.ring >= RING_STRIDE * size {
            wake.ring = 0.0;
            wake.left = !wake.left;
            let leg = side * 0.13 * size * if wake.left { 1.0 } else { -1.0 };
            let strength = ((0.3 + speed / 5.0) * kick).min(1.6) * size;
            self.ripples.add(at + leg + dir * 0.1 * size, self.now, strength);
        }

        wake.splash += dist * SPLASHES_PER_M * kick / size.max(0.5);
        let mut budget = 6;
        while wake.splash >= 1.0 && budget > 0 {
            wake.splash -= 1.0;
            budget -= 1;
            let s = self.roll();
            let across = (rand01(s ^ 3) * 2.0 - 1.0) * 0.22 * size;
            let from = at + side * across + dir * (ahead + rand01(s ^ 4) * 0.15) * size;
            let power = ((0.2 + speed / 7.0) * kick).min(1.3) * size.sqrt();
            self.splash(from, power, dir * speed * 0.3);
        }
        wake.splash = wake.splash.min(2.0);
    }

    /// Landing in the water from a jump.
    fn land(&mut self, at: Vec3, size: f32) {
        self.burst(at, 0.35 * size, 5, 1.0 * size.sqrt());
        self.ripples.add(at, self.now, 1.3 * size);
    }
}

/// Every other player's, zombie's, hellhound's and the boss's wake — and a
/// zombie's splash dying in the water.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn avatar_wakes(
    time: Res<Time>,
    current_map: Res<CurrentMap>,
    assets: Option<Res<WaterAssets>>,
    mut ripples: ResMut<WaterRipples>,
    splashes: Query<(), With<SplashParticle>>,
    poses: Query<&PlayerPose>,
    mut avatars: Query<(
        Entity,
        &RemoteAvatar,
        &Transform,
        Has<crate::dogs::DogVisual>,
        Has<crate::boss::BossVisual>,
        Has<crate::melee_predict::PredictedDeath>,
        Option<&mut Wake>,
    )>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let Some(assets) = assets else { return };
    let dt = time.delta_secs().max(1e-4);
    let map = current_map.0;
    let mut stir = Stir {
        splashed: splashes.iter().count(),
        commands: &mut commands,
        assets: &assets,
        ripples: &mut ripples,
        now: time.elapsed_secs_wrapped(),
        seq: *seq,
    };
    let mut fresh = Vec::new();
    for (entity, avatar, tf, dog, boss, predicted_death, wake) in &mut avatars {
        let Ok(pose) = poses.get(avatar.src) else { continue };
        let alive = crate::melee_predict::alive(pose, predicted_death);
        let feet = tf.translation;
        let Some(mut wake) = wake else {
            fresh.push((entity, Wake::new(feet, alive, pose.jumping)));
            continue;
        };
        let moved = (feet - wake.prev).with_y(0.0);
        wake.prev = feet;
        let was_alive = std::mem::replace(&mut wake.alive, alive);
        let was_airborne = std::mem::replace(&mut wake.airborne, pose.jumping);
        let size = if boss {
            2.2
        } else if dog {
            0.6
        } else {
            1.0
        };

        // Its body hitting the water: along where it falls (forward or
        // back — either way, along its facing).
        if let Some(at) = wake.body_falls_at {
            if stir.now >= at || stir.now < at - 5.0 {
                wake.body_falls_at = None;
                if let Some(surface) = surface_over(map, feet) {
                    let along = Vec3::new(-pose.yaw.sin(), 0.0, -pose.yaw.cos());
                    for i in 0..5 {
                        let k = (i as f32 / 4.0 - 0.5) * 1.5 * size;
                        let at = feet.with_y(surface) + along * k;
                        stir.burst(at, 0.25 * size, 2, 1.0 * size.sqrt());
                        if i % 2 == 0 {
                            stir.ripples.add(at, stir.now, 1.3 * size);
                        }
                    }
                }
            }
        }

        if pose.zombie == ZombieAnim::Rising {
            continue;
        }
        let Some(surface) = surface_over(map, feet) else {
            wake.ring = 0.0;
            wake.splash = 0.0;
            continue;
        };
        let at = feet.with_y(surface);
        if was_alive && !alive {
            // Dropped where it stood.
            stir.burst(at, 0.35 * size, 3, 0.8 * size.sqrt());
            stir.ripples.add(at, stir.now, 1.0 * size);
            wake.body_falls_at = Some(stir.now + BODY_FALL_SECS);
            continue;
        }
        if !alive || moved.length() > TELEPORT_M {
            continue;
        }
        if was_airborne && !pose.jumping {
            stir.land(at, size);
        }
        let kick = if pose.sliding { 2.2 } else { 1.0 };
        stir.wade(&mut wake, at, moved, dt, size, kick, 0.15);
    }
    *seq = stir.seq;
    for (entity, wake) in fresh {
        commands.entity(entity).insert(wake);
    }
}

/// Our own wake: the same, from our feet, the spray kicked up a little
/// further ahead so it's seen looking down (a slide's well ahead, as a
/// sheet of it thrown up in front).
#[allow(clippy::too_many_arguments)]
fn local_wake(
    time: Res<Time>,
    current_map: Res<CurrentMap>,
    assets: Option<Res<WaterAssets>>,
    mut ripples: ResMut<WaterRipples>,
    splashes: Query<(), With<SplashParticle>>,
    slide: Res<Slide>,
    player: Single<(&Transform, &PlayerPhysics), With<Player>>,
    mut local: ResMut<LocalWake>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    let (transform, physics) = *player;
    let feet = transform.translation - Vec3::Y * EYE_HEIGHT;
    let airborne = !physics.grounded;
    let Some(wake) = local.0.as_mut() else {
        local.0 = Some(Wake::new(feet, true, airborne));
        return;
    };
    let moved = (feet - wake.prev).with_y(0.0);
    wake.prev = feet;
    let was_airborne = std::mem::replace(&mut wake.airborne, airborne);
    let Some(surface) = surface_over(current_map.0, feet) else {
        wake.ring = 0.0;
        wake.splash = 0.0;
        return;
    };
    if moved.length() > TELEPORT_M {
        return;
    }
    let now = time.elapsed_secs_wrapped();
    let mut stir = Stir {
        splashed: splashes.iter().count(),
        commands: &mut commands,
        assets: &assets,
        ripples: &mut ripples,
        now,
        seq: now.to_bits(),
    };
    let at = feet.with_y(surface);
    if was_airborne && !airborne {
        stir.land(at, 1.0);
    }
    if airborne {
        return;
    }
    let (kick, ahead) = if slide.stance == Stance::Sliding { (2.6, 1.0) } else { (1.0, 0.5) };
    stir.wade(wake, at, moved, time.delta_secs().max(1e-4), 1.0, kick, ahead);
}

/// A shot into the water (anyone's — its tracer's line): a little splash
/// and a ring where it goes in. (Its hole's on the floor under the water.)
#[allow(clippy::too_many_arguments)]
fn shot_splashes(
    time: Res<Time>,
    current_map: Res<CurrentMap>,
    assets: Option<Res<WaterAssets>>,
    mut ripples: ResMut<WaterRipples>,
    splashes: Query<(), With<SplashParticle>>,
    mut tracers: EventReader<crate::FireTracer>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let Some(assets) = assets else {
        tracers.clear();
        return;
    };
    let now = time.elapsed_secs_wrapped();
    let mut stir = Stir {
        splashed: splashes.iter().count(),
        commands: &mut commands,
        assets: &assets,
        ripples: &mut ripples,
        now,
        seq: *seq,
    };
    for tracer in tracers.read() {
        let Some(at) = crate::water::water_crossing(current_map.0, tracer.start, tracer.end) else {
            continue;
        };
        stir.burst(at, 0.08, 2, 0.55);
        stir.ripples.add(at, now, 0.7);
    }
    *seq = stir.seq;
}

/// A blast going off in, or just over, a pool throws a column of it up.
#[allow(clippy::too_many_arguments)]
fn blast_splashes(
    time: Res<Time>,
    current_map: Res<CurrentMap>,
    assets: Option<Res<WaterAssets>>,
    mut ripples: ResMut<WaterRipples>,
    mut blasts: EventReader<Explosion>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let Some(assets) = assets else {
        blasts.clear();
        return;
    };
    for blast in blasts.read() {
        let Some(surface) = surface_near(current_map.0, blast.feet, BLAST_REACH) else {
            continue;
        };
        // Weaker the higher over the water it went off.
        let height = (blast.feet.y - surface).max(0.0);
        let fade = 1.0 - 0.6 * (height / BLAST_REACH);
        let size = if blast.dog {
            0.6
        } else if blast.boss {
            1.5
        } else {
            1.0
        } * fade;
        *seq = seq.wrapping_add(1);
        let now = time.elapsed_secs_wrapped();
        spawn_big_splash(&mut commands, &assets, &mut ripples, now, blast.feet.with_y(surface), size, *seq ^ now.to_bits());
    }
}
