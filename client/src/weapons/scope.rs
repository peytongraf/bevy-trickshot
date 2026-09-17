//! The render-to-texture scope: its camera, reticle and rear lens, and the
//! system that drives the aim-in slide + sight-picture fade.

use bevy::prelude::*;

use crate::settings::Settings;
use crate::util::ads_ease;

use super::ads::{ads_fov_rad, scope_picture_amount, Ads, AdsTuning};
use super::sway::AimSwayState;

// --- scope (render-to-texture) -------------------------------------------------
// A second camera renders the world only (no view model) through a narrow FOV
// into `SCOPE_RT_SIZE`² and that image is shown on the scope's rear lens, so the
// player looks *through* the scope instead of down the tube. The scope camera is
// only active while `ads.t > 0`, so the extra pass costs nothing at the hip.
/// Render-target resolution for the scope view.
pub(crate) const SCOPE_RT_SIZE: u32 = 512;
/// Scope camera FOV (degrees). Much narrower than the main camera = the
/// magnification. Adjustable live in the tuning panel.
pub(crate) const SCOPE_FOV_DEG: f32 = 6.5;
/// Distance (metres) the reticle quad sits in front of the scope camera.
pub(crate) const RETICLE_DIST: f32 = 0.2;
/// ADS amount below which the scope camera is switched off, so no stale frame
/// is rendered at the hip. The lens itself stays visible either way — see below.
pub(crate) const SCOPE_SHOW_AT: f32 = 0.02;

// The rear lens is one lit `StandardMaterial`. Off-aim it reads as a smooth,
// strongly reflective coated-glass disc catching the sun; as `Ads::t` → 1 the
// mirror sheen is dialled out and the render-to-texture sight picture (carried
// on `emissive`) fades in, so glare can't wash out the shot.
/// Cool anti-reflective-coating tint of the glass when not aiming.
pub(crate) const LENS_TINT: (f32, f32, f32) = (0.14, 0.21, 0.34);
/// Lens surface roughness at the hip (low = tight, mirror-like highlight) and
/// while fully scoped (fully matte — with `reflectance` at 0 there's no sun
/// glint on the glass once aimed in).
pub(crate) const LENS_ROUGHNESS_HIP: f32 = 0.04;
pub(crate) const LENS_ROUGHNESS_ADS: f32 = 1.0;
/// Metalness / reflectance of the glass at the hip; both fall to zero as the
/// player scopes in, leaving a non-reflective black backing behind the sight
/// picture so the sun can't glint off the lens while aimed in.
pub(crate) const LENS_METALLIC_HIP: f32 = 0.65;
pub(crate) const LENS_REFLECTANCE_HIP: f32 = 1.0;

/// Marks the scope's rear (player-facing) lens mesh; it displays the scope
/// render target and fades in with ADS.
#[derive(Component)]
pub(crate) struct ScopeLens;

/// The second camera that renders the magnified world into the scope texture.
#[derive(Component)]
pub(crate) struct ScopeCamera;

/// The `crosshair.png` quad in front of the scope camera; only that camera sees
/// it, so the reticle appears in the scope image and nowhere else.
#[derive(Component)]
pub(crate) struct ScopeReticle;

/// Handle to the image the scope camera renders into and the lens samples.
#[derive(Resource)]
pub(crate) struct ScopeRenderTarget(pub(crate) Handle<Image>);

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
    /// Where the reticle sits at `Ads::t == 0`, as a fraction of the scope's
    /// half-view — it eases to centre by full ADS. Positive `x` = left, positive
    /// `y` = up, so the default starts the crosshair toward the upper-left like a
    /// CoD scope. The scope camera counter-aims by the same amount so the target
    /// stays under the reticle.
    pub(crate) aim_in_frac: Vec2,
    /// Keep the HUD centre dot at full opacity instead of fading it out as the
    /// sight picture comes in.
    pub(crate) center_dot_always: bool,
}

impl Default for CrosshairSettings {
    fn default() -> Self {
        Self {
            scale: 1.2,
            aim_in_frac: Vec2::new(3.0, 3.0),
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
/// of the screen and stays there, full stop. The scope *camera*, though, still
/// picks up [`AimSwayState::offset`] (written by [`aim_idle_sway`] onto the
/// real [`WorldModelCamera`]) so the magnified picture drifts by the exact
/// same amount as the real aim: the world moves under the reticle instead of
/// the reticle moving over the world, which is what actually sells "you're
/// looking through a real optic, and holding it takes real effort."
#[allow(clippy::type_complexity)]
pub(crate) fn update_scope(
    (ads, aim_sway, crosshair): (Res<Ads>, Res<AimSwayState>, Res<CrosshairSettings>),
    tuning: Res<AdsTuning>,
    settings: Res<Settings>,
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
    let world_fov = ads_fov_rad(settings.fov, &tuning, ads.t);
    let scope_fov = world_fov.lerp(tuning.scope_fov_deg.to_radians(), picture);
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = scope_fov;
    }

    // CoD-style aim-in drift: at the hip the reticle starts toward a corner
    // (`aim_in_frac` of the scope's half-view) and the scope camera looks that
    // way too, so the point the eye is already aiming at stays pinned under the
    // reticle while the glass slides up into it. Eases to zero by full ADS —
    // composed with the real aim-sway rotation so the scope camera ends up
    // rotated by exactly the same amount as `WorldModelCamera` once the raise
    // finishes (aim_in == 0), and the picture it renders lines up with the
    // real aim.
    let half_fov = scope_fov * 0.5;
    let aim_in = Vec2::new(
        half_fov * crosshair.aim_in_frac.x,
        half_fov * crosshair.aim_in_frac.y,
    ) * (1.0 - e);
    let aim_in_rot = Quat::from_euler(EulerRot::YXZ, -aim_in.x, -aim_in.y, 0.0);
    let aim_sway_rot = Quat::from_euler(EulerRot::YXZ, aim_sway.offset.x, aim_sway.offset.y, 0.0);
    cam_transform.rotation = aim_sway_rot * aim_in_rot;

    // Fill the scope camera's square view (`scale` lets the crosshair art run
    // past the glass edge) and pin the reticle to dead centre — no sway, no
    // breathing, nothing but the fixed forward offset. It only ever needs
    // resizing, never re-aiming.
    let fill = 2.0 * RETICLE_DIST * half_fov.tan() * crosshair.scale.max(0.01);
    **reticle = Transform {
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
                LENS_TINT.0 * k,
                LENS_TINT.1 * k,
                LENS_TINT.2 * k,
                0.9f32.lerp(1.0, e),
            );
            // Dial the mirror sheen out as the player scopes in — reflectance to
            // zero at full ADS so the sun leaves no glint on the glass.
            material.perceptual_roughness = LENS_ROUGHNESS_HIP.lerp(LENS_ROUGHNESS_ADS, e);
            material.metallic = LENS_METALLIC_HIP * k;
            material.reflectance = LENS_REFLECTANCE_HIP * k;
        }
    }
}
