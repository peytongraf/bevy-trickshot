//! The sniper scope's look while aimed, after Call of Duty's: the world
//! around the lens softly blurred, so the eye goes to the picture in it, and
//! the picture itself darkening toward the rim, with a faint warm-to-cool
//! fringe along the bottom of its edge and a pale sheen round the rest, the
//! way real glass shows.
//!
//! * The blur ([`apply_scope_blur`]): a depth-of-field blur on the *world*
//!   camera, focused so close that everything's out of it. The picture in
//!   the lens is the scope camera's, drawn over the world by the gun's
//!   camera — so only the world *around* the lens blurs. It comes in with
//!   the scope picture (`weapons::ads::scope_picture_amount`), and is a
//!   player setting (`Settings::scope_blur`).
//! * The vignette and fringe ([`ScopeVignette`]): a ring texture
//!   ([`vignette_image`]) on a quad in front of the reticle's camera, so it's
//!   drawn into the scope picture over the reticle — rebuilt whenever its
//!   tuning changes.
//!
//! Nothing to reset between games: the blur's set afresh every frame, and
//! the vignette rides on the scope camera.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::dof::{DepthOfField, DepthOfFieldMode};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::RenderLayers;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;

use crate::player::WorldModelCamera;
use crate::settings::Settings;
use crate::weapons::{scope_picture_amount, Ads, AdsTuning, ScopeCamera};
use crate::AppState;

/// Tunables — the debug panel's "Scope look" section.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct ScopeFxSettings {
    /// How blurred (px across, at 1080p — scaled to the screen) the world
    /// around the lens gets, fully scoped.
    pub(crate) outside_blur_px: f32,
    /// Where the lens's visible edge is in the scope picture (`1` = the
    /// picture's inscribed circle) — the vignette and fringe go by it.
    pub(crate) rim_radius: f32,
    /// The vignette: how far out (× the rim radius) it starts darkening, and
    /// how dark it gets at the rim (`0..1`).
    pub(crate) vignette_start: f32,
    pub(crate) vignette_strength: f32,
    /// The fringe: how strong (`0..1`) and how wide (× the rim radius) the
    /// coloured band just inside the rim is — orange where it's strongest,
    /// going yellow then teal toward its ends...
    pub(crate) fringe_strength: f32,
    pub(crate) fringe_width: f32,
    /// ...where round the rim it's centred (°, `0` the bottom, clockwise),
    /// and how far (°) round either way it reaches.
    pub(crate) fringe_angle_deg: f32,
    pub(crate) fringe_spread_deg: f32,
    /// The pale sheen on the rest of the rim (`0..1`).
    pub(crate) rim_sheen: f32,
}

impl Default for ScopeFxSettings {
    fn default() -> Self {
        Self {
            outside_blur_px: 10.0,
            rim_radius: 1.0,
            vignette_start: 0.72,
            vignette_strength: 0.65,
            fringe_strength: 0.45,
            fringe_width: 0.05,
            fringe_angle_deg: 0.0,
            fringe_spread_deg: 65.0,
            rim_sheen: 0.12,
        }
    }
}

/// The vignette quad (a child of the scope camera — see [`attach_vignette`]).
#[derive(Component)]
pub(crate) struct ScopeVignette;

/// On a scope camera that has its [`ScopeVignette`].
#[derive(Component)]
struct HasVignette;

/// The vignette's texture and material, made once.
#[derive(Resource)]
struct ScopeFxAssets {
    image: Handle<Image>,
    material: Handle<StandardMaterial>,
    mesh: Handle<Mesh>,
}

/// How far (m) in front of the scope camera the vignette quad sits — nearer
/// than the reticle (`weapons::scope::RETICLE_DIST`), so it's drawn over it.
const VIGNETTE_DIST: f32 = 0.15;

/// The vignette's texture size (px, square).
const VIGNETTE_PX: u32 = 256;

pub(crate) struct ScopeFxPlugin;

impl Plugin for ScopeFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScopeFxSettings>()
            .add_systems(Startup, make_assets)
            .add_systems(Update, (attach_vignette, rebuild_vignette))
            .add_systems(
                PostUpdate,
                (
                    size_vignette.before(bevy::render::camera::CameraUpdateSystem),
                    apply_scope_blur.run_if(in_state(AppState::InGame)),
                ),
            );
    }
}

/// The vignette ring for `cfg`: transparent in the middle, darkening toward
/// the rim, a coloured band just inside it along the bottom (orange at its
/// heart, yellow then teal toward its ends) and a pale sheen round the rest,
/// and black past it.
fn vignette_image(cfg: &ScopeFxSettings) -> Image {
    let n = VIGNETTE_PX as usize;
    let mut data = Vec::with_capacity(n * n * 4);
    let rim = cfg.rim_radius.max(0.05);
    for y in 0..n {
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let r = (u * u + v * v).sqrt() / rim;
            let s = |a: f32, b: f32, x: f32| {
                let t = ((x - a) / (b - a).max(1e-4)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let dark = if r >= 1.0 { 1.0 } else { s(cfg.vignette_start, 1.0, r) * cfg.vignette_strength };
            let width = cfg.fringe_width.max(1e-3);
            let ring = if r >= 1.0 { 0.0 } else { (-((r - (1.0 - width)) / (width * 0.5)).powi(2)).exp() };
            // How far round from the fringe's heart (`v` is down the picture,
            // so `0` is straight down), as a share of its reach either way.
            let from_bottom = u.atan2(v).to_degrees() - cfg.fringe_angle_deg;
            let from_bottom = (from_bottom + 180.0).rem_euclid(360.0) - 180.0;
            let along = (from_bottom.abs() / cfg.fringe_spread_deg.max(1.0)).min(1.0);
            let warm = Vec3::new(1.0, 0.55, 0.18);
            let mid = Vec3::new(0.85, 0.85, 0.3);
            let cool = Vec3::new(0.25, 0.7, 0.95);
            let tint = if along < 0.5 { warm.lerp(mid, along * 2.0) } else { mid.lerp(cool, along * 2.0 - 1.0) };
            let coloured = ring * cfg.fringe_strength * (1.0 - along * along);
            let sheen = ring * cfg.rim_sheen * along;
            let band = (coloured + sheen).min(1.0);
            let colour = if band > 1e-5 { (tint * coloured + Vec3::splat(0.85) * sheen) / band } else { Vec3::ZERO };
            let fringe = Srgba::new(colour.x, colour.y, colour.z, 1.0);
            // The fringe over the darkening.
            let a = band + dark * (1.0 - band);
            let (cr, cg, cb) = if a > 1e-4 {
                (fringe.red * band / a, fringe.green * band / a, fringe.blue * band / a)
            } else {
                (0.0, 0.0, 0.0)
            };
            let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[byte(cr), byte(cg), byte(cb), byte(a)]);
        }
    }
    Image::new(
        Extent3d {
            width: VIGNETTE_PX,
            height: VIGNETTE_PX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

fn make_assets(
    mut commands: Commands,
    settings: Res<ScopeFxSettings>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let image = images.add(vignette_image(&settings));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(image.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.insert_resource(ScopeFxAssets {
        image,
        material,
        mesh: meshes.add(Rectangle::new(1.0, 1.0)),
    });
}

/// Give each scope camera its vignette quad, on the reticle's layer (so only
/// the reticle's camera draws it).
fn attach_vignette(
    mut commands: Commands,
    assets: Option<Res<ScopeFxAssets>>,
    cameras: Query<Entity, (With<ScopeCamera>, Without<HasVignette>)>,
) {
    let Some(assets) = assets else { return };
    for camera in &cameras {
        commands.entity(camera).insert(HasVignette).with_child((
            ScopeVignette,
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            Transform::from_xyz(0.0, 0.0, -VIGNETTE_DIST),
            RenderLayers::layer(crate::SCOPE_OVERLAY_LAYER),
        ));
    }
}

/// Redraw the vignette's texture when its tuning changes.
fn rebuild_vignette(settings: Res<ScopeFxSettings>, assets: Option<Res<ScopeFxAssets>>, mut images: ResMut<Assets<Image>>) {
    let Some(assets) = assets else { return };
    if settings.is_changed() && !settings.is_added() {
        images.insert(assets.image.id(), vignette_image(&settings));
    }
}

/// Fill the scope camera's (square) view with the vignette, at its FOV now.
fn size_vignette(
    cameras: Query<(&Projection, &Children), With<ScopeCamera>>,
    mut vignettes: Query<&mut Transform, With<ScopeVignette>>,
) {
    for (projection, children) in &cameras {
        let Projection::Perspective(p) = projection else { continue };
        let fill = 2.0 * VIGNETTE_DIST * (p.fov * 0.5).tan();
        for child in children.iter() {
            if let Ok(mut tf) = vignettes.get_mut(child) {
                let want = Transform::from_xyz(0.0, 0.0, -VIGNETTE_DIST).with_scale(Vec3::new(fill, fill, 1.0));
                if *tf != want {
                    *tf = want;
                }
            }
        }
    }
}

/// Blur the world around the lens as the scope picture comes in: a
/// depth-of-field blur focused right in front of the eye (everything past it
/// as soft as `outside_blur_px` lets it go). Off — taken off the camera —
/// at the hip, with no scope up, or with the setting off.
#[allow(clippy::too_many_arguments)]
fn apply_scope_blur(
    settings: Res<Settings>,
    fx: Res<ScopeFxSettings>,
    ads: Res<Ads>,
    tuning: Res<AdsTuning>,
    window: Query<&Window, With<PrimaryWindow>>,
    scope: Query<&Camera, With<ScopeCamera>>,
    mut world: Query<(Entity, Option<&mut DepthOfField>), With<WorldModelCamera>>,
    mut commands: Commands,
) {
    let Ok((camera, dof)) = world.single_mut() else { return };
    let scoped = scope.iter().any(|c| c.is_active);
    let amount = if settings.scope_blur && scoped { scope_picture_amount(ads.t, &tuning) } else { 0.0 };
    let height = window.single().map_or(1080.0, |w| w.physical_height() as f32);
    let px = fx.outside_blur_px * amount * height / 1080.0;
    if px < 0.5 {
        if dof.is_some() {
            commands.entity(camera).remove::<DepthOfField>();
        }
        return;
    }
    match dof {
        Some(mut dof) => {
            if dof.max_circle_of_confusion_diameter != px {
                dof.max_circle_of_confusion_diameter = px;
            }
        }
        None => {
            commands.entity(camera).insert(DepthOfField {
                mode: DepthOfFieldMode::Gaussian,
                // (Nearer than anything the world camera draws, with a wide
                // aperture: all of it's out of focus, as blurred as `px` lets.)
                focal_distance: 0.3,
                aperture_f_stops: 0.5,
                max_circle_of_confusion_diameter: px,
                ..default()
            });
        }
    }
}

/// The debug panel's "Scope look" section.
#[derive(SystemParam)]
pub(crate) struct ScopeFxDebug<'w> {
    settings: ResMut<'w, ScopeFxSettings>,
}

impl ScopeFxDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label("While scoped: the world around the lens blurred (Settings → Graphics → Scope blur), and the picture darkening to the rim with a coloured fringe along its bottom.");
        // (Edited on a copy, so the texture's only rebuilt when something moved.)
        let mut s = (*self.settings).clone();
        ui.add(egui::Slider::new(&mut s.outside_blur_px, 0.0f32..=40.0).text("blur around the lens (px at 1080p)"));
        ui.add(egui::Slider::new(&mut s.rim_radius, 0.5f32..=1.5).text("lens rim (× the picture's circle)"));
        ui.add(egui::Slider::new(&mut s.vignette_start, 0.0f32..=1.0).text("vignette starts (× rim)"));
        ui.add(egui::Slider::new(&mut s.vignette_strength, 0.0f32..=1.0).text("vignette darkness at the rim"));
        ui.add(egui::Slider::new(&mut s.fringe_strength, 0.0f32..=1.0).text("fringe strength"));
        ui.add(egui::Slider::new(&mut s.fringe_width, 0.005f32..=0.3).text("fringe width (× rim)"));
        ui.add(egui::Slider::new(&mut s.fringe_angle_deg, -180.0f32..=180.0).text("fringe centred at (°, 0 = bottom)"));
        ui.add(egui::Slider::new(&mut s.fringe_spread_deg, 5.0f32..=180.0).text("fringe reaches round (° each way)"));
        ui.add(egui::Slider::new(&mut s.rim_sheen, 0.0f32..=1.0).text("pale sheen round the rest"));
        ui.horizontal(|ui| {
            if ui.button("Reset scope look").clicked() {
                s = ScopeFxSettings::default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("{s:#?}");
            }
        });
        if s != *self.settings {
            *self.settings = s;
        }
    }
}
