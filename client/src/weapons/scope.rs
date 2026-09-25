//! The render-to-texture scope: its camera, reticle and rear lens, and the
//! system that drives the aim-in slide + sight-picture fade.

use bevy::pbr::DistanceFog;
use bevy::prelude::*;

use crate::killcam::ActiveKillCam;
use crate::player::WorldModelCamera;
use crate::settings::{CrosshairId, Settings};
use crate::util::ads_ease;

use super::ads::{
    ads_fov_rad, full_scope_fov_rad, scope_picture_amount, Ads, AdsTuning, Optic,
};
use super::sway::{AimSwayState, WeaponSwayState};

// --- scope (render-to-texture) -------------------------------------------------
// A second camera renders the world only (no view model) through a narrow FOV
// into `SCOPE_RT_SIZE`² and that image is shown on the scope's rear lens, so the
// player looks *through* the scope instead of down the tube. The scope camera is
// only active while `ads.t > 0`, so the extra pass costs nothing at the hip.
/// Render-target resolution for the scope view.
pub(crate) const SCOPE_RT_SIZE: u32 = 512;
/// Distance (metres) the reticle quad sits in front of the scope camera.
pub(crate) const RETICLE_DIST: f32 = 0.2;
/// ADS amount below which the scope camera is switched off, so no stale frame
/// is rendered at the hip. The lens itself stays visible either way — see below.
pub(crate) const SCOPE_SHOW_AT: f32 = 0.02;

// The rear lens is one lit `StandardMaterial`. Off-aim it reads as a smooth,
// strongly reflective coated-glass disc catching the sun; as `Ads::t` → 1 the
// mirror sheen is dialled out and the render-to-texture sight picture (carried
// on `emissive`) fades in, so glare can't wash out the shot.
/// Lens surface roughness while fully scoped (fully matte — with `reflectance`
/// at 0 there's no sun glint on the glass once aimed in).
pub(crate) const LENS_ROUGHNESS_ADS: f32 = 1.0;

/// Panel-adjustable look of the lens glass at the hip. As `Ads::t` → 1 the tint
/// fades to a black backing and metallic / reflectance fall to zero (roughness
/// eases to [`LENS_ROUGHNESS_ADS`]), so the sun can't glint off the lens while
/// aimed in; these are the values that fade starts from.
#[derive(Resource, Clone)]
pub(crate) struct LensSettings {
    /// Anti-reflective-coating tint of the glass (sRGB).
    pub(crate) tint: [f32; 3],
    /// Opacity of the glass at the hip (`1` = solid; less lets the world show
    /// through). Rises to fully opaque as the sight picture comes in.
    pub(crate) alpha: f32,
    /// Surface roughness (low = tight, mirror-like highlight).
    pub(crate) roughness: f32,
    /// Metalness of the glass.
    pub(crate) metallic: f32,
    /// Specular reflectance of the glass.
    pub(crate) reflectance: f32,
}

impl Default for LensSettings {
    fn default() -> Self {
        Self {
            tint: [0.0, 0.0, 0.0],
            alpha: 0.98,
            roughness: 0.25,
            metallic: 0.5,
            reflectance: 0.75,
        }
    }
}

/// Marks the scope's rear (player-facing) lens mesh; it displays the scope
/// render target and fades in with ADS.
#[derive(Component)]
pub(crate) struct ScopeLens;

/// The second camera that renders the magnified world into the scope texture.
#[derive(Component)]
pub(crate) struct ScopeCamera;

/// The reticle quad (texture picked by [`Settings::crosshair`]) in front of
/// the scope camera; only that camera sees it, so the reticle appears in the
/// scope image and nowhere else.
#[derive(Component)]
pub(crate) struct ScopeReticle;

/// Handle to the image the scope camera renders into and the lens samples.
#[derive(Resource)]
pub(crate) struct ScopeRenderTarget(pub(crate) Handle<Image>);

/// Handle to [`ScopeReticle`]'s material — [`apply_crosshair_texture`] swaps
/// its `base_color_texture` live whenever [`Settings::crosshair`] changes.
#[derive(Resource)]
pub(crate) struct ScopeReticleMaterial(pub(crate) Handle<StandardMaterial>);

/// `Settings::crosshair` → its texture path under `assets/textures/reticles/`. Kept
/// here (next to the one place that loads it) rather than on the enum
/// itself, the same way `environment::map` keeps `MapId`'s `.glb` paths out
/// of `shared::MapId`.
pub(crate) fn crosshair_asset_path(id: CrosshairId) -> &'static str {
    match id {
        CrosshairId::HashReticle => "textures/reticles/hash.png",
        CrosshairId::DuplexReticle => "textures/reticles/duplex.png",
        CrosshairId::HashReticleRedDot => "textures/reticles/hash_red_dot.png",
    }
}

/// Push `Settings::crosshair` onto the reticle material whenever it changes.
pub(crate) fn apply_crosshair_texture(
    settings: Res<Settings>,
    reticle_material: Res<ScopeReticleMaterial>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut applied: Local<Option<CrosshairId>>,
) {
    if applied.is_some_and(|c| c == settings.crosshair) {
        return;
    }
    *applied = Some(settings.crosshair);
    if let Some(material) = materials.get_mut(&reticle_material.0) {
        material.base_color_texture =
            Some(asset_server.load(crosshair_asset_path(settings.crosshair)));
    }
}

/// Copy the world camera's [`DistanceFog`] onto the scope camera whenever it
/// changes, so the lens picture is fogged exactly like the world around it.
/// `apply_scene_tuning` only ever writes the fog to [`WorldModelCamera`]
/// (per-map colour / visibility), so without this the scope camera renders the
/// world — sky included — with no fog at all.
pub(crate) fn sync_scope_fog(
    world_fog: Single<Ref<DistanceFog>, (With<WorldModelCamera>, Without<ScopeCamera>)>,
    mut scope_fog: Single<&mut DistanceFog, (With<ScopeCamera>, Without<WorldModelCamera>)>,
    mut synced: Local<bool>,
) {
    if *synced && !world_fog.is_changed() {
        return;
    }
    *synced = true;
    **scope_fog = world_fog.clone();
}

/// Panel-adjustable scope-reticle behaviour: size on the glass, the CoD-style
/// aim-in drift (starts toward one corner and slides to centre as you scope
/// in), and whether the HUD centre dot ever fades. The reticle otherwise never
/// moves — see [`update_scope`] — so the crosshair reads as pinned to the dead
/// centre of the screen the instant the raise finishes, exactly like the
/// gun-sway-instead-of-crosshair-sway newer Call of Duty titles use.
#[derive(Resource)]
pub(crate) struct CrosshairSettings {
    /// Reticle size multiplier (`1.0` = fills the scope view exactly; `>1` pushes
    /// the crosshair's outer ends past the glass edge).
    pub(crate) scale: f32,
    /// Where the reticle sits at `Ads::t == 0`, as a multiple of the finished
    /// scope's half-view — it eases to centre by full ADS. Positive `x` = left, positive
    /// `y` = up, so the default starts the crosshair toward the upper-left like a
    /// CoD scope. The scope camera counter-aims by the same amount so the target
    /// stays under the reticle.
    pub(crate) aim_in_frac: Vec2,
    /// Counter-sway gain per axis (`x` = yaw, `y` = pitch): how far the reticle
    /// travels *against* the weapon sway, in scope half-views per radian of
    /// [`WeaponSwayState::offset`] — the same offset the view model is tipped
    /// by this frame, so the two move in lockstep. The lens rides on the gun,
    /// so it swings with the sway; a gain that cancels that swing exactly keeps
    /// the reticle's centre still on the screen. `0` = off (the reticle sits at
    /// the lens centre and swings with the gun). Dialed in by eye. Rotating the whole sniper about
    /// the eye moves the lens by roughly the sway angle, which puts a good gain
    /// near `1 / (lens half-size on screen)` ≈ 2; dial it in from there.
    pub(crate) sway_counter: Vec2,
    /// Keep the HUD centre dot at full opacity instead of fading it out as the
    /// sight picture comes in.
    pub(crate) center_dot_always: bool,
}

impl Default for CrosshairSettings {
    fn default() -> Self {
        Self {
            scale: 1.2,
            aim_in_frac: Vec2::new(3.0, 3.0),
            sway_counter: Vec2::new(2.3, 2.0),
            center_dot_always: false,
        }
    }
}

/// Drive the render-to-texture scope: switch its camera on only while aiming,
/// keep its magnification in sync, slide the reticle in from the corner, and
/// fade the scope image in on the lens.
///
/// The reticle quad itself only ever moves for that corner-to-centre raise
/// slide (`aim_in`) — once the raise finishes it is pinned to the dead centre
/// of the scope view and stays there, full stop. The scope *camera*, though, still
/// picks up [`AimSwayState::offset`] (written by [`aim_idle_sway`] onto the
/// real [`WorldModelCamera`]) so the magnified picture drifts by the exact
/// same amount as the real aim: the world moves under the reticle instead of
/// the reticle moving over the world, which is what actually sells "you're
/// looking through a real optic, and holding it takes real effort."
#[allow(clippy::type_complexity)]
pub(crate) fn update_scope(
    (ads, aim_sway, crosshair, weapon_sway): (
        Res<Ads>,
        Res<AimSwayState>,
        Res<CrosshairSettings>,
        Res<WeaponSwayState>,
    ),
    tuning: Res<AdsTuning>,
    settings: Res<Settings>,
    killcam: Res<ActiveKillCam>,
    lens_cfg: Res<LensSettings>,
    scope_camera: Single<
        (&mut Camera, &mut Projection, &mut Transform),
        (With<ScopeCamera>, Without<ScopeReticle>),
    >,
    mut reticle: Single<&mut Transform, (With<ScopeReticle>, Without<ScopeCamera>)>,
    mut lens: Query<(&MeshMaterial3d<StandardMaterial>, &mut Visibility), With<ScopeLens>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let active = ads.t > SCOPE_SHOW_AT;

    // How far the magnified sight picture has come in: held off until the optic
    // is nearly centred on the eye, then ramped to full by `ads.t == 1`, so the
    // transition reads as a plain zoom-and-raise rather than a second,
    // wrongly-zoomed copy of the target sliding in.
    let picture = scope_picture_amount(ads.t, &tuning);
    // Follows the pose blend, so the drift below tracks the glass coming up.
    let e = ads_ease(ads.t, tuning.ads_ease);

    let (mut camera, mut projection, mut cam_transform) = scope_camera.into_inner();
    camera.is_active = active;
    // Before the picture comes in, keep the scope camera at the world FOV so the
    // render target matches the view *behind* the glass 1:1; converge to the
    // real scope magnification as the picture arrives.
    let optic = Optic::current(&settings, &killcam);
    let world_fov = ads_fov_rad(optic, &tuning, ads.t);
    let scope_fov = world_fov.lerp(full_scope_fov_rad(optic, &tuning), picture);
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = scope_fov;
    }

    // CoD-style aim-in drift: at the hip the reticle sits off-centre — up-left
    // for positive `aim_in_frac`, as a multiple of the *finished* scope's
    // half-view, so the same setting reads the same at every zoom — and eases to
    // dead centre by full ADS. The scope camera looks the opposite way by the
    // same angle, so the reticle stays over the point the eye is already aiming
    // at while the glass slides up into it. Composed with the real aim-sway
    // rotation, so once the raise finishes (`aim_in == 0`) the camera is
    // rotated by exactly the same amount as `WorldModelCamera` and the picture
    // it renders lines up with the real aim.
    let final_half_fov = full_scope_fov_rad(optic, &tuning) * 0.5;
    let aim_in = Vec2::new(
        final_half_fov * crosshair.aim_in_frac.x,
        final_half_fov * crosshair.aim_in_frac.y,
    ) * (1.0 - e);
    let aim_in_rot = Quat::from_euler(EulerRot::YXZ, -aim_in.x, -aim_in.y, 0.0);
    let aim_sway_rot = Quat::from_euler(EulerRot::YXZ, aim_sway.offset.x, aim_sway.offset.y, 0.0);
    cam_transform.rotation = aim_sway_rot * aim_in_rot;

    // Fill the scope camera's square view (`scale` lets the crosshair art run
    // past the glass edge), and hold the reticle `aim_in` off-centre: rotating
    // the quad about the camera (positive yaw = left, positive pitch = up)
    // swings it toward the corner, exactly opposite the camera's counter-aim
    // above. It also moves opposite the weapon sway (`sway_counter`): the lens
    // rides on the gun, so the reticle drawn inside it swings with the sway
    // unless it's pushed back by the same amount, read from the very
    // `WeaponSwayState` the view model was just tipped by. With the raise
    // finished and the counter dialed in, the reticle stays dead centre on the
    // screen; the sway itself lives on the camera and the model instead.
    let half_fov = scope_fov * 0.5;
    let fill = 2.0 * RETICLE_DIST * half_fov.tan() * crosshair.scale.max(0.01);
    let counter = weapon_sway.offset * crosshair.sway_counter * final_half_fov;
    let drift = Quat::from_euler(EulerRot::YXZ, aim_in.x - counter.x, aim_in.y - counter.y, 0.0);
    **reticle = Transform::from_rotation(drift)
        * Transform {
            translation: Vec3::new(0.0, 0.0, -RETICLE_DIST),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(fill, fill, 1.0),
        };

    // The lens is always drawn: a reflective glass disc at the hip, the sight
    // picture while scoped. `e` crossfades between the two looks, tied to the
    // picture ramp so the glass stays mirror-like until the picture is due.
    let e = picture;
    let k = 1.0 - e;
    for (material, mut visibility) in &mut lens {
        *visibility = Visibility::Inherited;
        if let Some(material) = materials.get_mut(&material.0) {
            // Sight picture rides on emissive: black at the hip, full while scoped.
            material.emissive = LinearRgba::rgb(e, e, e);
            // Glass tint fades to a black backing so the sight picture stays clean.
            material.base_color = Color::srgba(
                lens_cfg.tint[0] * k,
                lens_cfg.tint[1] * k,
                lens_cfg.tint[2] * k,
                lens_cfg.alpha.lerp(1.0, e),
            );
            // Dial the mirror sheen out as the player scopes in — reflectance to
            // zero at full ADS so the sun leaves no glint on the glass.
            material.perceptual_roughness = lens_cfg.roughness.lerp(LENS_ROUGHNESS_ADS, e);
            material.metallic = lens_cfg.metallic * k;
            material.reflectance = lens_cfg.reflectance * k;
        }
    }
}
