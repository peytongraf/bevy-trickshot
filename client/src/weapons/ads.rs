//! Aim-down-sight: the no-scope inaccuracy cone, the live `Ads::t` ramp, and
//! the pure FOV/pose blend helpers shared with the kill-cam replay.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::keybinds::KeyBindings;
use crate::killcam::{self, ActiveKillCam};
use crate::player::WorldModelCamera;
use crate::settings::Settings;
use crate::util::{ads_ease, ease};
use crate::GameSounds;

use super::view_model::{lerp_pose, ViewModel, ViewModelPoses};

/// Seconds to go from hip to full aim-down-sight (and back).
pub(crate) const ADS_DURATION: f32 = 0.4;
// Hip FOV is a player setting (`Settings::fov`, default 90°). Everything on
// screen — inside the scope or not — is drawn at the current FOV, so shrinking it
// is the scope "magnification". The full-ADS FOV is derived from the equipped
// scope's zoom (`Settings::scope_zoom`) and the hip FOV; see `full_ads_fov_rad`.
//
// The lens shows a second camera's picture, and it has to line up with the world
// drawn around the lens. The view-model camera has a fixed FOV, so the lens covers
// a fixed fraction of the screen no matter how far the world camera zooms; that
// makes the scope camera's half-angle a constant multiple (in tangent space) of
// the world camera's. This pair was dialed in by eye in the tuning panel (a
// 9.5° world FOV against a 6.5° scope FOV matched inside and outside of the
// lens) and gives that constant — see `lens_fit_default`.
pub(crate) const LENS_CAL_ADS_FOV_DEG: f32 = 9.5;
pub(crate) const LENS_CAL_SCOPE_FOV_DEG: f32 = 6.5;

/// `tan(scope half-angle) / tan(world half-angle)` at the calibration pair above.
pub(crate) fn lens_fit_default() -> f32 {
    (LENS_CAL_SCOPE_FOV_DEG.to_radians() * 0.5).tan()
        / (LENS_CAL_ADS_FOV_DEG.to_radians() * 0.5).tan()
}

/// No-scope inaccuracy ("No-scope spread" panel section). Each shot is thrown
/// off the aim point by a random up/down + left/right angle; the cap on that
/// angle is `hip_max_deg` at the hip and eases to `0` at full ADS, so a fully
/// scoped shot always lands dead on. The random draw can still come out small,
/// which is what lets an occasional hip shot connect.
#[derive(Resource)]
pub(crate) struct NoScopeSpread {
    /// Largest up/down or left/right offset at the hip, in degrees.
    pub(crate) hip_max_deg: f32,
    /// Accuracy curve: `1` = linear falloff, `>1` keeps the spread wide through
    /// a partial ADS and then tightens fast as it approaches full ADS.
    pub(crate) curve: f32,
}

impl Default for NoScopeSpread {
    fn default() -> Self {
        Self {
            hip_max_deg: 4.0,
            curve: 2.5,
        }
    }
}

/// The per-axis half-angle (radians) a shot may be thrown off the aim point at
/// the given ADS amount — `cfg.hip_max_deg` at `ads_t == 0`, easing to `0` at
/// `ads_t == 1` along `1 - ads_t^curve`.
pub(crate) fn noscope_spread_angle(cfg: &NoScopeSpread, ads_t: f32) -> f32 {
    let t = ads_t.clamp(0.0, 1.0);
    let scale = (1.0 - t.powf(cfg.curve.max(0.01))).clamp(0.0, 1.0);
    cfg.hip_max_deg.to_radians() * scale
}

/// Aim-down-sight amount: 0 at the hip, 1 looking fully through the scope.
#[derive(Resource, Default)]
pub(crate) struct Ads {
    pub(crate) t: f32,
}

/// `Ads::t` at or below this counts as "no scope" for style points.
pub(crate) const NOSCOPE_ADS_MAX: f32 = 0.05;

/// Dev-only knobs for dialing in the ADS pose (driven by the egui panel).
#[derive(Resource)]
pub(crate) struct AdsTuning {
    /// Pin ADS to fully aimed regardless of the right mouse button, so the pose
    /// can be tuned with the cursor free.
    pub(crate) force_full: bool,
    /// How much of the world view the lens picture spans: the scope camera's
    /// `tan(half FOV)` as a fraction of the world camera's at full ADS. It only
    /// depends on the lens's size on screen, so one value serves every scope
    /// zoom; raise it if the lens picture looks zoomed out relative to the world
    /// around it, lower it if it looks zoomed in.
    pub(crate) lens_fit: f32,
    /// Milliseconds to go from hip to full aim-down-sight (and back), the way
    /// Call of Duty reports ADS time. Lower = snappier.
    pub(crate) ads_duration_ms: f32,
    /// Shape of the hip↔ADS blend: `0` = linear, `1` = full ease-in-out
    /// (smootherstep). Applied to the view-model pose and the FOV zoom.
    pub(crate) ads_ease: f32,
    /// `Ads::t` at which the magnified sight picture starts fading onto the
    /// glass. Below this the lens just reads as reflective glass and the scope
    /// camera tracks the world FOV, so aiming in is a clean zoom-and-raise
    /// instead of a second, pre-zoomed image of the target sliding into place.
    /// `0.0` = old behaviour (picture fades in across the whole blend).
    pub(crate) scope_picture_at: f32,
}

impl Default for AdsTuning {
    fn default() -> Self {
        Self {
            force_full: false,
            lens_fit: lens_fit_default(),
            ads_duration_ms: ADS_DURATION * 1000.0,
            ads_ease: 0.8,
            scope_picture_at: 0.25,
        }
    }
}

/// Ramp `Ads::t` toward 1 while the right mouse button is held, back toward 0
/// otherwise.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_ads(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    tuning: Res<AdsTuning>,
    sounds: Res<GameSounds>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    mut ads: ResMut<Ads>,
    mut was_aiming: Local<bool>,
    mut commands: Commands,
) {
    if tuning.force_full {
        ads.t = 1.0;
        *was_aiming = true;
        return;
    }

    let aiming =
        window.cursor_options.grab_mode != CursorGrabMode::None && binds.aim.pressed(&keys, &mouse);

    // One-shot cue the instant the player starts / stops aiming.
    if aiming != *was_aiming {
        let (clip, bit) = if aiming {
            (sounds.aim_in.clone(), killcam::SND_AIM_IN)
        } else {
            (sounds.aim_out.clone(), killcam::SND_AIM_OUT)
        };
        commands.spawn((AudioPlayer::new(clip), PlaybackSettings::DESPAWN));
        snd.note(bit);
        *was_aiming = aiming;
    }

    let target = if aiming { 1.0 } else { 0.0 };
    let step = time.delta_secs() / (tuning.ads_duration_ms.max(1.0) / 1000.0);
    ads.t = if ads.t < target {
        (ads.t + step).min(target)
    } else {
        (ads.t - step).max(target)
    };
}

/// The hip FOV and scope magnification an optic is drawn with: the live
/// player's own settings, or the shooter's while a kill cam replays them.
#[derive(Clone, Copy)]
pub(crate) struct Optic {
    pub(crate) hip_fov_deg: f32,
    pub(crate) zoom: f32,
}

impl Optic {
    /// The local player's own hip FOV and equipped scope.
    pub(crate) fn live(settings: &Settings) -> Self {
        Self {
            hip_fov_deg: settings.fov,
            zoom: settings.scope_zoom.magnification(),
        }
    }

    /// What to render with right now: the kill cam's shooter while one plays,
    /// otherwise the live player.
    pub(crate) fn current(settings: &Settings, killcam: &ActiveKillCam) -> Self {
        killcam
            .0
            .as_ref()
            .and_then(|run| run.optic)
            .unwrap_or_else(|| Self::live(settings))
    }
}

/// Pure: the world camera's vertical FOV (radians) at full ADS. Magnification is
/// the ratio of view-plane extents, so a `zoom`× scope has
/// `tan(ads/2) = tan(hip/2) / zoom` — exactly `zoom`× the hip view at any hip
/// FOV, and independent of aspect ratio.
pub(crate) fn full_ads_fov_rad(optic: Optic) -> f32 {
    2.0 * ((optic.hip_fov_deg.to_radians() * 0.5).tan() / optic.zoom.max(1.0)).atan()
}

/// Pure: the scope camera's vertical FOV (radians) at full ADS — the world
/// camera's, narrowed by [`AdsTuning::lens_fit`] so the lens picture lines up
/// with the world around it.
pub(crate) fn full_scope_fov_rad(optic: Optic, tuning: &AdsTuning) -> f32 {
    2.0 * ((full_ads_fov_rad(optic) * 0.5).tan() * tuning.lens_fit).atan()
}

/// Pure: the world camera's vertical FOV (radians), blended from the hip FOV
/// toward the scope's full-ADS zoom by `ads_t`. Shared by the live [`apply_ads`]
/// system and the kill-cam replay, so a replay renders at the *shooter's* hip
/// FOV and scope instead of the viewer's own.
pub(crate) fn ads_fov_rad(optic: Optic, tuning: &AdsTuning, ads_t: f32) -> f32 {
    optic.hip_fov_deg.to_radians().lerp(
        full_ads_fov_rad(optic),
        ads_ease(ads_t, tuning.ads_ease),
    )
}

/// How far the magnified scope picture has faded in: `0` until `Ads::t` reaches
/// `scope_picture_at`, easing to `1` by full ADS. Keeps the raise reading as a
/// plain zoom-and-lift instead of a second, pre-zoomed image of the target.
pub(crate) fn scope_picture_amount(ads_t: f32, tuning: &AdsTuning) -> f32 {
    let span = (1.0 - tuning.scope_picture_at).max(1e-3);
    ease(((ads_t - tuning.scope_picture_at) / span).clamp(0.0, 1.0))
}

pub(crate) fn apply_ads(
    ads: Res<Ads>,
    poses: Res<ViewModelPoses>,
    tuning: Res<AdsTuning>,
    settings: Res<Settings>,
    killcam: Res<ActiveKillCam>,
    mut world_projection: Single<&mut Projection, With<WorldModelCamera>>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let e = ads_ease(ads.t, tuning.ads_ease);

    if let Projection::Perspective(perspective) = world_projection.as_mut() {
        perspective.fov = ads_fov_rad(Optic::current(&settings, &killcam), &tuning, ads.t);
    }

    **view_model = lerp_pose(&poses.hip, &poses.ads, e);
}
