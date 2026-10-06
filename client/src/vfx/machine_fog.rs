//! The cold fog perk machines vent once the power's on, the way Call of Duty:
//! Cold War's do: puffs leave the dispenser slot on the front and both sides
//! (all about 1 m up), get pushed out, sink,
//! and roll out along the floor around the machine, growing and thinning as
//! they go. It never stops while the power's on.
//!
//! Each puff is a camera-facing smoke sprite (`textures/vfx/smoke.png`, the
//! barrel smoke's) in world space, lit — so the machine's own light tints
//! the fog in its perk's colour — with a faint glow of its own so it never
//! goes black in the dark. Tuned in the debug panel ("Zombies perks" →
//! "Perk machines" → "Fog"), via [`crate::zombies_hud::PerkMachineSettings`]
//! — and the Pack-a-Punch's, which vents the same way, separately
//! ("Pack-a-Punch machine" → "Fog", [`crate::pap::PapSettings`]).
//!
//! Puffs are `StateScoped(InGame)` and cleared the moment we're not in a
//! `Zombies` game, so nothing carries into the next one.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use lightyear::prelude::LocalId;

use crate::net::GameClient;
use crate::util::{rand01, rand_roll};
use crate::zombies_hud::{zombies_game, PerkMachineSettings};
use crate::vfx::{smoke_billboard_rotation, SmokeAssets};
use crate::{AppState, WorldModelCamera};

/// Most puffs alive at once, across every machine.
const MAX_PUFFS: usize = 700;
/// Most puffs one vent can owe at once (a long frame doesn't dump a pile).
const MAX_BACKLOG: f32 = 3.0;

/// The fog's look — every perk machine's (and, its own, the Pack-a-Punch's).
#[derive(Clone, Copy)]
pub(crate) struct MachineFog {
    pub(crate) enabled: bool,
    /// Puffs a second from each vent.
    pub(crate) rate: f32,
    /// Seconds each puff lives.
    pub(crate) life_secs: f32,
    /// Seconds it takes to thicken in.
    pub(crate) fade_in_secs: f32,
    /// How high (m) above the machine's base the front vent is...
    pub(crate) front_height: f32,
    /// ...and the side vents.
    pub(crate) side_height: f32,
    /// Which way (degrees about up) the machine's front faces in its own
    /// frame — the models don't say, so it's tunable.
    pub(crate) front_yaw_deg: f32,
    /// How fast (m/s) a puff leaves its vent...
    pub(crate) out_speed: f32,
    /// ...and starts sinking.
    pub(crate) fall_speed: f32,
    /// How fast (m/s²) it sinks harder (cold fog is heavy).
    pub(crate) gravity: f32,
    /// How hard (m/s²) it keeps spreading out along the floor.
    pub(crate) floor_spread: f32,
    /// How quickly its motion dies away (per second).
    pub(crate) drag: f32,
    /// Puff size (m) leaving the vent, and by the end of its life.
    pub(crate) start_size: f32,
    pub(crate) end_size: f32,
    /// Peak opacity (0..1).
    pub(crate) opacity: f32,
    /// Its own faint glow, so it shows where the light doesn't reach.
    pub(crate) glow: f32,
    /// Machines further than this (m) from the camera don't vent.
    pub(crate) max_distance: f32,
}

impl Default for MachineFog {
    fn default() -> Self {
        Self {
            enabled: true,
            rate: 4.5,
            life_secs: 2.0,
            fade_in_secs: 0.4,
            front_height: 1.0,
            side_height: 1.0,
            front_yaw_deg: 0.0,
            out_speed: 0.3,
            fall_speed: 0.25,
            gravity: 0.6,
            floor_spread: 0.75,
            drag: 1.0,
            start_size: 0.58,
            end_size: 1.7,
            opacity: 0.12,
            glow: 0.16,
            max_distance: 27.0,
        }
    }
}

/// On a perk machine's root (on its ground, turned with it): it vents fog
/// while the power's on. The vents' backlog of puffs owed lives here.
#[derive(Component, Default)]
pub(crate) struct MachineFogEmitter {
    owed: f32,
    /// The Pack-a-Punch's (its own fog settings and size), not a perk
    /// machine's.
    pap: bool,
}

impl MachineFogEmitter {
    /// The Pack-a-Punch's.
    pub(crate) fn pap() -> Self {
        Self { owed: 0.0, pap: true }
    }
}

/// The fog settings and half-size of the machine an emitter (or a puff) is
/// the Pack-a-Punch's or a perk machine's.
fn fog_for(pap: bool, perks: &PerkMachineSettings, pap_cfg: &crate::pap::PapSettings) -> (MachineFog, Vec3) {
    if pap {
        (pap_cfg.fog, pap_cfg.half_extents())
    } else {
        (perks.fog, perks.half_extents())
    }
}

/// One drifting fog puff.
#[derive(Component)]
struct FogPuff {
    velocity: Vec3,
    /// Out from the machine along the floor, for the spread.
    out: Vec3,
    /// The floor it settles onto.
    floor_y: f32,
    age: f32,
    roll: f32,
    spin: f32,
    /// A Pack-a-Punch puff (its settings, not the perk machines').
    pap: bool,
}

pub(crate) struct MachineFogPlugin;

impl Plugin for MachineFogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (emit_machine_fog, update_machine_fog)
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_machine_fog(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    map_lights: Res<crate::power::MapLightSettings>,
    settings: Res<PerkMachineSettings>,
    pap_cfg: Res<crate::pap::PapSettings>,
    assets: Option<Res<SmokeAssets>>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut emitters: Query<(&GlobalTransform, &mut MachineFogEmitter)>,
    puffs: Query<Entity, With<FogPuff>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    // How far the fog's come in with the power (0..=1).
    mut power: Local<f32>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let lobby = zombies_game(&local, &lobbies);
    let Some(lobby) = lobby else {
        for e in &puffs {
            commands.entity(e).despawn();
        }
        *power = 0.0;
        return;
    };
    let Some(assets) = assets else { return };
    let powered = shared::power::has_power(lobby.map, lobby.power_on) || map_lights.force_on;
    let dt = time.delta_secs();
    // Swells in over a couple of seconds once the power's on.
    *power = (*power + if powered { dt / 2.0 } else { -dt }).clamp(0.0, 1.0);
    let cam = camera.translation();
    let mut budget = MAX_PUFFS.saturating_sub(puffs.iter().count());

    for (gt, mut emitter) in &mut emitters {
        let (fog, half) = fog_for(emitter.pap, &settings, &pap_cfg);
        if !fog.enabled || *power <= 0.0 || fog.rate <= 0.0 {
            emitter.owed = 0.0;
            continue;
        }
        let front = Quat::from_rotation_y(fog.front_yaw_deg.to_radians());
        // (local position, local direction out of the vent)
        let vents = [
            (front * Vec3::new(0.0, fog.front_height, half.z + 0.03), front * Vec3::Z),
            (front * Vec3::new(-half.x - 0.03, fog.side_height, 0.0), front * Vec3::NEG_X),
            (front * Vec3::new(half.x + 0.03, fog.side_height, 0.0), front * Vec3::X),
        ];
        let base = gt.translation();
        if base.distance(cam) > fog.max_distance {
            emitter.owed = 0.0;
            continue;
        }
        emitter.owed = (emitter.owed + dt * fog.rate * *power).min(MAX_BACKLOG);
        while emitter.owed >= 1.0 {
            emitter.owed -= 1.0;
            for (pos, dir) in vents {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                *seq = seq.wrapping_add(1);
                let s = seq.wrapping_mul(2_654_435_761);
                let out = (gt.rotation() * dir).normalize_or_zero();
                let side = Vec3::Y.cross(out).normalize_or_zero();
                // A little spread across the vent and in speed.
                let jitter = (rand01(s ^ 0x1) - 0.5) * 0.8;
                let velocity = (out + side * jitter) * fog.out_speed * (0.7 + 0.6 * rand01(s ^ 0x2))
                    + Vec3::NEG_Y * fog.fall_speed;
                let at = gt.transform_point(pos) + side * (rand01(s ^ 0x3) - 0.5) * 0.15;
                let roll = rand_roll(s ^ 0x4);
                let material = materials.add(StandardMaterial {
                    // (Faded in by `update_machine_fog` from nothing.)
                    base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                    base_color_texture: Some(assets.texture.clone()),
                    alpha_mode: AlphaMode::Blend,
                    perceptual_roughness: 1.0,
                    reflectance: 0.0,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                commands.spawn((
                    StateScoped(AppState::InGame),
                    FogPuff {
                        velocity,
                        out: (out + side * jitter).normalize_or_zero(),
                        floor_y: base.y,
                        age: 0.0,
                        roll,
                        spin: (rand01(s ^ 0x5) - 0.5) * 0.6,
                        pap: emitter.pap,
                    },
                    Mesh3d(assets.mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(at).with_scale(Vec3::ZERO),
                    NoFrustumCulling,
                    NotShadowCaster,
                ));
            }
        }
    }
}

fn update_machine_fog(
    time: Res<Time>,
    settings: Res<PerkMachineSettings>,
    pap_cfg: Res<crate::pap::PapSettings>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut puffs: Query<(Entity, &mut Transform, &mut FogPuff, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cam_pos = camera.translation();
    let cam_up = camera.up().as_vec3();
    let cam_right = camera.right().as_vec3();

    for (entity, mut t, mut puff, material) in &mut puffs {
        let (fog, _) = fog_for(puff.pap, &settings, &pap_cfg);
        let life = fog.life_secs.max(0.1);
        puff.age += dt;
        let k = puff.age / life;
        if k >= 1.0 {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }
        let size = fog.start_size.lerp(fog.end_size, k.sqrt());

        // Sinks, then rolls out along the floor — its middle kept a bit
        // above it, so the puff sits on the ground rather than in it.
        puff.velocity.y -= fog.gravity * dt;
        let out = puff.out;
        puff.velocity += out * fog.floor_spread * dt;
        puff.velocity *= (-fog.drag * dt).exp();
        let mut pos = t.translation + puff.velocity * dt;
        let rest = puff.floor_y + size * 0.3;
        if pos.y < rest {
            pos.y = rest;
            puff.velocity.y = puff.velocity.y.max(0.0);
        }
        t.translation = pos;
        t.scale = Vec3::splat(size);
        puff.roll += puff.spin * dt;
        t.rotation = smoke_billboard_rotation(pos, cam_pos, cam_up, cam_right, puff.roll);

        let fade_in = (puff.age / fog.fade_in_secs.max(0.01)).min(1.0);
        let alpha = fog.opacity * fade_in * (1.0 - k).powf(1.5);
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::srgba(1.0, 1.0, 1.0, alpha);
            m.emissive = LinearRgba::gray(fog.glow);
        }
    }
}
