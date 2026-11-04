//! The ritual played out at its altar (`ZombiesLayout::ritual_altar`), by
//! the ritual's clock ([`EggRitual`], [`shared::easter_egg::RITUAL_SECS`]):
//!
//! 1. the dagger's driven point first into the middle of the altar's basin;
//! 2. Barrett's vial comes down over the basin, tips, and pours — a thin
//!    stream falling under gravity (its head falling from the vial's mouth,
//!    and once the pour stops, its tail after it) — and the blood spreads
//!    out unevenly across the basin's floor from where the stream lands,
//!    rising as it fills; the vial rights itself and is gone;
//! 3. the squad's blood fills it to the brim...
//! 4. ...and it spills over at the front (+z, toward the sarcophagus), runs
//!    across the top and down the altar's face, over its step, and spreads
//!    into a puddle on the floor — while all of it starts to glow;
//! 5. the sarcophagus lights up.
//!
//! The blood is [`BloodMaterial`] (`blood.wgsl`): uneven, wet, lit like the
//! rest of the map. The altar's shape ([`ALTAR_APOTHEM`], [`BASIN_RADIUS`],
//! …) is its model's (`super::RITUAL_ALTAR_MODEL`) — change them with it. The
//! pieces are there from the ritual's start until the egg's reset, and as it
//! ended after it; `StateScoped(InGame)`, so gone with the game.

use std::f32::consts::PI;

use bevy::pbr::{ExtendedMaterial, MaterialExtension, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use lightyear::prelude::LocalId;
use shared::easter_egg::{EggStep, RITUAL_ALTAR_TOP, RITUAL_SECS, SARCOPHAGUS_HALF_EXTENTS};
use shared::Lobby;

use crate::net::GameClient;
use crate::AppState;

use super::{dagger, egg_game, ramp, DaggerPose, EggRitual, VIAL_MODEL};

pub(super) struct RitualFxPlugin;

impl Plugin for RitualFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BloodMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .add_systems(Startup, make_assets)
        .add_systems(
            Update,
            (sync_ritual_fx, animate_ritual)
                .chain()
                .after(super::track_ritual)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

// --- the altar's shape — its model's (as made: its origin on the ground under
// its middle, its front +z) -------------------------------------------------------------------

/// How far (m) its body's sides are from its middle, and its step's.
pub(crate) const ALTAR_APOTHEM: f32 = 1.05;
pub(crate) const STEP_APOTHEM: f32 = 1.3;
/// How high (m) its step's top is.
pub(crate) const STEP_TOP: f32 = 0.18;
/// The round basin in its top: how wide (radius, m) and deep.
pub(crate) const BASIN_RADIUS: f32 = 0.475;
pub(crate) const BASIN_DEPTH: f32 = 0.05;
const TOP: f32 = RITUAL_ALTAR_TOP;

/// The vial's height (m) — `VIAL_MODEL`, its origin under its base.
const VIAL_HEIGHT: f32 = 0.12;

// --- its timings (s into the ritual) ------------------------------------------------

/// The vial comes down over the basin, tips over...
const VIAL_IN: (f32, f32) = (3.0, 3.4);
const VIAL_TIP: (f32, f32) = (3.3, 3.9);
/// ...pours from then till then...
const POUR_FROM: f32 = 3.75;
const POUR_TO: f32 = 5.9;
/// ...rights itself, lifts off and is gone.
const VIAL_RIGHT: (f32, f32) = (5.95, 6.35);
const VIAL_GONE: f32 = 6.6;
/// How far (rad) it tips.
const VIAL_TIP_ANGLE: f32 = 125.0 * PI / 180.0;
/// Where its mouth is over the basin, tipped (off the middle, clear of the
/// dagger).
const POUR_AT: Vec2 = Vec2::new(-0.12, 0.0);
/// How high its mouth pours from (m).
const POUR_HEIGHT: f32 = 1.28;
/// The basin fills from its floor to just under its brim, Barrett's blood
/// then the squad's...
const FILL: [(f32, f32); 3] = [(3.9, 0.0), (6.0, 0.6), (8.0, 1.0)];
/// ...overflows down the front — (s, m along the run) — ...
const RUN_KNOTS_SECS: [f32; 5] = [8.0, 9.2, 9.65, 10.15, 10.35];
/// ...and spreads on the floor.
const PUDDLE_SPREAD: (f32, f32) = (10.3, RITUAL_SECS);
/// All of it glowing.
const GLOW: (f32, f32) = (9.5, 13.0);

/// The floor puddle's quad: its middle out in front of the step, and how
/// far (m) it reaches either side.
const PUDDLE_HALF: f32 = 0.9;
const PUDDLE_AT_Z: f32 = STEP_APOTHEM + 0.75;

const GRAVITY: f32 = 9.8;

// --- the blood's material ----------------------------------------------------------

#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Mirrors `BloodParams` in `blood.wgsl`.
    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct BloodParams {
        pub(crate) spread: f32,
        pub(crate) seed: f32,
        pub(crate) glow: f32,
        pub(crate) mode: f32,
        pub(crate) start: Vec2,
        pub(crate) clip: f32,
        pub(crate) reach: f32,
        pub(crate) size: f32,
        pub(crate) _pad0: f32,
        pub(crate) _pad1: f32,
        pub(crate) _pad2: f32,
    }
}
use params::BloodParams;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct BloodSurface {
    #[uniform(100)]
    params: BloodParams,
}

impl MaterialExtension for BloodSurface {
    fn fragment_shader() -> ShaderRef {
        "shaders/blood.wgsl".into()
    }
}

/// Blood lying on a surface — a pool, or a run (see `blood.wgsl`).
pub(crate) type BloodMaterial = ExtendedMaterial<StandardMaterial, BloodSurface>;

fn blood_material(params: BloodParams) -> BloodMaterial {
    BloodMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            alpha_mode: AlphaMode::Blend,
            reflectance: 0.5,
            // (Off the stone under it.)
            depth_bias: 50.0,
            ..default()
        },
        extension: BloodSurface { params },
    }
}

// --- the meshes --------------------------------------------------------------------

/// A flat square `half` m from its middle to its sides, facing up, its uv
/// across it (u along +x, v along +z).
fn flat_quad(half: f32) -> Mesh {
    let h = half;
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-h, 0.0, -h], [h, 0.0, -h], [h, 0.0, h], [-h, 0.0, h]],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        .with_inserted_indices(Indices::U32(vec![0, 3, 2, 0, 2, 1]))
}

/// The overflow's path, from the basin's brim at the front: across the top,
/// over its edge, down its face, across the step and down it to the floor —
/// each point with the way its surface faces. Just off the stone.
fn run_path() -> [(Vec3, Vec3); 8] {
    let off = 0.003;
    let diag = Vec3::new(0.0, 1.0, 1.0).normalize();
    [
        (Vec3::new(0.0, TOP + off, BASIN_RADIUS - 0.01), Vec3::Y),
        (Vec3::new(0.0, TOP + off, ALTAR_APOTHEM - 0.012), Vec3::Y),
        (Vec3::new(0.0, TOP - 0.012, ALTAR_APOTHEM + off), diag),
        (Vec3::new(0.0, STEP_TOP + 0.012, ALTAR_APOTHEM + off), Vec3::Z),
        (Vec3::new(0.0, STEP_TOP + off, ALTAR_APOTHEM + 0.015), diag),
        (Vec3::new(0.0, STEP_TOP + off, STEP_APOTHEM - 0.012), Vec3::Y),
        (Vec3::new(0.0, STEP_TOP - 0.012, STEP_APOTHEM + off), diag),
        (Vec3::new(0.0, 0.004, STEP_APOTHEM + off), Vec3::Z),
    ]
}

/// How far (m) along the run each of its points is.
fn run_lengths() -> [f32; 8] {
    let path = run_path();
    let mut out = [0.0; 8];
    for i in 1..8 {
        out[i] = out[i - 1] + path[i].0.distance(path[i - 1].0);
    }
    out
}

/// A strip `width` m wide along [`run_path`], uv (across it, m along it).
fn run_mesh(width: f32) -> Mesh {
    let path = run_path();
    let lengths = run_lengths();
    let (mut pos, mut nor, mut uv, mut idx) = (vec![], vec![], vec![], vec![]);
    for (i, (p, n)) in path.iter().enumerate() {
        for side in [-0.5f32, 0.5] {
            pos.push((*p + Vec3::X * side * width).to_array());
            nor.push(n.to_array());
            uv.push([side + 0.5, lengths[i]]);
        }
        if i > 0 {
            let k = (i as u32 - 1) * 2;
            idx.extend([k, k + 1, k + 3, k, k + 3, k + 2]);
        }
    }
    // (Each quad wound to face its own way: flipped where it came out
    // backwards.)
    for q in idx.chunks_mut(3) {
        let [a, b, c] = [q[0], q[1], q[2]].map(|i| Vec3::from_array(pos[i as usize]));
        let n = Vec3::from_array(nor[q[0] as usize]);
        if (b - a).cross(c - a).dot(n) < 0.0 {
            q.swap(1, 2);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nor)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

#[derive(Resource)]
struct RitualAssets {
    basin_quad: Handle<Mesh>,
    puddle_quad: Handle<Mesh>,
    run: Handle<Mesh>,
    stream: Handle<Mesh>,
    stream_material: Handle<StandardMaterial>,
}

fn make_assets(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    commands.insert_resource(RitualAssets {
        basin_quad: meshes.add(flat_quad(BASIN_RADIUS)),
        puddle_quad: meshes.add(flat_quad(PUDDLE_HALF)),
        run: meshes.add(run_mesh(0.085)),
        stream: meshes.add(Cylinder::new(1.0, 1.0)),
        stream_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.0, 0.01),
            perceptual_roughness: 0.05,
            reflectance: 0.5,
            ..default()
        }),
    });
}

// --- in game -----------------------------------------------------------------------

/// The ritual's pieces, under one root — at the altar.
#[derive(Component)]
struct RitualFx;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum RitualPart {
    Dagger,
    Vial,
    Stream,
    Basin,
    Run,
    Puddle,
    /// A warm light over the altar (candles, until there are some), and the
    /// blood's own glow.
    AltarLight,
    PoolLight,
}

/// The light on the sarcophagus as it's done (by itself — not at the altar).
#[derive(Component)]
struct SarcophagusLight;

/// The ritual's pieces there from when it begins (and as it ended, after —
/// until the egg's reset), on a map with the ritual altar placed.
fn sync_ritual_fx(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    assets: Res<RitualAssets>,
    asset_server: Res<AssetServer>,
    mut blood: ResMut<Assets<BloodMaterial>>,
    roots: Query<Entity, With<RitualFx>>,
    mut commands: Commands,
) {
    let altar = egg_game(&local, &lobbies)
        .filter(|l| l.easter_egg.step >= EggStep::Ritual)
        .and_then(|l| shared::level::layout(l.map).ritual_altar);
    let Some(altar) = altar else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    if !roots.is_empty() {
        return;
    }
    let pool = |seed: f32, start: Vec2, clip: f32, size: f32| BloodParams {
        seed,
        start,
        clip,
        size,
        ..default()
    };
    let basin = blood.add(blood_material(pool(3.1, POUR_AT / BASIN_RADIUS, 1.0, BASIN_RADIUS)));
    let puddle_start = Vec2::new(0.0, (STEP_APOTHEM + 0.02 - PUDDLE_AT_Z) / PUDDLE_HALF);
    let puddle = blood.add(blood_material(pool(7.7, puddle_start, 0.0, PUDDLE_HALF)));
    let run = blood.add(blood_material(BloodParams { seed: 5.3, mode: 1.0, ..default() }));
    let red = Color::srgb(1.0, 0.08, 0.06);
    commands
        .spawn((StateScoped(AppState::InGame), RitualFx, crate::util::placed(altar), Visibility::default()))
        .with_children(|fx| {
            // (The ritual moves these; the dagger's model is point down
            // under its own.)
            fx.spawn((RitualPart::Dagger, Transform::default(), Visibility::Hidden))
                .with_child(dagger(&asset_server, Vec3::ZERO, DaggerPose::PointDown));
            fx.spawn((
                RitualPart::Vial,
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(VIAL_MODEL))),
                Transform::default(),
                Visibility::Hidden,
            ));
            fx.spawn((
                RitualPart::Stream,
                Mesh3d(assets.stream.clone()),
                MeshMaterial3d(assets.stream_material.clone()),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
            ));
            for (part, mesh, material, at) in [
                (RitualPart::Basin, &assets.basin_quad, basin, Vec3::Y * (TOP - BASIN_DEPTH)),
                (RitualPart::Run, &assets.run, run, Vec3::ZERO),
                (RitualPart::Puddle, &assets.puddle_quad, puddle, Vec3::new(0.0, 0.004, PUDDLE_AT_Z)),
            ] {
                fx.spawn((
                    part,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(at),
                    Visibility::Hidden,
                    NotShadowCaster,
                ));
            }
            fx.spawn((
                RitualPart::AltarLight,
                PointLight {
                    color: Color::srgb(1.0, 0.62, 0.32),
                    intensity: 0.0,
                    range: 5.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.35, TOP + 0.9, 0.6),
            ));
            fx.spawn((
                RitualPart::PoolLight,
                PointLight {
                    color: red,
                    intensity: 0.0,
                    range: 6.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, TOP + 0.35, 0.3),
            ));
        });
    commands.spawn((
        StateScoped(AppState::InGame),
        RitualFx,
        SarcophagusLight,
        PointLight {
            color: red,
            intensity: 0.0,
            range: 8.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::default(),
    ));
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powf(2.2)
}

/// Along `knots` of (s, value), `t` s in — straight between them.
fn along(knots: &[(f32, f32)], t: f32) -> f32 {
    let mut v = knots[0].1;
    for w in knots.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if t >= t0 {
            v = v0 + (v1 - v0) * ramp(t, t0, t1);
        }
    }
    v
}

/// The vial `t` s in, in the altar's frame — and where its mouth is.
fn vial_pose(t: f32) -> (Transform, Vec3) {
    let tip = VIAL_TIP_ANGLE * ease(ramp(t, VIAL_TIP.0, VIAL_TIP.1)) * (1.0 - ease(ramp(t, VIAL_RIGHT.0, VIAL_RIGHT.1)));
    // (Tipped toward +x, its top over the basin.)
    let rotation = Quat::from_rotation_z(-tip);
    let mouth_up = rotation * Vec3::Y * VIAL_HEIGHT;
    let full_mouth = Quat::from_rotation_z(-VIAL_TIP_ANGLE) * Vec3::Y * VIAL_HEIGHT;
    // Its base where its mouth, fully tipped, is over the pour spot.
    let base_xz = Vec3::new(POUR_AT.x, 0.0, POUR_AT.y) - full_mouth.with_y(0.0);
    let base_y = POUR_HEIGHT - full_mouth.y;
    let drop_in = 0.35 * (1.0 - ease(ramp(t, VIAL_IN.0, VIAL_IN.1)));
    let lift_off = 0.35 * ease(ramp(t, VIAL_RIGHT.1 - 0.15, VIAL_GONE));
    let base = base_xz.with_y(base_y + drop_in + lift_off);
    (Transform::from_translation(base).with_rotation(rotation), base + mouth_up)
}

/// How full (0..1) the basin is `t` s in.
fn fill(t: f32) -> f32 {
    ease_out(along(&FILL, t))
}

/// Play the ritual out on its pieces by its clock — or, once it's over,
/// hold them as it ended.
#[allow(clippy::type_complexity)]
fn animate_ritual(
    time: Res<Time>,
    ritual: Res<EggRitual>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut parts: Query<
        (&RitualPart, &mut Transform, &mut Visibility, Option<&MeshMaterial3d<BloodMaterial>>, Option<&mut PointLight>),
        Without<SarcophagusLight>,
    >,
    mut sarcophagus_light: Query<(&mut PointLight, &mut Transform), With<SarcophagusLight>>,
    mut blood: ResMut<Assets<BloodMaterial>>,
) {
    let Some(lobby) = egg_game(&local, &lobbies) else { return };
    let t = ritual.t.unwrap_or(RITUAL_SECS);
    let now = time.elapsed_secs();
    let flicker = 0.85 + 0.1 * (now * 7.3).sin() + 0.05 * (now * 17.1).sin();
    let wobble = (now * 9.0).sin() * 0.5 + 0.5;
    let glow = ramp(t, GLOW.0, GLOW.1);
    let level = TOP - BASIN_DEPTH + 0.002 + (BASIN_DEPTH - 0.005) * fill(t);
    let (vial, mouth) = vial_pose(t);
    let lengths = run_lengths();
    let run_knots: [(f32, f32); 5] = std::array::from_fn(|i| (RUN_KNOTS_SECS[i], lengths[[0, 1, 3, 5, 7][i]]));
    for (part, mut transform, mut vis, material, light) in &mut parts {
        let mut params = None;
        let shown = match part {
            // Down from above, the blade half buried in the basin's floor.
            RitualPart::Dagger => {
                let fall = ramp(t, 0.3, 1.5).powi(3);
                let y = 1.65 + (TOP - BASIN_DEPTH + 0.02 - 1.65) * fall;
                transform.translation = Vec3::Y * y;
                t >= 0.3
            }
            RitualPart::Vial => {
                *transform = vial;
                (VIAL_IN.0..VIAL_GONE).contains(&t)
            }
            // Falling from its mouth: its head from when it starts pouring,
            // its tail once it stops — each under gravity — down to the
            // blood in the basin.
            RitualPart::Stream => {
                let fallen = |since: f32| 0.5 * GRAVITY * (t - since).max(0.0).powi(2);
                let head = (mouth.y - fallen(POUR_FROM)).max(level);
                let tail = if t < POUR_TO { mouth.y } else { mouth.y - fallen(POUR_TO) };
                let len = tail - head;
                let pouring = t >= POUR_FROM && len > 0.002;
                if pouring {
                    let sway = Vec3::new((now * 23.0).sin(), 0.0, (now * 31.0).cos()) * 0.0012;
                    let thin = if t < POUR_TO { 1.0 } else { 0.6 };
                    *transform = Transform::from_translation(mouth.with_y((head + tail) * 0.5) + sway)
                        .with_scale(Vec3::new(0.0045 * thin, len, 0.0045 * thin));
                }
                pouring
            }
            RitualPart::Basin => {
                transform.translation.y = level;
                // (From where the stream lands, out across the floor.)
                let spread = 2.0 * ease_out(ramp(t, POUR_FROM + 0.12, 7.2));
                params = Some((spread, 0.0));
                spread > 0.0
            }
            RitualPart::Run => {
                let reach = along(&run_knots, t);
                params = Some((0.0, reach));
                reach > 0.0
            }
            RitualPart::Puddle => {
                let spread = 1.15 * ease_out(ramp(t, PUDDLE_SPREAD.0, PUDDLE_SPREAD.1));
                params = Some((spread, 0.0));
                spread > 0.0
            }
            RitualPart::AltarLight => {
                if let Some(mut light) = light {
                    light.intensity = 30_000.0 * flicker;
                }
                continue;
            }
            RitualPart::PoolLight => {
                if let Some(mut light) = light {
                    light.intensity = 60_000.0 * glow * (0.8 + 0.2 * wobble);
                }
                continue;
            }
        };
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        if let (Some((spread, reach)), Some(material)) = (params, material) {
            if let Some(m) = blood.get_mut(&material.0) {
                let p = &mut m.extension.params;
                (p.spread, p.reach, p.glow) = (spread, reach, glow);
            }
        }
    }
    if let (Some(sarcophagus), Ok((mut light, mut transform))) =
        (shared::level::layout(lobby.map).sarcophagus, sarcophagus_light.single_mut())
    {
        light.intensity = 250_000.0 * ramp(t, 12.5, 15.0) * (0.75 + 0.25 * wobble);
        transform.translation =
            sarcophagus.pos + Vec3::Y * (SARCOPHAGUS_HALF_EXTENTS.y * 2.0 + 0.5) * sarcophagus.scale;
    }
}
