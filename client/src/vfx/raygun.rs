//! The Ray Gun's shot, as Call of Duty draws it: three green rings puffed
//! out of the barrel one after another — each starting small, drifting
//! forward as it swells, then fading — a bright green bolt flying from the
//! barrel, and a quick green burst where it lands — a flash that swells and fades, sparks, and a pulse of
//! green light.
//!
//! Our own bolt leaves the barrel the moment we fire ([`RayGunBolt`], from
//! `weapons::resolve_local_shot`), aimed at the surface the shot's ray meets
//! here; everyone else's when the server says it was fired
//! (`shared::RayGunFired`, `net`). Each flies at the speed the server flies
//! it (`shared::weapon::RAYGUN_BOLT_SPEED`, `server::raygun`), and the
//! server's word when it really lands steers it ([`RayGunLanded`]) — onto
//! the zombie it hit, which this client can't tell. All of it is
//! `StateScoped(InGame)`.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use lightyear::prelude::PeerId;
use shared::weapon::RAYGUN_BOLT_SPEED;

use crate::util::rand01;
use crate::AppState;

/// How long (s) a bolt waits at its guessed end for the server's word on
/// where it really landed, before bursting there anyway.
const CONFIRM_WAIT_SECS: f32 = 0.5;
/// The core sphere's radius (m) at `RayGunLook::bolt_size` 1 — its mesh's.
const CORE_RADIUS: f32 = 0.055;

/// How the Ray Gun's shot looks — the "Ray Gun" debug window's "Bolt",
/// "Burst" and "Muzzle rings" sections (`weapons::raygun_debug_ui`). A
/// bolt's own look is set as it's fired; the rest live.
#[derive(Resource, Clone)]
pub(crate) struct RayGunLook {
    /// Every part's colour.
    pub(crate) color: [f32; 3],
    /// The bolt: how bright its core, rings and sparks glow, the core's and
    /// the rings' size (×), how far apart (m) its three rings ride and how
    /// fast (rad/s) they spin, and its light (lumens; range m).
    pub(crate) bolt_glow: f32,
    pub(crate) bolt_size: f32,
    pub(crate) bolt_ring_size: f32,
    pub(crate) bolt_ring_gap: f32,
    pub(crate) bolt_ring_spin: f32,
    pub(crate) bolt_light: f32,
    pub(crate) bolt_light_range: f32,
    /// The burst: how long (s) it lasts, how big (m) it swells, how bright
    /// and opaque it starts, and its light (lumens; range m).
    pub(crate) burst_secs: f32,
    pub(crate) burst_radius: f32,
    pub(crate) burst_glow: f32,
    pub(crate) burst_opacity: f32,
    pub(crate) burst_light: f32,
    pub(crate) burst_light_range: f32,
    /// Its sparks: how many, how long (s) they last, their slowest speed
    /// (m/s — the fastest go 2.5× that), and their size (×).
    pub(crate) sparks: u32,
    pub(crate) spark_secs: f32,
    pub(crate) spark_speed: f32,
    pub(crate) spark_size: f32,
    /// The rings each shot puffs out of the barrel: how many, how far apart
    /// (s) they leave, how long (s) each lives, how fast (m/s) it starts
    /// out drifting forward, its size (×) leaving and at the end, and how
    /// opaque it starts (it fades from there).
    pub(crate) muzzle_rings: u32,
    pub(crate) muzzle_ring_gap_secs: f32,
    pub(crate) muzzle_ring_secs: f32,
    pub(crate) muzzle_ring_speed: f32,
    pub(crate) muzzle_ring_start_scale: f32,
    pub(crate) muzzle_ring_end_scale: f32,
    pub(crate) muzzle_ring_opacity: f32,
}

impl Default for RayGunLook {
    fn default() -> Self {
        Self {
            color: [0.35, 1.0, 0.3],
            bolt_glow: 6.0,
            bolt_size: 1.0,
            bolt_ring_size: 1.0,
            bolt_ring_gap: 0.22,
            bolt_ring_spin: 14.0,
            bolt_light: 60_000.0,
            bolt_light_range: 6.0,
            burst_secs: 0.3,
            burst_radius: 1.1,
            burst_glow: 5.0,
            burst_opacity: 0.09,
            burst_light: 400_000.0,
            burst_light_range: 9.0,
            sparks: 50,
            spark_secs: 0.55,
            spark_speed: 4.0,
            spark_size: 0.8,
            muzzle_rings: 3,
            muzzle_ring_gap_secs: 0.06,
            muzzle_ring_secs: 0.2,
            muzzle_ring_speed: 2.5,
            muzzle_ring_start_scale: 0.2,
            muzzle_ring_end_scale: 0.9,
            muzzle_ring_opacity: 0.1,
        }
    }
}

impl RayGunLook {
    fn color(&self) -> Color {
        Color::srgb(self.color[0], self.color[1], self.color[2])
    }
}

/// Whose bolt it is.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum BoltOwner {
    Own,
    Remote(PeerId),
}

/// Fire `owner`'s Ray Gun bolt from `start` toward `end`, where it waits on
/// the server's [`RayGunLanded`].
#[derive(Event)]
pub(crate) struct RayGunBolt {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
    pub(crate) owner: BoltOwner,
}

/// The server says where `owner`'s oldest bolt in flight really landed — or,
/// `claimed`, our own client saw ours strike a zombie there
/// (`hit_detection`), which the server's word that follows is then about.
#[derive(Event)]
pub(crate) struct RayGunLanded {
    pub(crate) end: Vec3,
    pub(crate) owner: BoltOwner,
    pub(crate) claimed: bool,
}

/// Our bolts already burst where we saw them strike (`RayGunLanded::claimed`)
/// whose word from the server hasn't come yet — skipped when it does.
#[derive(Resource, Default)]
struct ClaimedBolts(u32);

pub(crate) struct RayGunVfxPlugin;

impl Plugin for RayGunVfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<RayGunBolt>()
            .add_event::<RayGunLanded>()
            .init_resource::<ClaimedBolts>()
            .init_resource::<RayGunLook>()
            .init_resource::<ShooterVelocities>()
            .add_systems(
                OnExit(AppState::InGame),
                |mut c: ResMut<ClaimedBolts>, mut v: ResMut<ShooterVelocities>| {
                    c.0 = 0;
                    v.0.clear();
                },
            )
            .add_systems(Startup, make_assets)
            .add_systems(
                Update,
                (
                    track_shooter_velocities,
                    apply_glow,
                    spawn_bolts,
                    steer_own_bolts,
                    fly_bolts,
                    play_bursts,
                    fly_sparks,
                    puff_muzzle_rings,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Resource)]
struct RayGunAssets {
    ring: Handle<Mesh>,
    muzzle_ring: Handle<Mesh>,
    core: Handle<Mesh>,
    spark: Handle<Mesh>,
    glow: Handle<StandardMaterial>,
}

fn glowing(materials: &mut Assets<StandardMaterial>, color: Color, strength: f32, alpha: f32) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color.with_alpha(alpha),
        emissive: emissive(color, strength),
        unlit: true,
        alpha_mode: if alpha < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
        ..default()
    })
}

fn emissive(color: Color, strength: f32) -> LinearRgba {
    let c = color.to_linear();
    LinearRgba::rgb(c.red * strength, c.green * strength, c.blue * strength)
}

/// The bolts' and sparks' shared glow, kept to [`RayGunLook`].
fn apply_glow(look: Res<RayGunLook>, assets: Option<Res<RayGunAssets>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let Some(assets) = assets else { return };
    if !look.is_changed() {
        return;
    }
    if let Some(m) = materials.get_mut(&assets.glow) {
        m.base_color = look.color();
        m.emissive = emissive(look.color(), look.bolt_glow);
    }
}

fn make_assets(
    look: Res<RayGunLook>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    commands.insert_resource(RayGunAssets {
        ring: meshes.add(Torus::new(0.09, 0.13)),
        muzzle_ring: meshes.add(Torus::new(0.105, 0.13)),
        core: meshes.add(Sphere::new(CORE_RADIUS)),
        spark: meshes.add(Sphere::new(0.025)),
        glow: glowing(&mut materials, look.color(), look.bolt_glow, 1.0),
    });
}

/// A bolt in flight.
#[derive(Component)]
struct Bolt {
    end: Vec3,
    owner: BoltOwner,
    /// Still waiting on the server's word on where it landed.
    awaiting: bool,
    /// Seconds it's been waiting at its end for that.
    waited: f32,
    age: f32,
}

/// One of a bolt's rings: where along it (m, back from the front) it rides,
/// and its size (× `RayGunLook::bolt_ring_size`).
#[derive(Component)]
struct BoltRing(f32, f32);

/// A burst: its age (s), and its own fading material.
#[derive(Component)]
struct Burst {
    age: f32,
    material: Handle<StandardMaterial>,
}

/// A ring puffed out of the barrel: its age (s — negative until it
/// leaves), the way it drifts, and its own fading material.
#[derive(Component)]
struct MuzzleRing {
    age: f32,
    dir: Vec3,
    /// The shooter's velocity (m/s) as they fired, carried on top of its
    /// own drift — so a shot on the move looks the same as one standing
    /// still, rather than the shooter running through (or away from) it.
    carry: Vec3,
    material: Handle<StandardMaterial>,
}

/// How fast (m/s) everyone else is moving, from their smoothed (interpolated)
/// pose frame to frame — what a ring they puff out carries.
#[derive(Resource, Default)]
struct ShooterVelocities(bevy::platform::collections::HashMap<PeerId, (Vec3, Vec3)>);

/// How quickly (1/s) [`ShooterVelocities`] follows a change — smooths out
/// the interpolated pose's frame-to-frame jitter.
const VELOCITY_SMOOTHING: f32 = 15.0;

fn track_shooter_velocities(
    time: Res<Time>,
    poses: Query<(&shared::PlayerId, &shared::PlayerPose), With<lightyear::prelude::Interpolated>>,
    mut tracked: ResMut<ShooterVelocities>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let k = 1.0 - (-VELOCITY_SMOOTHING * dt).exp();
    let mut seen = bevy::platform::collections::HashMap::default();
    for (id, pose) in &poses {
        let (vel, prev) = match tracked.0.get(&id.0) {
            Some(&(vel, prev)) => (vel.lerp((pose.translation - prev) / dt, k), pose.translation),
            None => (Vec3::ZERO, pose.translation),
        };
        seen.insert(id.0, (vel, prev));
    }
    tracked.0 = seen;
}

#[derive(Component)]
struct Spark {
    velocity: Vec3,
    age: f32,
}

fn spawn_bolts(
    look: Res<RayGunLook>,
    me: Query<&crate::player::PlayerPhysics, With<crate::player::Player>>,
    others: Res<ShooterVelocities>,
    mut fired: EventReader<RayGunBolt>,
    assets: Option<Res<RayGunAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(assets) = assets else {
        fired.clear();
        return;
    };
    for bolt in fired.read() {
        let dir = (bolt.end - bolt.start).normalize_or(Vec3::NEG_Z);
        let carry = match bolt.owner {
            BoltOwner::Own => me
                .single()
                .map_or(Vec3::ZERO, |p| p.horizontal_velocity + Vec3::Y * p.vertical_velocity),
            BoltOwner::Remote(peer) => others.0.get(&peer).map_or(Vec3::ZERO, |&(vel, _)| vel),
        };
        for i in 0..look.muzzle_rings {
            let material = glowing(
                &mut materials,
                look.color(),
                5.0 * look.muzzle_ring_opacity,
                look.muzzle_ring_opacity,
            );
            // (A torus lies in its XZ plane: stood up to face along the
            // shot.)
            let facing = Transform::from_translation(bolt.start).looking_to(dir, Vec3::Y).rotation
                * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
            commands.spawn((
                StateScoped(AppState::InGame),
                MuzzleRing {
                    age: -(i as f32) * look.muzzle_ring_gap_secs,
                    dir,
                    carry,
                    material: material.clone(),
                },
                Mesh3d(assets.muzzle_ring.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(bolt.start)
                    .with_rotation(facing)
                    .with_scale(Vec3::splat(look.muzzle_ring_start_scale)),
                Visibility::Hidden,
                NotShadowCaster,
            ));
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                Bolt {
                    end: bolt.end,
                    owner: bolt.owner,
                    awaiting: true,
                    waited: 0.0,
                    age: 0.0,
                },
                Transform::from_translation(bolt.start).looking_to(dir, Vec3::Y),
                Visibility::default(),
            ))
            .with_children(|b| {
                b.spawn((
                    Mesh3d(assets.core.clone()),
                    MeshMaterial3d(assets.glow.clone()),
                    Transform::from_scale(Vec3::splat(look.bolt_size.max(0.01))),
                    NotShadowCaster,
                ));
                for i in 0..3 {
                    let back = i as f32 * look.bolt_ring_gap;
                    let size = 1.0 - i as f32 * 0.22;
                    b.spawn((
                        BoltRing(back, size),
                        Mesh3d(assets.ring.clone()),
                        MeshMaterial3d(assets.glow.clone()),
                        // (A torus lies in its XZ plane: stood up to face
                        // along the flight, -Z.)
                        Transform::from_translation(Vec3::Z * back)
                            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                            .with_scale(Vec3::splat((size * look.bolt_ring_size).max(0.01))),
                        NotShadowCaster,
                    ));
                }
                b.spawn(PointLight {
                    color: look.color(),
                    intensity: look.bolt_light,
                    range: look.bolt_light_range,
                    shadows_enabled: false,
                    ..default()
                });
            });
    }
}

/// The server's word on where a bolt really landed: the oldest of its
/// owner's still waiting goes there instead (or, if it's already burst, a
/// burst there now).
fn steer_own_bolts(
    look: Res<RayGunLook>,
    mut landed: EventReader<RayGunLanded>,
    mut claimed: ResMut<ClaimedBolts>,
    mut bolts: Query<&mut Bolt>,
    assets: Option<Res<RayGunAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for ev in landed.read() {
        if ev.owner == BoltOwner::Own {
            if ev.claimed {
                claimed.0 += 1;
            } else if claimed.0 > 0 {
                // (Already burst where we saw it strike.)
                claimed.0 -= 1;
                continue;
            }
        }
        let oldest = bolts
            .iter_mut()
            .filter(|b| b.awaiting && b.owner == ev.owner)
            .max_by(|a, b| a.age.total_cmp(&b.age));
        match oldest {
            Some(mut bolt) => {
                bolt.end = ev.end;
                bolt.awaiting = false;
            }
            None => {
                if let Some(assets) = assets.as_ref() {
                    burst(&mut commands, assets, &look, &mut materials, ev.end);
                }
            }
        }
    }
}

fn fly_bolts(
    look: Res<RayGunLook>,
    time: Res<Time>,
    assets: Option<Res<RayGunAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bolts: Query<(Entity, &mut Bolt, &mut Transform), Without<BoltRing>>,
    mut rings: Query<(&BoltRing, &mut Transform), Without<Bolt>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let Some(assets) = assets else { return };
    for (entity, mut bolt, mut t) in &mut bolts {
        bolt.age += dt;
        let to = bolt.end - t.translation;
        let step = RAYGUN_BOLT_SPEED * dt;
        if to.length() > step {
            let dir = to / to.length();
            t.translation += dir * step;
            t.look_to(dir, Vec3::Y);
            continue;
        }
        t.translation = bolt.end;
        if bolt.awaiting && bolt.waited < CONFIRM_WAIT_SECS {
            bolt.waited += dt;
            continue;
        }
        burst(&mut commands, &assets, &look, &mut materials, bolt.end);
        commands.entity(entity).despawn();
    }
    // The rings spin as they go.
    let spin = time.elapsed_secs() * look.bolt_ring_spin;
    for (ring, mut t) in &mut rings {
        t.rotation = Quat::from_rotation_z(spin + ring.0 * 9.0) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        t.scale = Vec3::splat((ring.1 * look.bolt_ring_size).max(0.01));
    }
}

/// A burst at `at`: the swelling flash and its light, and sparks.
fn burst(
    commands: &mut Commands,
    assets: &RayGunAssets,
    look: &RayGunLook,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
) {
    let material = glowing(materials, look.color(), look.burst_glow, look.burst_opacity);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            Burst {
                age: 0.0,
                material: material.clone(),
            },
            Mesh3d(assets.core.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(at),
            NotShadowCaster,
        ))
        .with_child(PointLight {
            color: look.color(),
            intensity: look.burst_light,
            range: look.burst_light_range,
            shadows_enabled: false,
            ..default()
        });
    let seed = (at.x * 131.0 + at.z * 71.0 + at.y * 17.0) as u32;
    for i in 0..look.sparks {
        let r = |k: u32| rand01(seed.wrapping_add(i * 7919).wrapping_mul(2_654_435_761).wrapping_add(k)) * 2.0 - 1.0;
        let speed = look.spark_speed * (1.0 + (r(4) + 1.0) * 0.75);
        let velocity = Vec3::new(r(1), r(2).abs() + 0.3, r(3)).normalize_or(Vec3::Y) * speed;
        commands.spawn((
            StateScoped(AppState::InGame),
            Spark { velocity, age: 0.0 },
            Mesh3d(assets.spark.clone()),
            MeshMaterial3d(assets.glow.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(look.spark_size.max(0.01))),
            NotShadowCaster,
        ));
    }
}

fn play_bursts(
    look: Res<RayGunLook>,
    time: Res<Time>,
    mut bursts: Query<(Entity, &mut Burst, &mut Transform, &Children)>,
    mut lights: Query<&mut PointLight>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, mut b, mut t, children) in &mut bursts {
        b.age += time.delta_secs();
        let k = (b.age / look.burst_secs.max(0.01)).clamp(0.0, 1.0);
        if k >= 1.0 {
            materials.remove(&b.material);
            commands.entity(entity).despawn();
            continue;
        }
        // Fast out, then fading.
        let grow = 1.0 - (1.0 - k).powi(3);
        let fade = 1.0 - k;
        t.scale = Vec3::splat((0.15 + grow * look.burst_radius) / CORE_RADIUS);
        if let Some(m) = materials.get_mut(&b.material) {
            m.base_color = look.color().with_alpha(look.burst_opacity * fade * fade);
            m.emissive = emissive(look.color(), look.burst_glow * fade);
        }
        for child in children.iter() {
            if let Ok(mut light) = lights.get_mut(child) {
                light.intensity = look.burst_light * fade * fade;
            }
        }
    }
}

fn fly_sparks(
    look: Res<RayGunLook>,
    time: Res<Time>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform)>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let life = look.spark_secs.max(0.01);
    for (entity, mut s, mut t) in &mut sparks {
        s.age += dt;
        if s.age >= life {
            commands.entity(entity).despawn();
            continue;
        }
        s.velocity.y -= 9.0 * dt;
        t.translation += s.velocity * dt;
        t.scale = Vec3::splat((1.0 - s.age / life) * look.spark_size.max(0.01));
    }
}

/// Each muzzle ring, once it's left: drifting forward (slowing as it
/// goes) while it swells, fading out over the back half of its life.
fn puff_muzzle_rings(
    look: Res<RayGunLook>,
    time: Res<Time>,
    mut rings: Query<(Entity, &mut MuzzleRing, &mut Transform, &mut Visibility)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut ring, mut t, mut vis) in &mut rings {
        ring.age += dt;
        // (Carried along with the shooter even before it leaves.)
        t.translation += ring.carry * dt;
        if ring.age < 0.0 {
            continue;
        }
        let k = ring.age / look.muzzle_ring_secs.max(0.01);
        if k >= 1.0 {
            materials.remove(&ring.material);
            commands.entity(entity).despawn();
            continue;
        }
        *vis = Visibility::Inherited;
        t.translation += ring.dir * look.muzzle_ring_speed * (1.0 - k).powi(2) * dt;
        let grow = 1.0 - (1.0 - k).powi(2);
        let (start, end) = (look.muzzle_ring_start_scale, look.muzzle_ring_end_scale);
        t.scale = Vec3::splat((start + grow * (end - start)).max(0.001));
        let fade = ((1.0 - k) * 2.0).min(1.0);
        if let Some(m) = materials.get_mut(&ring.material) {
            m.base_color = look.color().with_alpha(look.muzzle_ring_opacity * fade);
            m.emissive = emissive(look.color(), 5.0 * look.muzzle_ring_opacity * fade);
        }
    }
}
