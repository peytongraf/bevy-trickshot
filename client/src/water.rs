//! The water a map's layout keeps (`shared::level::Water`, `Waterfall`):
//! pools whose surface ripples, glitters with the torches' light and bends
//! the floor seen through it ([`WaterMaterial`], `water.wgsl` — Bevy's
//! screen-space refraction underneath), and waterfalls pouring into them — a
//! falling sheet that tears into strands as it drops
//! ([`WaterfallMaterial`], `waterfall.wgsl`), and where it lands, mist and
//! droplets thrown up ([`SprayMaterial`], `water_spray.wgsl`, moved by
//! [`animate_spray`]) and rings and foam spreading over the pool under it
//! (the pool's own shader, told where each one lands).
//!
//! In a game each waterfall also roars — `GameSounds::waterfall_loop`,
//! looped from where it lands, each from its own point in the clip so
//! they're not heard in step — and steps taken wading splash
//! (`player::footsteps`, through [`wading`]).
//!
//! Zombies climbing up out of a pool throw it up around them instead of
//! rock and dust ([`spawn_splash`], `vfx::zombie_rise`), and anyone wading,
//! falling or blowing it up stirs it — spray and [`WaterRipples`]
//! (`water_wake`).
//!
//! Nothing here is solid or slows anyone down.
//!
//! In a game they're the current map's ([`sync_game_water`], all
//! `StateScoped(InGame)`); in the level editor its own copy follows the
//! layout being edited (`level_editor`, through [`WaterBuilder::sync`]).
//! Each set is rebuilt whenever its lists change, so nothing carries over
//! between games.

use bevy::ecs::query::QueryFilter;
use bevy::ecs::system::SystemParam;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy::render::view::NoFrustumCulling;
use shared::level::{Water, Waterfall};

use crate::audio_engine::prelude::*;
use crate::util::rand01;
use crate::{AppState, CurrentMap};

const WATER_SHADER_ASSET_PATH: &str = "shaders/water.wgsl";
const WATERFALL_SHADER_ASSET_PATH: &str = "shaders/waterfall.wgsl";
const SPRAY_SHADER_ASSET_PATH: &str = "shaders/water_spray.wgsl";

use params::{SprayParams, WaterParams, WaterfallParams, MAX_IMPACTS, MAX_RIPPLES};

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct (as in `power_ups`).
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Most waterfalls one pool shows landing in it.
    pub(crate) const MAX_IMPACTS: usize = 8;
    /// Most ripples on the water at once (`WaterRipples`).
    pub(crate) const MAX_RIPPLES: usize = 64;

    /// Mirrors `WaterParams` in `water.wgsl`.
    #[derive(Clone, Copy, PartialEq, ShaderType)]
    pub(crate) struct WaterParams {
        pub(crate) impacts: [Vec4; MAX_IMPACTS],
        pub(crate) impact_power: [Vec4; MAX_IMPACTS],
        pub(crate) waves: f32,
        pub(crate) caustics: f32,
        pub(crate) depth: f32,
        pub(crate) impact_count: u32,
        pub(crate) ripples: [Vec4; MAX_RIPPLES],
    }

    // (By hand: arrays this long have no `Default`.)
    impl Default for WaterParams {
        fn default() -> Self {
            Self {
                impacts: [Vec4::ZERO; MAX_IMPACTS],
                impact_power: [Vec4::ZERO; MAX_IMPACTS],
                waves: 0.0,
                caustics: 0.0,
                depth: 0.0,
                impact_count: 0,
                ripples: [Vec4::ZERO; MAX_RIPPLES],
            }
        }
    }

    /// Mirrors `WaterfallParams` in `waterfall.wgsl`.
    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct WaterfallParams {
        pub(crate) side: Vec4,
        pub(crate) fall_time: f32,
        pub(crate) width: f32,
        pub(crate) opacity: f32,
        pub(crate) seed: f32,
    }

    /// Mirrors `SprayParams` in `water_spray.wgsl`.
    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct SprayParams {
        pub(crate) kind: f32,
        pub(crate) opacity: f32,
        pub(crate) _pad0: f32,
        pub(crate) _pad1: f32,
    }
}

/// A pool's surface's ripples, foam and caustics — see `water.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct WaterSurface {
    #[uniform(100)]
    params: WaterParams,
}

impl MaterialExtension for WaterSurface {
    fn fragment_shader() -> ShaderRef {
        WATER_SHADER_ASSET_PATH.into()
    }
}

/// A pool's surface: see-through, refracting PBR, rippled.
pub(crate) type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterSurface>;

/// A waterfall's falling sheet — see `waterfall.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct WaterfallSheet {
    #[uniform(100)]
    params: WaterfallParams,
}

impl MaterialExtension for WaterfallSheet {
    fn fragment_shader() -> ShaderRef {
        WATERFALL_SHADER_ASSET_PATH.into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        WATERFALL_SHADER_ASSET_PATH.into()
    }
}

pub(crate) type WaterfallMaterial = ExtendedMaterial<StandardMaterial, WaterfallSheet>;

/// The mist / droplets where a waterfall lands — see `water_spray.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct SprayPuffs {
    #[uniform(100)]
    params: SprayParams,
}

impl MaterialExtension for SprayPuffs {
    fn vertex_shader() -> ShaderRef {
        SPRAY_SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SPRAY_SHADER_ASSET_PATH.into()
    }
}

pub(crate) type SprayMaterial = ExtendedMaterial<StandardMaterial, SprayPuffs>;

pub(crate) struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        // (None of it casts a shadow or needs a prepass.)
        app.add_plugins((
            MaterialPlugin::<WaterMaterial> {
                prepass_enabled: false,
                shadows_enabled: false,
                ..default()
            },
            MaterialPlugin::<WaterfallMaterial> {
                prepass_enabled: false,
                shadows_enabled: false,
                ..default()
            },
            MaterialPlugin::<SprayMaterial> {
                prepass_enabled: false,
                shadows_enabled: false,
                ..default()
            },
        ))
        .add_systems(Startup, setup_assets)
        // (Ungated: leaving the game is what takes them away.)
        .init_resource::<WaterRipples>()
        // (Nothing of a game's ripples carries into the next.)
        .add_systems(OnExit(AppState::InGame), reset_ripples)
        .add_systems(Update, (sync_game_water, upload_ripples, animate_spray, animate_splashes).chain());
    }
}

/// What every waterfall's spray shares: the quad and the two looks.
#[derive(Resource)]
pub(crate) struct WaterAssets {
    quad: Handle<Mesh>,
    mist: Handle<SprayMaterial>,
    droplets: Handle<SprayMaterial>,
}

fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<SprayMaterial>>) {
    // (White vertex colours: the spray's vertex stage hands each puff's age
    // and seed through them.)
    let quad = Mesh::from(Rectangle::new(1.0, 1.0)).with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 4]);
    let spray = |kind: f32, opacity: f32| SprayMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            perceptual_roughness: 0.5,
            // (Lit through from behind, as a backlit mist is.)
            diffuse_transmission: 0.6,
            ..default()
        },
        extension: SprayPuffs {
            params: SprayParams {
                kind,
                opacity,
                ..default()
            },
        },
    };
    commands.insert_resource(WaterAssets {
        quad: meshes.add(quad),
        mist: materials.add(spray(0.0, 1.0)),
        droplets: materials.add(spray(1.0, 0.9)),
    });
}

/// On each pool's and waterfall's root: its own mesh and material (freed
/// when it goes).
#[derive(Component)]
pub(crate) struct MapWater {
    mesh: Handle<Mesh>,
    water: Option<Handle<WaterMaterial>>,
    fall: Option<Handle<WaterfallMaterial>>,
}

/// On a waterfall's root: where (in its own frame) its spray comes up.
#[derive(Component)]
struct SprayOrigin {
    foot: Vec3,
    half_width: f32,
    spray: f32,
}

/// One puff of mist or splash of droplets, a child of its waterfall's root.
#[derive(Component)]
struct SprayPuff {
    droplets: bool,
    age: f32,
    life: f32,
    vel: Vec3,
    seed: f32,
    /// Bumped each time it's thrown up again, for fresh randomness.
    roll: u32,
}

/// On the game's water (not the editor's).
#[derive(Component)]
struct GameWater;

/// Builds (and rebuilds) a set of pools and waterfalls.
#[derive(SystemParam)]
pub(crate) struct WaterBuilder<'w> {
    assets: Option<Res<'w, WaterAssets>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    waters: ResMut<'w, Assets<WaterMaterial>>,
    falls: ResMut<'w, Assets<WaterfallMaterial>>,
}

/// What a set was last built from.
pub(crate) type ShownWater = (Vec<Water>, Vec<Waterfall>);

/// Whether someone whose eyes are at `eye` (stood up) is wading in any of
/// `map`'s water — so their steps splash.
pub(crate) fn wading(map: shared::MapId, eye: Vec3) -> bool {
    surface_over(map, eye - Vec3::Y * crate::player::EYE_HEIGHT).is_some()
}

/// The surface height of whichever of `map`'s water `feet` (or a floor
/// point) is wading in, if any.
pub(crate) fn surface_over(map: shared::MapId, feet: Vec3) -> Option<f32> {
    shared::level::layout(map)
        .waters
        .iter()
        .find(|w| w.wading(feet))
        .map(|w| w.at.pos.y)
}

/// The surface height of whichever of `map`'s water `p` is in, or just
/// over (up to `above` m) — where something going off there splashes it.
pub(crate) fn surface_near(map: shared::MapId, p: Vec3, above: f32) -> Option<f32> {
    shared::level::layout(map)
        .waters
        .iter()
        .find(|w| w.covers(p) && p.y < w.at.pos.y + above && p.y > w.at.pos.y - shared::level::WADE_DEPTH)
        .map(|w| w.at.pos.y)
}

/// Most small splash particles out at once (past it, new ones are
/// skipped — a [`spawn_big_splash`] always goes up whole).
const SPLASH_MAX: usize = 220;

/// One drop or puff of a splash, flying free until it's gone — a droplet
/// as soon as it drops back under the surface.
#[derive(Component)]
pub(crate) struct SplashParticle {
    droplets: bool,
    age: f32,
    life: f32,
    vel: Vec3,
    seed: f32,
    /// How big it is (1 = a waterfall's spray's).
    size: f32,
    /// The surface it was thrown from.
    surface: f32,
}

/// Throw one splash particle from `at`.
#[allow(clippy::too_many_arguments)]
fn throw(commands: &mut Commands, assets: &WaterAssets, at: Vec3, droplets: bool, vel: Vec3, life: f32, size: f32, seed: f32) {
    commands.spawn((
        SplashParticle {
            droplets,
            age: 0.0,
            life,
            vel,
            seed,
            size,
            surface: at.y,
        },
        StateScoped(AppState::InGame),
        Mesh3d(assets.quad.clone()),
        MeshMaterial3d(if droplets { assets.droplets.clone() } else { assets.mist.clone() }),
        Transform::from_translation(at + Vec3::Y * 0.03).with_scale(Vec3::splat(0.0001)),
        NotShadowCaster,
        NoFrustumCulling,
    ));
}

/// How long a droplet thrown up at `vy` is in the air (and a little over,
/// so it's still there when it drops back in).
fn airtime(vy: f32) -> f32 {
    (2.0 * vy.max(0.5) / 9.81) * 1.6 + 0.1
}

/// Throw water up from `at` (on a surface): a few droplets flung up and
/// out, and maybe a puff of mist — the same looks as a waterfall's spray.
/// `power` (≈0.3..1) is how hard; `seed` varies it; `push` carries it
/// along (someone wading kicks it forward).
/// Returns how many particles it threw.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_splash(commands: &mut Commands, assets: &WaterAssets, existing: usize, at: Vec3, power: f32, push: Vec3, seed: u32) -> usize {
    let r = |k: u32| rand01(seed.wrapping_mul(2_654_435_761) ^ k);
    let drops = 1 + (r(1) * 3.0 * power).round() as u32;
    let mist = u32::from(r(2) < 0.25 + 0.4 * power);
    let count = (drops + mist).min(SPLASH_MAX.saturating_sub(existing) as u32);
    for k in 0..count {
        let droplets = k < drops;
        let s = |j: u32| r(k.wrapping_mul(31) + j * 7 + 10);
        let angle = s(1) * core::f32::consts::TAU;
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let size = 0.6 + 0.4 * power;
        if droplets {
            let vel = out * (0.4 + s(2) * 1.2) * power + Vec3::Y * (1.4 + s(3) * 2.2) * power + push;
            throw(commands, assets, at, true, vel, airtime(vel.y), size, s(5));
        } else {
            let vel = out * (0.2 + s(2) * 0.4) + Vec3::Y * (0.3 + s(3) * 0.5) * power + push * 0.4;
            throw(commands, assets, at, false, vel, puff_life(false, s(4)) * 0.6, size, s(5));
        }
    }
    count as usize
}

/// A blast going off in the water: a tall column of it thrown up and
/// raining back down, a wide skirt of spray out low, a cloud of mist, and
/// heavy rings running out across the pool. `size` 1 is a grenade's.
pub(crate) fn spawn_big_splash(commands: &mut Commands, assets: &WaterAssets, ripples: &mut WaterRipples, now: f32, at: Vec3, size: f32, seed: u32) {
    let size = size.clamp(0.3, 3.0);
    let k = size.sqrt();
    let r = |j: u32| rand01(seed.wrapping_mul(2_654_435_761) ^ j);
    let mut n = 0u32;
    let mut next = || {
        n += 1;
        let base = n.wrapping_mul(97);
        move |j: u32| r(base + j)
    };
    // The column: thrown nearly straight up, hard.
    for _ in 0..(36.0 * size) as u32 {
        let s = next();
        let angle = s(1) * core::f32::consts::TAU;
        let spread = s(2).powi(2) * 1.6;
        let vel = Vec3::new(angle.cos() * spread, 5.0 + s(3) * 5.0, angle.sin() * spread) * k;
        let from = at + Vec3::new(angle.cos(), 0.0, angle.sin()) * s(4) * 0.5 * k;
        throw(commands, assets, from, true, vel, airtime(vel.y), (1.3 + s(5)) * k, s(6));
    }
    // The skirt: out low and wide.
    for _ in 0..(28.0 * size) as u32 {
        let s = next();
        let angle = s(1) * core::f32::consts::TAU;
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let vel = (out * (2.0 + s(2) * 3.5) + Vec3::Y * (1.5 + s(3) * 2.5)) * k;
        throw(commands, assets, at + out * 0.3 * k, true, vel, airtime(vel.y), (1.0 + s(5) * 0.8) * k, s(6));
    }
    // Mist hanging over it all.
    for _ in 0..(12.0 * size) as u32 {
        let s = next();
        let angle = s(1) * core::f32::consts::TAU;
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let vel = (out * (0.6 + s(2) * 1.2) + Vec3::Y * (1.0 + s(3) * 2.5)) * k;
        throw(commands, assets, at + out * s(4) * 0.8 * k, false, vel, 2.0 + s(5) * 1.5, (2.0 + s(6) * 1.5) * k, s(7));
    }
    // Rings: one after another out of the middle, and churn around it.
    for i in 0..4 {
        ripples.add(at, now + i as f32 * 0.18, (3.0 - i as f32 * 0.5) * k);
    }
    for i in 0..6 {
        let angle = i as f32 / 6.0 * core::f32::consts::TAU + r(900);
        ripples.add(at + Vec3::new(angle.cos(), 0.0, angle.sin()) * 0.9 * k, now + 0.3, 1.2 * k);
    }
}

/// Fly each splash particle — droplets fall back, mist slows and spreads —
/// and clear it away once it's spent, or a droplet's back in the water.
fn animate_splashes(time: Res<Time>, mut particles: Query<(Entity, &mut SplashParticle, &mut Transform)>, mut commands: Commands) {
    let dt = time.delta_secs().min(0.1);
    for (entity, mut p, mut t) in &mut particles {
        p.age += dt;
        let sunk = p.droplets && p.vel.y < 0.0 && t.translation.y < p.surface - 0.05;
        if p.age >= p.life || sunk {
            commands.entity(entity).despawn();
            continue;
        }
        if p.droplets {
            p.vel.y -= 9.81 * dt;
            // (A little drag, so a hard throw doesn't go on for ever.)
            p.vel *= (-0.4 * dt).exp();
        } else {
            p.vel *= (-1.6 * dt).exp();
        }
        t.translation += p.vel * dt;
        let f = (p.age / p.life).clamp(0.0, 1.0);
        let size = if p.droplets {
            (0.12 + 0.2 * f.sqrt()) * p.size
        } else {
            (0.25 + 0.7 * f.sqrt()) * p.size
        };
        // (Its age and seed ride in its scale — `water_spray.wgsl`.)
        t.scale = Vec3::new(size, size * (1.0 + p.seed), size * (1.0 + f));
    }
}

/// Rings running out across the water from wherever it was disturbed —
/// someone wading, falling in, a blast — the newest [`MAX_RIPPLES`] of
/// them, handed to every pool's shader (`water.wgsl`, `upload_ripples`).
/// Each is (x, z, when it started — `Time::elapsed_secs_wrapped`, the
/// shader's `globals.time` — and how strong).
#[derive(Resource)]
pub(crate) struct WaterRipples {
    list: [Vec4; MAX_RIPPLES],
    next: usize,
}

impl Default for WaterRipples {
    fn default() -> Self {
        Self {
            list: [Vec4::ZERO; MAX_RIPPLES],
            next: 0,
        }
    }
}

impl WaterRipples {
    /// A ring from `at`, starting at `start` (which can be a moment off).
    pub(crate) fn add(&mut self, at: Vec3, start: f32, strength: f32) {
        self.list[self.next] = Vec4::new(at.x, at.z, start, strength.max(0.0));
        self.next = (self.next + 1) % MAX_RIPPLES;
    }
}

/// Hand the ripples to every pool in the game, whenever there are new ones.
fn upload_ripples(
    ripples: Res<WaterRipples>,
    pools: Query<&MapWater, With<GameWater>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
) {
    if !ripples.is_changed() {
        return;
    }
    for pool in &pools {
        if let Some(material) = pool.water.as_ref().and_then(|h| materials.get_mut(h)) {
            material.extension.params.ripples = ripples.list;
        }
    }
}

fn reset_ripples(mut ripples: ResMut<WaterRipples>) {
    *ripples = WaterRipples::default();
}

/// Keep the current map's water while in a game.
fn sync_game_water(
    state: Res<State<AppState>>,
    current: Res<CurrentMap>,
    existing: Query<(Entity, &MapWater), With<GameWater>>,
    mut builder: WaterBuilder,
    mut commands: Commands,
    mut shown: Local<ShownWater>,
    sounds: Option<Res<crate::GameSounds>>,
) {
    let (waters, falls): (&[Water], &[Waterfall]) = if *state.get() == AppState::InGame {
        let layout = shared::level::layout(current.0);
        (&layout.waters, &layout.waterfalls)
    } else {
        (&[], &[])
    };
    let roar = sounds.as_ref().map(|s| &s.waterfall_loop);
    builder.sync(waters, falls, &mut shown, &existing, roar, &mut commands, || {
        (StateScoped(AppState::InGame), GameWater)
    });
}

impl WaterBuilder<'_> {
    /// Make the water spawned with `marker` match `waters` and `falls`: all
    /// rebuilt whenever they differ from what was last built (`shown`) or
    /// some have gone (left the state they were scoped to). With a `roar`,
    /// each waterfall loops it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync<F: QueryFilter, B: Bundle>(
        &mut self,
        waters: &[Water],
        falls: &[Waterfall],
        shown: &mut ShownWater,
        existing: &Query<(Entity, &MapWater), F>,
        roar: Option<&Handle<AudioSource>>,
        commands: &mut Commands,
        marker: impl Fn() -> B,
    ) {
        if shown.0.as_slice() == waters && shown.1.as_slice() == falls && existing.iter().count() == waters.len() + falls.len() {
            return;
        }
        let Some(assets) = self.assets.as_deref() else { return };
        for (e, old) in existing {
            self.meshes.remove(&old.mesh);
            if let Some(m) = &old.water {
                self.waters.remove(m);
            }
            if let Some(m) = &old.fall {
                self.falls.remove(m);
            }
            commands.entity(e).despawn();
        }
        *shown = (waters.to_vec(), falls.to_vec());

        for water in waters {
            let mesh = self.meshes.add(Plane3d::new(Vec3::Y, water.size * 0.5));
            let material = self.waters.add(water_material(water, falls));
            commands.spawn((
                marker(),
                MapWater {
                    mesh: mesh.clone(),
                    water: Some(material.clone()),
                    fall: None,
                },
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(water.at.pos).with_rotation(water.at.rotation()),
                NotShadowCaster,
            ));
        }

        for (i, fall) in falls.iter().enumerate() {
            let seed = rand01((i as u32).wrapping_mul(2_654_435_761) ^ fall.at.pos.x.to_bits() ^ fall.at.pos.z.to_bits());
            let mesh = self.meshes.add(sheet_mesh(fall));
            let material = self.falls.add(WaterfallMaterial {
                base: StandardMaterial {
                    base_color: Color::WHITE,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    perceptual_roughness: 0.3,
                    reflectance: 0.35,
                    diffuse_transmission: 0.4,
                    ..default()
                },
                extension: WaterfallSheet {
                    params: WaterfallParams {
                        side: (fall.rotation() * Vec3::X).extend(0.0),
                        fall_time: fall.fall_time().max(0.01),
                        width: fall.width,
                        opacity: fall.opacity,
                        seed: seed * 100.0,
                    },
                },
            });
            let foot = fall.local_at(fall.fall_time());
            let half_width = fall.width * 0.5 * 1.25;
            commands
                .spawn((
                    marker(),
                    MapWater {
                        mesh: mesh.clone(),
                        water: None,
                        fall: Some(material.clone()),
                    },
                    SprayOrigin {
                        foot,
                        half_width,
                        spray: fall.spray,
                    },
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::from_translation(fall.at.pos).with_rotation(fall.rotation()),
                    NotShadowCaster,
                ))
                .with_children(|root| {
                    if let Some(roar) = roar {
                        root.spawn((
                            AudioPlayer::new(roar.clone()),
                            Transform::from_translation(foot),
                            // (Somewhere different in the clip for each.)
                            PlaybackSettings::LOOP
                                .with_spatial(true)
                                .with_start(rand01((seed * 1e6) as u32 ^ 0x2545_f491)),
                        ));
                    }
                    if fall.spray <= 0.0 {
                        return;
                    }
                    let width = fall.width.max(0.2);
                    let mist = (4.0 + width * 5.0 * fall.spray).round().clamp(4.0, 28.0) as u32;
                    let droplets = (3.0 + width * 4.0 * fall.spray).round().clamp(3.0, 20.0) as u32;
                    for k in 0..mist + droplets {
                        let droplets = k >= mist;
                        let puff_seed = rand01(k.wrapping_mul(7_919) ^ (seed * 1e6) as u32);
                        let life = puff_life(droplets, puff_seed);
                        root.spawn((
                            SprayPuff {
                                droplets,
                                // (Waiting a while for its first throw, so
                                // they don't all go up at once.)
                                age: -life * rand01(k ^ 0x5bd1_e995),
                                life,
                                vel: Vec3::ZERO,
                                seed: puff_seed,
                                roll: k.wrapping_mul(97),
                            },
                            Mesh3d(assets.quad.clone()),
                            MeshMaterial3d(if droplets { assets.droplets.clone() } else { assets.mist.clone() }),
                            // (Off where it can't be seen until it's first
                            // thrown up — `animate_spray`.)
                            Transform::from_translation(foot).with_scale(Vec3::splat(0.0001)),
                            NotShadowCaster,
                            // (Billboarded well outside the quad's own
                            // bounds.)
                            NoFrustumCulling,
                        ));
                    }
                });
        }
    }
}

/// `water`'s material: told where each of `falls` lands in it.
fn water_material(water: &Water, falls: &[Waterfall]) -> WaterMaterial {
    let mut params = WaterParams {
        waves: water.waves,
        caustics: water.caustics,
        depth: water.depth,
        ..default()
    };
    let landing = falls.iter().filter_map(|f| {
        let foot = f.foot();
        // (Landing on it — not on something above or below it.)
        (water.covers(foot) && (foot.y - water.at.pos.y).abs() < 1.0).then_some(f)
    });
    for (i, fall) in landing.take(MAX_IMPACTS).enumerate() {
        let foot = fall.foot();
        params.impacts[i] = Vec4::new(foot.x, water.at.pos.y, foot.z, fall.width * 0.5 + 0.3);
        params.impact_power[i] = Vec4::new(fall.spray, 0.0, 0.0, 0.0);
        params.impact_count = i as u32 + 1;
    }
    WaterMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            // (Mirror-smooth, so what's under it comes through crisp — the
            // ripples are the shader's.)
            perceptual_roughness: 0.0,
            // (Water reflects about 2% head on.)
            reflectance: 0.35,
            specular_transmission: 1.0,
            ior: 1.33,
            // (Overridden per pixel by `water.wgsl`, which tints by depth
            // itself — see there.)
            thickness: water.depth.max(0.01),
            attenuation_distance: water.clarity.max(0.01),
            attenuation_color: Color::srgb(water.color[0], water.color[1], water.color[2]),
            ..default()
        },
        extension: WaterSurface { params },
    }
}

/// Rows down a waterfall's sheet, and across it.
const SHEET_ROWS: usize = 28;
const SHEET_COLS: usize = 6;

/// A waterfall's sheet in its own frame: following the water's arc off the
/// lip, widening a little as it falls and bowed out slightly across. Its uv
/// is (across, how far through its fall by time).
fn sheet_mesh(fall: &Waterfall) -> Mesh {
    let total = fall.fall_time();
    let mut positions = Vec::with_capacity(SHEET_ROWS * SHEET_COLS);
    let mut normals = Vec::with_capacity(SHEET_ROWS * SHEET_COLS);
    let mut uvs = Vec::with_capacity(SHEET_ROWS * SHEET_COLS);
    for row in 0..SHEET_ROWS {
        let v = row as f32 / (SHEET_ROWS - 1) as f32;
        let t = v * total;
        let center = fall.local_at(t);
        // Square to the way it's going (down and out), facing out.
        let normal = Vec3::new(0.0, fall.flow, 9.81 * t).normalize_or(Vec3::Z);
        let width = fall.width * (1.0 + 0.25 * v);
        for col in 0..SHEET_COLS {
            let u = col as f32 / (SHEET_COLS - 1) as f32;
            let across = u * 2.0 - 1.0;
            let bow = 0.04 * fall.width.min(2.0) * (1.0 - across * across);
            positions.push((center + Vec3::X * across * width * 0.5 + normal * bow).to_array());
            normals.push(normal.to_array());
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity((SHEET_ROWS - 1) * (SHEET_COLS - 1) * 6);
    for row in 0..SHEET_ROWS - 1 {
        for col in 0..SHEET_COLS - 1 {
            let a = (row * SHEET_COLS + col) as u32;
            let b = a + 1;
            let c = a + SHEET_COLS as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// How long (s) a puff lasts.
fn puff_life(droplets: bool, seed: f32) -> f32 {
    if droplets {
        0.45 + seed * 0.45
    } else {
        1.6 + seed * 1.2
    }
}

/// Throw each waterfall's spray up where it lands, again and again: mist
/// billowing up and out, slowing as it spreads; droplets flung out and
/// falling back. Each puff's size, age and seed ride in its scale (see
/// `water_spray.wgsl`).
fn animate_spray(
    time: Res<Time>,
    origins: Query<&SprayOrigin>,
    mut puffs: Query<(&mut SprayPuff, &mut Transform, &ChildOf)>,
) {
    let dt = time.delta_secs().min(0.1);
    for (mut puff, mut t, child_of) in &mut puffs {
        let Ok(origin) = origins.get(child_of.parent()) else { continue };
        let waiting = puff.age < 0.0;
        puff.age += dt;
        if puff.age < 0.0 {
            continue;
        }
        if waiting || puff.age >= puff.life {
            // Thrown up again, somewhere along where it lands.
            puff.roll = puff.roll.wrapping_add(1);
            let salt = puff.roll.wrapping_mul(2_246_822_519) ^ (puff.seed * 1e6) as u32;
            let r = |k: u32| rand01(salt ^ k);
            puff.age = 0.0;
            puff.life = puff_life(puff.droplets, r(1));
            let strength = origin.spray.sqrt();
            puff.vel = if puff.droplets {
                Vec3::new((r(2) - 0.5) * 2.4, 1.4 + r(3) * 2.2, (r(4) - 0.25) * 2.2) * strength
            } else {
                Vec3::new((r(2) - 0.5) * 0.8, 0.35 + r(3) * 0.8, (r(4) - 0.2) * 1.0) * strength
            };
            let along = (r(5) * 2.0 - 1.0) * origin.half_width;
            t.translation = origin.foot + Vec3::new(along, 0.05, (r(6) - 0.5) * 0.3);
        }
        if puff.droplets {
            puff.vel.y -= 9.81 * dt;
        } else {
            // (Mist slows as it spreads, but keeps rising a little.)
            puff.vel *= (-1.3 * dt).exp();
            puff.vel.y += 0.15 * dt;
        }
        let step = puff.vel * dt;
        t.translation += step;
        let f = (puff.age / puff.life).clamp(0.0, 1.0);
        let size = if puff.droplets {
            (0.18 + 0.25 * f.sqrt()) * (0.6 + 0.4 * origin.spray.min(2.0))
        } else {
            (0.35 + 1.25 * f.sqrt()) * (0.5 + 0.5 * origin.spray.min(2.0))
        };
        // (Its age and seed ride in its scale — `water_spray.wgsl`.)
        t.scale = Vec3::new(size, size * (1.0 + puff.seed), size * (1.0 + f));
    }
}
