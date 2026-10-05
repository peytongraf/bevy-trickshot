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
/// How long (s) a burst lasts, and how big (m) it swells.
const BURST_SECS: f32 = 0.3;
const BURST_RADIUS: f32 = 1.1;
/// Sparks each burst throws, and their life (s).
const SPARKS: usize = 10;
const SPARK_SECS: f32 = 0.35;
/// The rings each shot puffs out of the barrel: how many, how far apart
/// (s) they leave, how long (s) each lives, how fast (m/s) it starts out
/// drifting forward, and how much it swells from its starting size.
const MUZZLE_RINGS: usize = 3;
const MUZZLE_RING_GAP_SECS: f32 = 0.06;
const MUZZLE_RING_SECS: f32 = 0.55;
const MUZZLE_RING_SPEED: f32 = 5.0;
const MUZZLE_RING_START_SCALE: f32 = 0.35;
const MUZZLE_RING_END_SCALE: f32 = 2.4;
/// How opaque a muzzle ring starts (it fades from there).
const MUZZLE_RING_OPACITY: f32 = 0.5;

const GREEN: Color = Color::srgb(0.35, 1.0, 0.3);

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

/// The server says where `owner`'s oldest bolt in flight really landed.
#[derive(Event)]
pub(crate) struct RayGunLanded {
    pub(crate) end: Vec3,
    pub(crate) owner: BoltOwner,
}

pub(crate) struct RayGunVfxPlugin;

impl Plugin for RayGunVfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<RayGunBolt>()
            .add_event::<RayGunLanded>()
            .add_systems(Startup, make_assets)
            .add_systems(
                Update,
                (
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

fn glowing(materials: &mut Assets<StandardMaterial>, strength: f32, alpha: f32) -> Handle<StandardMaterial> {
    let c = GREEN.to_linear();
    materials.add(StandardMaterial {
        base_color: GREEN.with_alpha(alpha),
        emissive: LinearRgba::rgb(c.red * strength, c.green * strength, c.blue * strength),
        unlit: true,
        alpha_mode: if alpha < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
        ..default()
    })
}

fn make_assets(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    commands.insert_resource(RayGunAssets {
        ring: meshes.add(Torus::new(0.09, 0.13)),
        muzzle_ring: meshes.add(Torus::new(0.105, 0.13)),
        core: meshes.add(Sphere::new(0.055)),
        spark: meshes.add(Sphere::new(0.025)),
        glow: glowing(&mut materials, 6.0, 1.0),
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

/// One of a bolt's rings, and where along it (m, back from the front) it
/// rides.
#[derive(Component)]
struct BoltRing(f32);

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
    material: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Spark {
    velocity: Vec3,
    age: f32,
}

fn spawn_bolts(
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
        for i in 0..MUZZLE_RINGS {
            let material = glowing(&mut materials, 5.0 * MUZZLE_RING_OPACITY, MUZZLE_RING_OPACITY);
            // (A torus lies in its XZ plane: stood up to face along the
            // shot.)
            let facing = Transform::from_translation(bolt.start).looking_to(dir, Vec3::Y).rotation
                * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
            commands.spawn((
                StateScoped(AppState::InGame),
                MuzzleRing {
                    age: -(i as f32) * MUZZLE_RING_GAP_SECS,
                    dir,
                    material: material.clone(),
                },
                Mesh3d(assets.muzzle_ring.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(bolt.start)
                    .with_rotation(facing)
                    .with_scale(Vec3::splat(MUZZLE_RING_START_SCALE)),
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
                b.spawn((Mesh3d(assets.core.clone()), MeshMaterial3d(assets.glow.clone()), NotShadowCaster));
                for (i, back) in [0.0, 0.22, 0.44].into_iter().enumerate() {
                    b.spawn((
                        BoltRing(back),
                        Mesh3d(assets.ring.clone()),
                        MeshMaterial3d(assets.glow.clone()),
                        // (A torus lies in its XZ plane: stood up to face
                        // along the flight, -Z.)
                        Transform::from_translation(Vec3::Z * back)
                            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                            .with_scale(Vec3::splat(1.0 - i as f32 * 0.22)),
                        NotShadowCaster,
                    ));
                }
                b.spawn(PointLight {
                    color: GREEN,
                    intensity: 60_000.0,
                    range: 6.0,
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
    mut landed: EventReader<RayGunLanded>,
    mut bolts: Query<&mut Bolt>,
    assets: Option<Res<RayGunAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for ev in landed.read() {
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
                    burst(&mut commands, assets, &mut materials, ev.end);
                }
            }
        }
    }
}

fn fly_bolts(
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
        burst(&mut commands, &assets, &mut materials, bolt.end);
        commands.entity(entity).despawn();
    }
    // The rings spin as they go.
    let spin = time.elapsed_secs() * 14.0;
    for (ring, mut t) in &mut rings {
        t.rotation = Quat::from_rotation_z(spin + ring.0 * 9.0) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    }
}

/// A burst at `at`: the swelling flash and its light, and sparks.
fn burst(commands: &mut Commands, assets: &RayGunAssets, materials: &mut Assets<StandardMaterial>, at: Vec3) {
    let material = glowing(materials, 5.0, 0.85);
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
            color: GREEN,
            intensity: 400_000.0,
            range: 9.0,
            shadows_enabled: false,
            ..default()
        });
    let seed = (at.x * 131.0 + at.z * 71.0 + at.y * 17.0) as u32;
    for i in 0..SPARKS {
        let r = |k: u32| rand01(seed.wrapping_add(i as u32 * 7919).wrapping_mul(2_654_435_761).wrapping_add(k)) * 2.0 - 1.0;
        let velocity = Vec3::new(r(1), r(2).abs() + 0.3, r(3)).normalize_or(Vec3::Y) * (4.0 + (r(4) + 1.0) * 3.0);
        commands.spawn((
            StateScoped(AppState::InGame),
            Spark { velocity, age: 0.0 },
            Mesh3d(assets.spark.clone()),
            MeshMaterial3d(assets.glow.clone()),
            Transform::from_translation(at),
            NotShadowCaster,
        ));
    }
}

fn play_bursts(
    time: Res<Time>,
    mut bursts: Query<(Entity, &mut Burst, &mut Transform, &Children)>,
    mut lights: Query<&mut PointLight>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, mut b, mut t, children) in &mut bursts {
        b.age += time.delta_secs();
        let k = (b.age / BURST_SECS).clamp(0.0, 1.0);
        if k >= 1.0 {
            materials.remove(&b.material);
            commands.entity(entity).despawn();
            continue;
        }
        // Fast out, then fading.
        let grow = 1.0 - (1.0 - k).powi(3);
        let fade = 1.0 - k;
        t.scale = Vec3::splat((0.15 + grow * BURST_RADIUS) / 0.055);
        if let Some(m) = materials.get_mut(&b.material) {
            m.base_color = GREEN.with_alpha(0.85 * fade * fade);
            let c = GREEN.to_linear();
            m.emissive = LinearRgba::rgb(c.red, c.green, c.blue) * (5.0 * fade);
        }
        for child in children.iter() {
            if let Ok(mut light) = lights.get_mut(child) {
                light.intensity = 400_000.0 * fade * fade;
            }
        }
    }
}

fn fly_sparks(time: Res<Time>, mut sparks: Query<(Entity, &mut Spark, &mut Transform)>, mut commands: Commands) {
    let dt = time.delta_secs();
    for (entity, mut s, mut t) in &mut sparks {
        s.age += dt;
        if s.age >= SPARK_SECS {
            commands.entity(entity).despawn();
            continue;
        }
        s.velocity.y -= 9.0 * dt;
        t.translation += s.velocity * dt;
        t.scale = Vec3::splat(1.0 - s.age / SPARK_SECS);
    }
}

/// Each muzzle ring, once it's left: drifting forward (slowing as it
/// goes) while it swells, fading out over the back half of its life.
fn puff_muzzle_rings(
    time: Res<Time>,
    mut rings: Query<(Entity, &mut MuzzleRing, &mut Transform, &mut Visibility)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut ring, mut t, mut vis) in &mut rings {
        ring.age += dt;
        if ring.age < 0.0 {
            continue;
        }
        let k = ring.age / MUZZLE_RING_SECS;
        if k >= 1.0 {
            materials.remove(&ring.material);
            commands.entity(entity).despawn();
            continue;
        }
        *vis = Visibility::Inherited;
        t.translation += ring.dir * MUZZLE_RING_SPEED * (1.0 - k).powi(2) * dt;
        let grow = 1.0 - (1.0 - k).powi(2);
        t.scale = Vec3::splat(MUZZLE_RING_START_SCALE + grow * (MUZZLE_RING_END_SCALE - MUZZLE_RING_START_SCALE));
        let fade = ((1.0 - k) * 2.0).min(1.0);
        if let Some(m) = materials.get_mut(&ring.material) {
            m.base_color = GREEN.with_alpha(MUZZLE_RING_OPACITY * fade);
            let c = GREEN.to_linear();
            m.emissive = LinearRgba::rgb(c.red, c.green, c.blue) * (5.0 * MUZZLE_RING_OPACITY * fade);
        }
    }
}
