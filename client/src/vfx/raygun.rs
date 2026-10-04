//! The Ray Gun's shot, as Call of Duty draws it: a bright green bolt ringed
//! by spinning green rings, flying from the barrel, and a quick green burst
//! where it lands — a flash that swells and fades, sparks, and a pulse of
//! green light.
//!
//! Our own bolt leaves the barrel the moment we fire ([`RayGunBolt`], from
//! `weapons::resolve_local_shot`), aimed at the surface the shot's ray meets
//! here; the server's answer then steers it ([`RayGunLanded`]) onto where it
//! really landed — on the zombie it hit, which this client can't tell.
//! Everyone else's come off the server's answer directly (`net`). All of it
//! is `StateScoped(InGame)`.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::util::rand01;
use crate::AppState;

/// How long (s) an own bolt waits at its guessed end for the server's word
/// on where it really landed, before bursting there anyway.
const CONFIRM_WAIT_SECS: f32 = 0.35;
/// How long (s) a burst lasts, and how big (m) it swells.
const BURST_SECS: f32 = 0.3;
const BURST_RADIUS: f32 = 1.1;
/// Sparks each burst throws, and their life (s).
const SPARKS: usize = 10;
const SPARK_SECS: f32 = 0.35;

const GREEN: Color = Color::srgb(0.35, 1.0, 0.3);

/// Fire a Ray Gun bolt from `start` toward `end`. `own`: ours, waiting on
/// the server's [`RayGunLanded`].
#[derive(Event)]
pub(crate) struct RayGunBolt {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
    pub(crate) own: bool,
}

/// The server says where our oldest bolt in flight really landed.
#[derive(Event)]
pub(crate) struct RayGunLanded {
    pub(crate) end: Vec3,
}

pub(crate) struct RayGunVfxPlugin;

impl Plugin for RayGunVfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<RayGunBolt>()
            .add_event::<RayGunLanded>()
            .add_systems(Startup, make_assets)
            .add_systems(
                Update,
                (spawn_bolts, steer_own_bolts, fly_bolts, play_bursts, fly_sparks)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Resource)]
struct RayGunAssets {
    ring: Handle<Mesh>,
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
        core: meshes.add(Sphere::new(0.055)),
        spark: meshes.add(Sphere::new(0.025)),
        glow: glowing(&mut materials, 6.0, 1.0),
    });
}

/// A bolt in flight.
#[derive(Component)]
struct Bolt {
    end: Vec3,
    /// Ours, still waiting on the server's word on where it landed.
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

#[derive(Component)]
struct Spark {
    velocity: Vec3,
    age: f32,
}

fn spawn_bolts(mut fired: EventReader<RayGunBolt>, assets: Option<Res<RayGunAssets>>, mut commands: Commands) {
    let Some(assets) = assets else {
        fired.clear();
        return;
    };
    for bolt in fired.read() {
        let dir = (bolt.end - bolt.start).normalize_or(Vec3::NEG_Z);
        commands
            .spawn((
                StateScoped(AppState::InGame),
                Bolt {
                    end: bolt.end,
                    awaiting: bolt.own,
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

/// The server's word on where our bolt really landed: the oldest of ours
/// still waiting goes there instead (or, if it's already burst, a burst
/// there now).
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
            .filter(|b| b.awaiting)
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
    settings: Res<crate::RayGunSettings>,
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
        let step = settings.bolt_speed.max(1.0) * dt;
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
