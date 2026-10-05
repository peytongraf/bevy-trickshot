//! The Mystery Box's effects (`crate::mystery_box`), as Call of Duty draws
//! them: shut, now and then a tiny quick crackle of lightning off its
//! edges; open, smoke rolling up out of it and spilling over its sides,
//! lightning crackling round its rim far more often, and specks of light
//! drifting up out of it.
//!
//! The open effects follow how open the box is ([`MysteryBox::openness`],
//! the glow's fade), so they swell in with the lid and die away as it
//! shuts. Everything's in world space, worked out from the box's own frame
//! (so it turns and scales with it), `StateScoped(InGame)`, and cleared the
//! moment there's no box — nothing carries into the next game.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use shared::mystery_box::HALF_EXTENTS;

use crate::mystery_box::MysteryBox;
use crate::util::{rand01, rand_roll};
use crate::vfx::{smoke_billboard_rotation, SmokeAssets};
use crate::{AppState, WorldModelCamera};

/// Lightning's colour (linear, before its brightness): a cold blue-white.
const ARC_COLOR: [f32; 3] = [0.62, 0.76, 1.0];
const ARC_BRIGHTNESS: f32 = 14.0;
/// Arcs a second, shut (rare and subtle) and wide open.
const ARC_RATE_SHUT: f32 = 0.9;
const ARC_RATE_OPEN: f32 = 7.0;
/// An arc's length (m, the box as made) — short shut, longer open...
const ARC_LENGTH_SHUT: (f32, f32) = (0.05, 0.13);
const ARC_LENGTH_OPEN: (f32, f32) = (0.12, 0.32);
/// ...its thickness (m), and how long (s) it flickers.
const ARC_WIDTH: f32 = 0.006;
const ARC_SECS: (f32, f32) = (0.05, 0.13);
/// Kinks in an arc, and in the fork off it.
const ARC_SEGMENTS: usize = 6;
const FORK_SEGMENTS: usize = 3;

/// Smoke puffs a second, wide open, and how long (s) each lives.
const SMOKE_RATE: f32 = 16.0;
const SMOKE_SECS: (f32, f32) = (1.6, 2.6);
/// A puff's size (m) leaving the box, and by the end of its life.
const SMOKE_SIZE: (f32, f32) = (0.22, 0.75);
/// How thick a puff gets, and its own faint glow (so it's never black).
const SMOKE_OPACITY: f32 = 0.32;
const SMOKE_GLOW: f32 = 0.12;
/// How fast (m/s) it leaves the box sideways / up, how quickly (per s) that
/// dies away, and how hard (m/s²) it sinks once out — rolling over the rim.
const SMOKE_OUT_SPEED: f32 = 0.35;
const SMOKE_RISE_SPEED: f32 = 0.18;
const SMOKE_DRAG: f32 = 1.1;
const SMOKE_SINK: f32 = 0.12;

/// Specks a second, wide open, how long (s) each lives, their size (m) and
/// how fast (m/s) they float up.
const SPECK_RATE: f32 = 26.0;
const SPECK_SECS: (f32, f32) = (1.0, 2.2);
const SPECK_SIZE: (f32, f32) = (0.006, 0.014);
const SPECK_RISE: (f32, f32) = (0.2, 0.6);

/// Most of each alive at once (a long frame doesn't dump a pile).
const MAX_PUFFS: usize = 90;
const MAX_SPECKS: usize = 120;
const MAX_BACKLOG: f32 = 3.0;

pub(crate) struct MysteryBoxVfxPlugin;

impl Plugin for MysteryBoxVfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BoxFxState>()
            .add_systems(Startup, make_assets)
            .add_systems(
                Update,
                (emit_box_fx, update_arcs, update_puffs, update_specks)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Resource)]
struct BoxFxAssets {
    segment: Handle<Mesh>,
    arc: Handle<StandardMaterial>,
    speck: Handle<Mesh>,
    speck_glow: Handle<StandardMaterial>,
}

fn make_assets(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let [r, g, b] = ARC_COLOR;
    commands.insert_resource(BoxFxAssets {
        segment: meshes.add(Cylinder::new(0.5, 1.0)),
        arc: materials.add(StandardMaterial {
            base_color: LinearRgba::rgb(r * ARC_BRIGHTNESS, g * ARC_BRIGHTNESS, b * ARC_BRIGHTNESS).into(),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        }),
        speck: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        speck_glow: materials.add(StandardMaterial {
            base_color: LinearRgba::rgb(6.0, 5.4, 4.2).into(),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        }),
    });
}

/// What's owed of each effect, and the random sequence.
#[derive(Resource, Default)]
struct BoxFxState {
    arcs_owed: f32,
    puffs_owed: f32,
    specks_owed: f32,
    seq: u32,
}

impl BoxFxState {
    /// A fresh random number in 0..1.
    fn next(&mut self) -> f32 {
        self.seq = self.seq.wrapping_add(1);
        rand01(self.seq.wrapping_mul(2_654_435_761) ^ 0x5bd1_e995)
    }

    fn between(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// A crackle of lightning: its segments are children; gone after `secs`.
#[derive(Component)]
struct BoxArc {
    age: f32,
    secs: f32,
}

/// A smoke puff leaving the box.
#[derive(Component)]
struct BoxPuff {
    velocity: Vec3,
    age: f32,
    secs: f32,
    /// The rim's height (world), past which it starts sinking.
    rim_y: f32,
    roll: f32,
    spin: f32,
}

/// A speck of light floating up.
#[derive(Component)]
struct BoxSpeck {
    velocity: Vec3,
    age: f32,
    secs: f32,
    size: f32,
    twinkle: f32,
}

/// Owe each effect its share of this frame — the shut crackle always, the
/// rest as open as the box is — and spawn what's owed.
#[allow(clippy::too_many_arguments)]
fn emit_box_fx(
    time: Res<Time>,
    assets: Option<Res<BoxFxAssets>>,
    smoke: Option<Res<SmokeAssets>>,
    mut state: ResMut<BoxFxState>,
    boxes: Query<(&GlobalTransform, &MysteryBox)>,
    fx: Query<Entity, Or<(With<BoxArc>, With<BoxPuff>, With<BoxSpeck>)>>,
    puffs: Query<(), With<BoxPuff>>,
    specks: Query<(), With<BoxSpeck>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some((gt, mbox)) = boxes.iter().next() else {
        for e in &fx {
            commands.entity(e).despawn();
        }
        *state = BoxFxState::default();
        return;
    };
    let (Some(assets), Some(smoke)) = (assets, smoke) else { return };
    let dt = mbox.frame_secs(time.delta_secs());
    let open = mbox.openness();
    let scale = gt.compute_transform().scale.x.max(0.01);
    let half = HALF_EXTENTS;
    let top = half.y * 2.0;

    // Lightning, shut or open.
    let arc_rate = ARC_RATE_SHUT + (ARC_RATE_OPEN - ARC_RATE_SHUT) * open;
    state.arcs_owed = (state.arcs_owed + dt * arc_rate * 2.0 * state.next()).min(MAX_BACKLOG);
    while state.arcs_owed >= 1.0 {
        state.arcs_owed -= 1.0;
        let opened = state.next() < open;
        // Where it leaps from (the box's own frame): shut, anywhere along
        // its top edges and corners; open, round the rim — leaping out and up.
        let (from, out) = if !opened && state.next() < 0.3 {
            // A corner, somewhere up its height.
            let sx = if state.next() < 0.5 { -1.0 } else { 1.0 };
            let sz = if state.next() < 0.5 { -1.0 } else { 1.0 };
            let y = state.between((0.15, 1.0)) * top;
            (Vec3::new(sx * half.x, y, sz * half.z), Vec3::new(sx, 0.2, sz))
        } else if state.next() < 0.7 {
            // A long edge, front or back.
            let sz = if state.next() < 0.5 { -1.0 } else { 1.0 };
            let x = state.between((-1.0, 1.0)) * half.x;
            (Vec3::new(x, top, sz * half.z), Vec3::new(0.0, if opened { 1.0 } else { 0.5 }, sz))
        } else {
            // A short end.
            let sx = if state.next() < 0.5 { -1.0 } else { 1.0 };
            let z = state.between((-1.0, 1.0)) * half.z;
            (Vec3::new(sx * half.x, top, z), Vec3::new(sx, if opened { 1.0 } else { 0.5 }, 0.0))
        };
        let wobble = Vec3::new(state.between((-0.6, 0.6)), state.between((0.0, 0.6)), state.between((-0.6, 0.6)));
        let dir = (out.normalize() + wobble).normalize_or(Vec3::Y);
        let length = state.between(if opened { ARC_LENGTH_OPEN } else { ARC_LENGTH_SHUT });
        spawn_arc(&mut commands, &assets, &mut state, gt, from, dir, length, scale);
    }

    if open <= 0.01 {
        state.puffs_owed = 0.0;
        state.specks_owed = 0.0;
        return;
    }

    // Smoke, rolling up out of the opening and over the sides.
    state.puffs_owed = (state.puffs_owed + dt * SMOKE_RATE * open).min(MAX_BACKLOG);
    let mut room = MAX_PUFFS.saturating_sub(puffs.iter().count());
    while state.puffs_owed >= 1.0 && room > 0 {
        state.puffs_owed -= 1.0;
        room -= 1;
        let x = state.between((-0.85, 0.85));
        let z = state.between((-0.7, 0.7));
        let local = Vec3::new(x * half.x, top - 0.02, z * half.z);
        // Out toward the nearer end, and the front (more than the back:
        // the open lid's behind it).
        let sideways = Vec3::new(x.signum() * state.between((0.4, 1.0)), 0.0, state.between((-0.2, 1.0)));
        let velocity = gt.affine().transform_vector3(
            sideways * SMOKE_OUT_SPEED + Vec3::Y * SMOKE_RISE_SPEED * state.between((0.6, 1.4)),
        ) / scale;
        let at = gt.transform_point(local);
        let material = materials.add(StandardMaterial {
            // (Faded in by `update_puffs` from nothing.)
            base_color: Color::srgba(0.86, 0.83, 0.76, 0.0),
            base_color_texture: Some(smoke.texture.clone()),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        let roll = rand_roll(state.seq.wrapping_mul(97));
        let spin = state.between((-0.5, 0.5));
        let secs = state.between(SMOKE_SECS);
        commands.spawn((
            StateScoped(AppState::InGame),
            BoxPuff {
                velocity,
                age: 0.0,
                secs,
                rim_y: at.y,
                roll,
                spin,
            },
            Mesh3d(smoke.mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(at).with_scale(Vec3::ZERO),
            NoFrustumCulling,
            NotShadowCaster,
        ));
    }

    // Specks of light floating up.
    state.specks_owed = (state.specks_owed + dt * SPECK_RATE * open).min(MAX_BACKLOG);
    let mut room = MAX_SPECKS.saturating_sub(specks.iter().count());
    while state.specks_owed >= 1.0 && room > 0 {
        state.specks_owed -= 1.0;
        room -= 1;
        let local = Vec3::new(
            state.between((-0.9, 0.9)) * half.x,
            top * state.between((0.6, 1.1)),
            state.between((-0.8, 0.8)) * half.z,
        );
        let drift = Vec3::new(state.between((-0.12, 0.12)), 0.0, state.between((-0.12, 0.12)));
        let velocity = drift + Vec3::Y * state.between(SPECK_RISE) * scale;
        let size = state.between(SPECK_SIZE) * scale;
        let secs = state.between(SPECK_SECS);
        let twinkle = state.between((0.0, std::f32::consts::TAU));
        commands.spawn((
            StateScoped(AppState::InGame),
            BoxSpeck {
                velocity,
                age: 0.0,
                secs,
                size,
                twinkle,
            },
            Mesh3d(assets.speck.clone()),
            MeshMaterial3d(assets.speck_glow.clone()),
            Transform::from_translation(gt.transform_point(local)).with_scale(Vec3::ZERO),
            NotShadowCaster,
        ));
    }
}

/// One crackle: a jagged line `length` long from `from` along `dir` (the
/// box's own frame), with a short fork off it now and then.
#[allow(clippy::too_many_arguments)]
fn spawn_arc(
    commands: &mut Commands,
    assets: &BoxFxAssets,
    state: &mut BoxFxState,
    gt: &GlobalTransform,
    from: Vec3,
    dir: Vec3,
    length: f32,
    scale: f32,
) {
    let side = dir.any_orthonormal_vector();
    let other = dir.cross(side);
    let step = length / ARC_SEGMENTS as f32;
    let kink = |state: &mut BoxFxState, amount: f32| {
        (side * state.between((-1.0, 1.0)) + other * state.between((-1.0, 1.0))) * amount
    };
    let mut points = vec![from];
    for i in 1..=ARC_SEGMENTS {
        points.push(from + dir * step * i as f32 + kink(state, step * 0.7));
    }
    let mut lines: Vec<(Vec3, Vec3, f32)> = points.windows(2).map(|w| (w[0], w[1], 1.0)).collect();
    if state.next() < 0.5 {
        let at = 1 + (state.next() * (ARC_SEGMENTS - 2) as f32) as usize;
        let fork_dir = (dir + kink(state, 1.2)).normalize_or(dir);
        let mut p = points[at];
        for _ in 0..FORK_SEGMENTS {
            let next = p + fork_dir * step * 0.8 + kink(state, step * 0.6);
            lines.push((p, next, 0.6));
            p = next;
        }
    }
    let secs = state.between(ARC_SECS);
    let width = ARC_WIDTH * scale;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            BoxArc { age: 0.0, secs },
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .with_children(|arc| {
            for (a, b, thick) in lines {
                let (a, b) = (gt.transform_point(a), gt.transform_point(b));
                let d = b - a;
                let len = d.length().max(1e-4);
                arc.spawn((
                    Mesh3d(assets.segment.clone()),
                    MeshMaterial3d(assets.arc.clone()),
                    Transform {
                        translation: (a + b) * 0.5,
                        rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
                        scale: Vec3::new(width * thick, len, width * thick),
                    },
                    NotShadowCaster,
                ));
            }
        });
}

fn update_arcs(time: Res<Time>, mut arcs: Query<(Entity, &mut BoxArc)>, mut commands: Commands) {
    for (e, mut arc) in &mut arcs {
        arc.age += time.delta_secs();
        if arc.age >= arc.secs {
            commands.entity(e).despawn();
        }
    }
}

/// Each puff drifts out, slowing, sinks once it's over the rim, and grows
/// and thins as it goes.
fn update_puffs(
    time: Res<Time>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut puffs: Query<(Entity, &mut Transform, &mut BoxPuff, &MeshMaterial3d<StandardMaterial>), Without<BoxSpeck>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let (cam_pos, cam_up, cam_right) = (camera.translation(), camera.up().as_vec3(), camera.right().as_vec3());
    for (e, mut t, mut puff, material) in &mut puffs {
        puff.age += dt;
        let k = puff.age / puff.secs;
        if k >= 1.0 {
            materials.remove(&material.0);
            commands.entity(e).despawn();
            continue;
        }
        if t.translation.y > puff.rim_y || k > 0.3 {
            puff.velocity.y -= SMOKE_SINK * dt;
        }
        puff.velocity *= (-SMOKE_DRAG * dt).exp();
        let pos = t.translation + puff.velocity * dt;
        t.translation = pos;
        t.scale = Vec3::splat(SMOKE_SIZE.0.lerp(SMOKE_SIZE.1, k.sqrt()));
        puff.roll += puff.spin * dt;
        t.rotation = smoke_billboard_rotation(pos, cam_pos, cam_up, cam_right, puff.roll);
        let alpha = SMOKE_OPACITY * (k / 0.15).min(1.0) * (1.0 - k).powf(1.5);
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::srgba(0.86, 0.83, 0.76, alpha);
            m.emissive = LinearRgba::gray(SMOKE_GLOW * alpha / SMOKE_OPACITY);
        }
    }
}

/// Each speck floats up, twinkling, and shrinks away.
fn update_specks(
    time: Res<Time>,
    mut specks: Query<(Entity, &mut Transform, &mut BoxSpeck), Without<BoxPuff>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (e, mut t, mut speck) in &mut specks {
        speck.age += dt;
        let k = speck.age / speck.secs;
        if k >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        t.translation += speck.velocity * dt;
        let twinkle = 0.55 + 0.45 * (speck.age * 14.0 + speck.twinkle).sin();
        let size = speck.size * (k / 0.1).min(1.0) * (1.0 - k) * twinkle;
        t.scale = Vec3::splat(size);
    }
}
