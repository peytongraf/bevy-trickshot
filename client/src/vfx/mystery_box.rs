//! The Mystery Box's effects (`crate::mystery_box`), as Call of Duty draws
//! them: shut, now and then a tiny quick crackle of lightning off its
//! edges; open, smoke rolling up out of it and spilling over its sides,
//! lightning crackling round its rim far more often, and specks of light
//! drifting up out of it.
//!
//! Above it, always, Cold War's beacon: a jagged blue-white bolt of
//! lightning from the lid straight up into the sky, reshaping every fraction
//! of a second and shifting about over the top of the box, sometimes doubled
//! or forked, in a faint teal glow — so the box can be found from across the
//! map ([`update_beam`]). It lifts clear as the lid opens.
//!
//! The open effects follow how open the box is ([`MysteryBox::openness`],
//! the glow's fade), so they swell in with the lid and die away as it
//! shuts. Everything's in world space, worked out from the box's own frame
//! (so it turns and scales with it), `StateScoped(InGame)`, and cleared the
//! moment there's no box — nothing carries into the next game.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use shared::mystery_box::HALF_EXTENTS;

use crate::mystery_box::MysteryBox;
use crate::util::{rand01, rand_roll};
use crate::vfx::{smoke_billboard_rotation, SmokeAssets};
use crate::{AppState, WorldModelCamera};

/// Kinks in an arc, and in the fork off it.
const ARC_SEGMENTS: usize = 6;
const FORK_SEGMENTS: usize = 3;
/// Kinks up a strand — closer together low down, where it's seen up close.
const BEAM_SEGMENTS: usize = 30;
const BEAM_FORKS: usize = 3;
const BEAM_FORK_SEGMENTS: usize = 4;

/// How the box's effects look — the debug panel's "Mystery Box (Zombies)"
/// section ([`MysteryBoxFxDebug`]). Lengths are the box as made unless they
/// say otherwise; `(a, b)` pairs are a random range.
#[derive(Resource, Clone, PartialEq, Debug)]
pub(crate) struct MysteryBoxFxSettings {
    /// The beacon over the box at all.
    pub(crate) beam_enabled: bool,
    /// How see-through its bolts (core and halo) are: `1` solid, `0` gone.
    pub(crate) beam_bolt_opacity: f32,
    /// Lightning's colour (linear, before its brightness): a cold blue-white.
    pub(crate) arc_color: [f32; 3],
    pub(crate) arc_brightness: f32,
    /// Arcs a second, shut (rare and subtle) and wide open.
    pub(crate) arc_rate_shut: f32,
    pub(crate) arc_rate_open: f32,
    /// An arc's length (m, the box as made) — short shut, longer open...
    pub(crate) arc_length_shut: (f32, f32),
    pub(crate) arc_length_open: (f32, f32),
    /// ...its thickness (m), and how long (s) it flickers.
    pub(crate) arc_width: f32,
    pub(crate) arc_secs: (f32, f32),
    /// Smoke puffs a second, wide open, and how long (s) each lives.
    pub(crate) smoke_rate: f32,
    pub(crate) smoke_secs: (f32, f32),
    /// A puff's size (m) leaving the box, and by the end of its life.
    pub(crate) smoke_size: (f32, f32),
    /// How thick a puff gets, and its own faint glow (so it's never black).
    pub(crate) smoke_opacity: f32,
    pub(crate) smoke_glow: f32,
    /// How fast (m/s) it leaves the box sideways / up, how quickly (per s) that
    /// dies away, and how hard (m/s²) it sinks once out — rolling over the rim.
    pub(crate) smoke_out_speed: f32,
    pub(crate) smoke_rise_speed: f32,
    pub(crate) smoke_drag: f32,
    pub(crate) smoke_sink: f32,
    /// Specks a second, wide open, how long (s) each lives, their size (m) and
    /// how fast (m/s) they float up.
    pub(crate) speck_rate: f32,
    pub(crate) speck_secs: (f32, f32),
    pub(crate) speck_size: (f32, f32),
    pub(crate) speck_rise: (f32, f32),
    /// The beacon over the box ([`update_beam`]): how high (m) it reaches...
    pub(crate) beam_height: f32,
    /// ...where it starts over the box's top (m, the box as made) shut, and
    /// open — clear of the open lid and the prize hovering under it.
    pub(crate) beam_base_shut: f32,
    pub(crate) beam_base_open: f32,
    /// How long (s) one shape of it lasts before it reshapes.
    pub(crate) beam_reshape_secs: (f32, f32),
    /// How far (m) a kink strays sideways: this much at the base, growing with
    /// the square root of the height.
    pub(crate) beam_kink: f32,
    pub(crate) beam_kink_per_root_m: f32,
    /// How far (the box's half width / depth) its foot wanders over the box.
    pub(crate) beam_wander: (f32, f32),
    /// The chance a shape has a second strand beside the first, and forks.
    pub(crate) beam_second_chance: f32,
    pub(crate) beam_fork_chance: f32,
    /// A strand's core: white-blue and bright, this thick (m) up close, and at
    /// least this many metres thick per metre away (so it stays a few pixels
    /// wide across the map).
    pub(crate) beam_core_color: [f32; 3],
    pub(crate) beam_core_brightness: f32,
    pub(crate) beam_core_width: f32,
    pub(crate) beam_width_per_m: f32,
    /// Its halo: cyan, dimmer, this many times as wide.
    pub(crate) beam_halo_color: [f32; 3],
    pub(crate) beam_halo_brightness: f32,
    pub(crate) beam_halo_width: f32,
    /// The teal column of glow behind it: how wide (m) and bright.
    pub(crate) beam_glow_color: [f32; 3],
    pub(crate) beam_glow_brightness: f32,
    pub(crate) beam_glow_width: f32,
    /// The light it casts round the box (flickering with each shape).
    pub(crate) beam_light_intensity: f32,
    pub(crate) beam_light_range: f32,
}

impl Default for MysteryBoxFxSettings {
    fn default() -> Self {
        Self {
            beam_enabled: true,
            beam_bolt_opacity: 0.01,
            arc_color: [0.62, 0.76, 1.0],
            arc_brightness: 14.0,
            arc_rate_shut: 2.4,
            arc_rate_open: 7.0,
            arc_length_shut: (0.12, 0.2),
            arc_length_open: (0.14, 0.33),
            arc_width: 0.005,
            arc_secs: (0.07, 0.13),
            smoke_rate: 16.0,
            smoke_secs: (1.6, 2.6),
            smoke_size: (0.22, 0.75),
            smoke_opacity: 0.32,
            smoke_glow: 0.12,
            smoke_out_speed: 0.35,
            smoke_rise_speed: 0.18,
            smoke_drag: 1.1,
            smoke_sink: 0.12,
            speck_rate: 26.0,
            speck_secs: (1.0, 2.2),
            speck_size: (0.006, 0.014),
            speck_rise: (0.2, 0.6),
            beam_height: 140.0,
            beam_base_shut: 0.06,
            beam_base_open: 0.8,
            beam_reshape_secs: (0.035, 0.08),
            beam_kink: 0.185,
            beam_kink_per_root_m: 0.04,
            beam_wander: (0.05, 0.35),
            beam_second_chance: 0.18,
            beam_fork_chance: 0.52,
            beam_core_color: [0.78, 0.93, 1.0],
            beam_core_brightness: 45.0,
            beam_core_width: 0.002,
            beam_width_per_m: 0.0004,
            beam_halo_color: [0.25, 0.85, 1.0],
            beam_halo_brightness: 2.1,
            beam_halo_width: 4.8,
            beam_glow_color: [0.1, 0.75, 0.7],
            beam_glow_brightness: 0.02,
            beam_glow_width: 3.7,
            beam_light_intensity: 10_000.0,
            beam_light_range: 9.5,
        }
    }
}

/// Most of each alive at once (a long frame doesn't dump a pile).
const MAX_PUFFS: usize = 90;
const MAX_SPECKS: usize = 120;
const MAX_BACKLOG: f32 = 3.0;

pub(crate) struct MysteryBoxVfxPlugin;

impl Plugin for MysteryBoxVfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BoxFxState>()
            .init_resource::<MysteryBoxFxSettings>()
            .add_systems(Startup, make_assets)
            .add_systems(Update, apply_box_fx_look)
            .add_systems(
                Update,
                (emit_box_fx, update_arcs, update_puffs, update_specks, update_beam)
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
    beam_core: Handle<StandardMaterial>,
    beam_halo: Handle<StandardMaterial>,
    beam_glow: Handle<StandardMaterial>,
    glow_quad: Handle<Mesh>,
}

fn make_assets(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    // (The look as it starts; `apply_box_fx_look` keeps it to the settings.)
    let fx = MysteryBoxFxSettings::default();
    // (Lit by nothing, added over what's behind, and never fogged out — it's
    // to be seen from across the map.)
    let glowing = |color: [f32; 3], brightness: f32, texture: Option<Handle<Image>>| StandardMaterial {
        base_color: LinearRgba::rgb(color[0] * brightness, color[1] * brightness, color[2] * brightness).into(),
        base_color_texture: texture,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        fog_enabled: false,
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    let beam_core = materials.add(glowing(fx.beam_core_color, fx.beam_core_brightness, None));
    let beam_halo = materials.add(glowing(fx.beam_halo_color, fx.beam_halo_brightness, None));
    let beam_glow = materials.add(glowing(fx.beam_glow_color, fx.beam_glow_brightness, Some(images.add(glow_column_texture()))));
    let [r, g, b] = fx.arc_color;
    commands.insert_resource(BoxFxAssets {
        segment: meshes.add(Cylinder::new(0.5, 1.0)),
        arc: materials.add(StandardMaterial {
            base_color: LinearRgba::rgb(r * fx.arc_brightness, g * fx.arc_brightness, b * fx.arc_brightness).into(),
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
        beam_core,
        beam_halo,
        beam_glow,
        glow_quad: meshes.add(Rectangle::new(1.0, 1.0)),
    });
}

/// Keep the shared materials (the crackles', the beacon's) to the settings'
/// colours and brightness.
fn apply_box_fx_look(
    settings: Res<MysteryBoxFxSettings>,
    assets: Option<Res<BoxFxAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(assets) = assets else { return };
    if !settings.is_changed() && !assets.is_added() {
        return;
    }
    let fx = &*settings;
    let bolts = fx.beam_bolt_opacity.clamp(0.0, 1.0);
    for (handle, color, brightness, opacity) in [
        (&assets.arc, fx.arc_color, fx.arc_brightness, 1.0),
        (&assets.beam_core, fx.beam_core_color, fx.beam_core_brightness, bolts),
        (&assets.beam_halo, fx.beam_halo_color, fx.beam_halo_brightness, bolts),
        (&assets.beam_glow, fx.beam_glow_color, fx.beam_glow_brightness, 1.0),
    ] {
        if let Some(m) = materials.get_mut(handle) {
            // (Added over what's behind, so the alpha scales it.)
            m.base_color =
                LinearRgba::new(color[0] * brightness, color[1] * brightness, color[2] * brightness, opacity).into();
        }
    }
}

/// The glow column's look: brightest down its middle, falling off to
/// nothing at its sides, and fading out up high (and just at its foot).
fn glow_column_texture() -> Image {
    const W: usize = 32;
    const H: usize = 128;
    let mut data = Vec::with_capacity(W * H * 4);
    for j in 0..H {
        // (Row 0 is the top.)
        let up = 1.0 - (j as f32 + 0.5) / H as f32;
        let foot = (up / 0.02).min(1.0);
        let top = 1.0 - ((up - 0.25) / 0.75).clamp(0.0, 1.0);
        for i in 0..W {
            let x = ((i as f32 + 0.5) / W as f32) * 2.0 - 1.0;
            let across = (-(x / 0.42).powi(2)).exp() * (1.0 - x.abs()).max(0.0);
            let a = (across * foot * top * top * 255.0).round() as u8;
            data.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: W as u32,
            height: H as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
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
    existing: Query<Entity, Or<(With<BoxArc>, With<BoxPuff>, With<BoxSpeck>)>>,
    puffs: Query<(), With<BoxPuff>>,
    specks: Query<(), With<BoxSpeck>>,
    settings: Res<MysteryBoxFxSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let fx = &*settings;
    let Some((gt, mbox)) = boxes.iter().next() else {
        for e in &existing {
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
    let arc_rate = fx.arc_rate_shut + (fx.arc_rate_open - fx.arc_rate_shut) * open;
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
        let length = state.between(if opened { fx.arc_length_open } else { fx.arc_length_shut });
        spawn_arc(&mut commands, &assets, &mut state, fx, gt, from, dir, length, scale);
    }

    if open <= 0.01 {
        state.puffs_owed = 0.0;
        state.specks_owed = 0.0;
        return;
    }

    // Smoke, rolling up out of the opening and over the sides.
    state.puffs_owed = (state.puffs_owed + dt * fx.smoke_rate * open).min(MAX_BACKLOG);
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
            sideways * fx.smoke_out_speed + Vec3::Y * fx.smoke_rise_speed * state.between((0.6, 1.4)),
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
        let secs = state.between(fx.smoke_secs);
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
    state.specks_owed = (state.specks_owed + dt * fx.speck_rate * open).min(MAX_BACKLOG);
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
        let velocity = drift + Vec3::Y * state.between(fx.speck_rise) * scale;
        let size = state.between(fx.speck_size) * scale;
        let secs = state.between(fx.speck_secs);
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
    fx: &MysteryBoxFxSettings,
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
    let secs = state.between(fx.arc_secs);
    let width = fx.arc_width * scale;
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
    settings: Res<MysteryBoxFxSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let fx = &*settings;
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
            puff.velocity.y -= fx.smoke_sink * dt;
        }
        puff.velocity *= (-fx.smoke_drag * dt).exp();
        let pos = t.translation + puff.velocity * dt;
        t.translation = pos;
        t.scale = Vec3::splat(fx.smoke_size.0.lerp(fx.smoke_size.1, k.sqrt()));
        puff.roll += puff.spin * dt;
        t.rotation = smoke_billboard_rotation(pos, cam_pos, cam_up, cam_right, puff.roll);
        let alpha = fx.smoke_opacity * (k / 0.15).min(1.0) * (1.0 - k).powf(1.5);
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::srgba(0.86, 0.83, 0.76, alpha);
            m.emissive = LinearRgba::gray(fx.smoke_glow * alpha / fx.smoke_opacity);
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

// --- the beacon ---------------------------------------------------------------

/// The beacon over the box: the current shape (world-space strands, each a
/// run of points, with how thick it is) and when it next reshapes. Its
/// pieces are children: [`BeamSegment`]s (core and halo), the glow and the
/// light.
#[derive(Component)]
struct BoxBeam {
    strands: Vec<(Vec<Vec3>, f32)>,
    next_shape: f32,
    flicker: f32,
}

/// One piece of a strand: its index among all of the shape's pieces, and
/// whether it's the halo (or the core).
#[derive(Component)]
struct BeamSegment {
    index: usize,
    halo: bool,
}

#[derive(Component)]
struct BeamGlow;

#[derive(Component)]
struct BeamLight;

/// How many pieces a shape can have (two strands and their forks).
const BEAM_PIECES: usize = 2 * BEAM_SEGMENTS + BEAM_FORKS * BEAM_FORK_SEGMENTS;

/// A fresh shape over the box at `gt`, starting `base` (m, the box as made)
/// over its top: one strand, maybe a second beside it, maybe forks.
fn beam_shape(
    state: &mut BoxFxState,
    fx: &MysteryBoxFxSettings,
    gt: &GlobalTransform,
    base: f32,
) -> Vec<(Vec<Vec3>, f32)> {
    let half = HALF_EXTENTS;
    let top = half.y * 2.0 + base;
    let strand = |state: &mut BoxFxState, foot_local: Vec3| {
        let foot = gt.transform_point(foot_local);
        let mut points = vec![foot];
        for i in 1..=BEAM_SEGMENTS {
            // (Closer together low down.)
            let t = (i as f32 / BEAM_SEGMENTS as f32).powf(1.8);
            let rise = fx.beam_height * t;
            let stray = fx.beam_kink + fx.beam_kink_per_root_m * rise.sqrt();
            let kink = Vec3::new(state.between((-1.0, 1.0)), 0.0, state.between((-1.0, 1.0))) * stray;
            points.push(foot + Vec3::Y * rise + kink);
        }
        points
    };
    let wander = |state: &mut BoxFxState| {
        Vec3::new(
            state.between((-1.0, 1.0)) * fx.beam_wander.0 * half.x,
            top,
            state.between((-1.0, 1.0)) * fx.beam_wander.1 * half.z,
        )
    };
    let first_foot = wander(state);
    let first = strand(state, first_foot);
    let mut strands = Vec::new();
    // Forks: a short jag off the first strand, low down where it shows.
    for _ in 0..BEAM_FORKS {
        if state.next() >= fx.beam_fork_chance {
            continue;
        }
        let at = 2 + (state.next() * (BEAM_SEGMENTS as f32 * 0.45)) as usize;
        let mut p = first[at];
        let step = (first[at + 1] - first[at]).length().max(0.2);
        let out = Vec3::new(state.between((-1.0, 1.0)), 0.0, state.between((-1.0, 1.0))).normalize_or(Vec3::X);
        let mut fork = vec![p];
        for _ in 0..BEAM_FORK_SEGMENTS {
            p += (Vec3::Y * 0.8 + out * 0.5) * step + Vec3::new(state.between((-0.3, 0.3)), 0.0, state.between((-0.3, 0.3))) * step;
            fork.push(p);
        }
        strands.push((fork, 0.6));
    }
    strands.insert(0, (first, 1.0));
    if state.next() < fx.beam_second_chance {
        // Beside the first, a little thinner.
        let foot = first_foot + Vec3::new(state.between((-0.25, 0.25)), 0.0, state.between((-0.15, 0.15)));
        strands.insert(1, (strand(state, foot), 0.75));
    }
    strands
}

/// Keep the beacon over the box: reshape it every so often, lift it as the
/// lid opens, keep each piece a few pixels thick however far away it's seen,
/// and turn its glow to face the camera. Gone the moment there's no box.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_beam(
    time: Res<Time>,
    assets: Option<Res<BoxFxAssets>>,
    mut state: ResMut<BoxFxState>,
    boxes: Query<(&GlobalTransform, &MysteryBox)>,
    camera: Query<&GlobalTransform, With<WorldModelCamera>>,
    mut beams: Query<(Entity, &mut BoxBeam)>,
    mut segments: Query<(&BeamSegment, &mut Transform), (Without<BeamGlow>, Without<BeamLight>)>,
    mut glows: Query<&mut Transform, (With<BeamGlow>, Without<BeamSegment>, Without<BeamLight>)>,
    mut lights: Query<(&mut PointLight, &mut Transform), (With<BeamLight>, Without<BeamSegment>, Without<BeamGlow>)>,
    settings: Res<MysteryBoxFxSettings>,
    mut commands: Commands,
) {
    let fx = &*settings;
    let Some((gt, mbox)) = boxes.iter().next().filter(|_| fx.beam_enabled) else {
        for (e, _) in &beams {
            commands.entity(e).despawn();
        }
        return;
    };
    let Some(assets) = assets else { return };
    let Ok(camera) = camera.single() else { return };
    let eye = camera.translation();
    let base = fx.beam_base_shut + (fx.beam_base_open - fx.beam_base_shut) * mbox.openness();

    let Ok((_, mut beam)) = beams.single_mut() else {
        let strands = beam_shape(&mut state, fx, gt, base);
        commands
            .spawn((
                StateScoped(AppState::InGame),
                BoxBeam {
                    strands,
                    next_shape: 0.0,
                    flicker: 1.0,
                },
                Transform::IDENTITY,
                Visibility::default(),
            ))
            .with_children(|b| {
                for index in 0..BEAM_PIECES {
                    for halo in [false, true] {
                        b.spawn((
                            BeamSegment { index, halo },
                            Mesh3d(assets.segment.clone()),
                            MeshMaterial3d(if halo { assets.beam_halo.clone() } else { assets.beam_core.clone() }),
                            Transform::from_scale(Vec3::ZERO),
                            NotShadowCaster,
                            NoFrustumCulling,
                        ));
                    }
                }
                b.spawn((
                    BeamGlow,
                    Mesh3d(assets.glow_quad.clone()),
                    MeshMaterial3d(assets.beam_glow.clone()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
                let [r, g, b2] = fx.beam_halo_color;
                b.spawn((
                    BeamLight,
                    PointLight {
                        color: Color::srgb(r, g, b2),
                        intensity: 0.0,
                        range: fx.beam_light_range,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::IDENTITY,
                ));
            });
        return;
    };

    // A new shape now and then (held still while the game's paused).
    beam.next_shape -= mbox.frame_secs(time.delta_secs());
    let foot_y = gt.transform_point(Vec3::Y * (HALF_EXTENTS.y * 2.0 + base)).y;
    let lifted = beam.strands.first().is_some_and(|(s, _)| (s[0].y - foot_y).abs() > 0.02);
    if beam.next_shape <= 0.0 || lifted {
        if beam.next_shape <= 0.0 {
            beam.next_shape = state.between(fx.beam_reshape_secs);
            beam.flicker = state.between((0.7, 1.2));
            beam.strands = beam_shape(&mut state, fx, gt, base);
        } else {
            // (Only lifting with the lid: the same shape, moved up.)
            let rise = foot_y - beam.strands[0].0[0].y;
            for (points, _) in &mut beam.strands {
                for p in points.iter_mut() {
                    p.y += rise;
                }
            }
        }
    }

    // Each piece of the shape, as a stretched cylinder a few pixels thick
    // from wherever it's seen.
    let pieces: Vec<(Vec3, Vec3, f32)> = beam
        .strands
        .iter()
        .flat_map(|(points, thick)| points.windows(2).map(move |w| (w[0], w[1], *thick)))
        .take(BEAM_PIECES)
        .collect();
    let flicker = beam.flicker;
    for (segment, mut t) in &mut segments {
        let Some(&(a, b, thick)) = pieces.get(segment.index) else {
            if t.scale != Vec3::ZERO {
                t.scale = Vec3::ZERO;
            }
            continue;
        };
        let d = b - a;
        let len = d.length().max(1e-4);
        let mid = (a + b) * 0.5;
        let width = fx.beam_core_width.max(mid.distance(eye) * fx.beam_width_per_m)
            * thick
            * flicker
            * if segment.halo { fx.beam_halo_width } else { 1.0 };
        *t = Transform {
            translation: mid,
            rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
            scale: Vec3::new(width, len, width),
        };
    }

    // The glow: a column up the first strand, turned to face us.
    let foot = beam.strands[0].0[0];
    let centre = foot + Vec3::Y * fx.beam_height * 0.5;
    let to_eye = eye - centre;
    let facing = Quat::from_rotation_y(to_eye.x.atan2(to_eye.z));
    let glow_width = fx.beam_glow_width.max(foot.distance(eye) * fx.beam_width_per_m * 6.0);
    for mut t in &mut glows {
        *t = Transform {
            translation: centre,
            rotation: facing,
            scale: Vec3::new(glow_width, fx.beam_height, 1.0),
        };
    }
    let [r, g, b] = fx.beam_halo_color;
    for (mut light, mut t) in &mut lights {
        light.intensity = fx.beam_light_intensity * flicker;
        light.range = fx.beam_light_range;
        light.color = Color::srgb(r, g, b);
        t.translation = foot + Vec3::Y * 0.6;
    }
}

// --- debug ----------------------------------------------------------------------

/// The debug panel's "Mystery Box (Zombies)" section: the beacon, the
/// crackles, the smoke and the specks.
#[derive(SystemParam)]
pub(crate) struct MysteryBoxFxDebug<'w> {
    settings: ResMut<'w, MysteryBoxFxSettings>,
}

/// A slider for a random range's low and high ends.
fn range_sliders(ui: &mut egui::Ui, value: &mut (f32, f32), span: std::ops::RangeInclusive<f32>, label: &str) {
    ui.add(egui::Slider::new(&mut value.0, span.clone()).text(format!("{label}, min")));
    ui.add(egui::Slider::new(&mut value.1, span).text(format!("{label}, max")));
}

fn colour(ui: &mut egui::Ui, value: &mut [f32; 3], label: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.color_edit_button_rgb(value);
    });
}

impl MysteryBoxFxDebug<'_> {
    /// Its section in the main debug panel (`debug_ui`).
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        // (Edited on a copy, so the settings only count as changed — and the
        // materials re-tinted — when something really moved.)
        let mut s = (*self.settings).clone();
        ui.label("Effects over a Zombies game's Mystery Box. (Shown wherever the box is.)");
        ui.collapsing("Beacon (blue lightning to the sky)", |ui| {
            ui.checkbox(&mut s.beam_enabled, "On");
            ui.add(egui::Slider::new(&mut s.beam_bolt_opacity, 0.0f32..=1.0).text("bolt opacity (core and halo)"));
            ui.add(egui::Slider::new(&mut s.beam_height, 5.0f32..=400.0).text("height (m)"));
            ui.add(egui::Slider::new(&mut s.beam_base_shut, 0.0f32..=1.0).text("starts over the box, shut (m)"));
            ui.add(egui::Slider::new(&mut s.beam_base_open, 0.0f32..=2.0).text("starts over the box, open (m)"));
            range_sliders(ui, &mut s.beam_reshape_secs, 0.01..=0.5, "reshapes every (s)");
            ui.label("Shape");
            ui.add(egui::Slider::new(&mut s.beam_kink, 0.0f32..=0.5).text("kinks at the base (m)"));
            ui.add(egui::Slider::new(&mut s.beam_kink_per_root_m, 0.0f32..=0.3).text("kinks growing with height"));
            ui.add(egui::Slider::new(&mut s.beam_wander.0, 0.0f32..=1.0).text("wanders along the box"));
            ui.add(egui::Slider::new(&mut s.beam_wander.1, 0.0f32..=1.0).text("wanders across the box"));
            ui.add(egui::Slider::new(&mut s.beam_second_chance, 0.0f32..=1.0).text("chance of a second strand"));
            ui.add(egui::Slider::new(&mut s.beam_fork_chance, 0.0f32..=1.0).text("chance of each fork"));
            ui.label("Core");
            colour(ui, &mut s.beam_core_color, "colour");
            ui.add(egui::Slider::new(&mut s.beam_core_brightness, 0.0f32..=80.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.beam_core_width, 0.002f32..=0.1).text("thickness up close (m)"));
            ui.add(
                egui::Slider::new(&mut s.beam_width_per_m, 0.0f32..=0.005)
                    .text("thickness per metre away (keeps it visible far off)"),
            );
            ui.label("Halo");
            colour(ui, &mut s.beam_halo_color, "colour (and its light's)");
            ui.add(egui::Slider::new(&mut s.beam_halo_brightness, 0.0f32..=10.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.beam_halo_width, 1.0f32..=15.0).text("× the core's thickness"));
            ui.label("Glow column");
            colour(ui, &mut s.beam_glow_color, "colour");
            ui.add(egui::Slider::new(&mut s.beam_glow_brightness, 0.0f32..=4.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.beam_glow_width, 0.1f32..=6.0).text("width (m)"));
            ui.label("Light round the box");
            ui.add(egui::Slider::new(&mut s.beam_light_intensity, 0.0f32..=300_000.0).text("intensity"));
            ui.add(egui::Slider::new(&mut s.beam_light_range, 0.0f32..=30.0).text("range (m)"));
            if ui.button("Reset beacon").clicked() {
                let d = MysteryBoxFxSettings::default();
                s = MysteryBoxFxSettings {
                    beam_enabled: d.beam_enabled,
                    beam_bolt_opacity: d.beam_bolt_opacity,
                    beam_height: d.beam_height,
                    beam_base_shut: d.beam_base_shut,
                    beam_base_open: d.beam_base_open,
                    beam_reshape_secs: d.beam_reshape_secs,
                    beam_kink: d.beam_kink,
                    beam_kink_per_root_m: d.beam_kink_per_root_m,
                    beam_wander: d.beam_wander,
                    beam_second_chance: d.beam_second_chance,
                    beam_fork_chance: d.beam_fork_chance,
                    beam_core_color: d.beam_core_color,
                    beam_core_brightness: d.beam_core_brightness,
                    beam_core_width: d.beam_core_width,
                    beam_width_per_m: d.beam_width_per_m,
                    beam_halo_color: d.beam_halo_color,
                    beam_halo_brightness: d.beam_halo_brightness,
                    beam_halo_width: d.beam_halo_width,
                    beam_glow_color: d.beam_glow_color,
                    beam_glow_brightness: d.beam_glow_brightness,
                    beam_glow_width: d.beam_glow_width,
                    beam_light_intensity: d.beam_light_intensity,
                    beam_light_range: d.beam_light_range,
                    ..s.clone()
                };
            }
        });
        ui.collapsing("Crackles off the box", |ui| {
            colour(ui, &mut s.arc_color, "colour");
            ui.add(egui::Slider::new(&mut s.arc_brightness, 0.0f32..=60.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.arc_rate_shut, 0.0f32..=10.0).text("a second, shut"));
            ui.add(egui::Slider::new(&mut s.arc_rate_open, 0.0f32..=30.0).text("a second, open"));
            range_sliders(ui, &mut s.arc_length_shut, 0.0..=0.6, "length shut (m)");
            range_sliders(ui, &mut s.arc_length_open, 0.0..=1.0, "length open (m)");
            ui.add(egui::Slider::new(&mut s.arc_width, 0.001f32..=0.03).text("thickness (m)"));
            range_sliders(ui, &mut s.arc_secs, 0.01..=0.5, "lasts (s)");
        });
        ui.collapsing("Smoke (open)", |ui| {
            ui.add(egui::Slider::new(&mut s.smoke_rate, 0.0f32..=60.0).text("puffs a second"));
            range_sliders(ui, &mut s.smoke_secs, 0.2..=6.0, "lives (s)");
            range_sliders(ui, &mut s.smoke_size, 0.02..=2.0, "size, leaving / at the end (m)");
            ui.add(egui::Slider::new(&mut s.smoke_opacity, 0.0f32..=1.0).text("opacity"));
            ui.add(egui::Slider::new(&mut s.smoke_glow, 0.0f32..=1.0).text("glow"));
            ui.add(egui::Slider::new(&mut s.smoke_out_speed, 0.0f32..=2.0).text("out speed (m/s)"));
            ui.add(egui::Slider::new(&mut s.smoke_rise_speed, 0.0f32..=2.0).text("rise speed (m/s)"));
            ui.add(egui::Slider::new(&mut s.smoke_drag, 0.0f32..=5.0).text("slows down (per s)"));
            ui.add(egui::Slider::new(&mut s.smoke_sink, 0.0f32..=1.0).text("sinks over the rim (m/s²)"));
        });
        ui.collapsing("Specks of light (open)", |ui| {
            ui.add(egui::Slider::new(&mut s.speck_rate, 0.0f32..=100.0).text("a second"));
            range_sliders(ui, &mut s.speck_secs, 0.1..=5.0, "lives (s)");
            range_sliders(ui, &mut s.speck_size, 0.001..=0.05, "size (m)");
            range_sliders(ui, &mut s.speck_rise, 0.0..=2.0, "rises (m/s)");
        });
        ui.horizontal(|ui| {
            if ui.button("Reset all").clicked() {
                s = MysteryBoxFxSettings::default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("Mystery Box effects: {s:#?}");
            }
        });
        if s != *self.settings {
            *self.settings = s;
        }
    }
}

