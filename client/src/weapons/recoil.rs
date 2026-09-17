//! Camera shake + weapon recoil feedback: per-shot trauma that drives a
//! positional/rotational camera shake, a separate forward/back recoil kick,
//! and the view-model shudder that rides alongside both.

use bevy::prelude::*;

use super::ads::Ads;
use super::view_model::ViewModel;

/// Camera shake: one shot adds `SHAKE_ADD` trauma (capped at 1), which decays at
/// `SHAKE_DECAY` per second. The visible offset scales with `trauma²`, so it is
/// violent immediately and gone in a fraction of a second.
///
/// Two things ride on trauma, both on the `CameraShake` node (gun + cameras
/// together, so the gun stays locked to the screen while the world swings):
/// * a small positional up/down + side/side jitter (`SHAKE_POS_MAX`), and
/// * a **view punch** — a directional pitch-up (`SHAKE_VIEW_PUNCH_DEG`) that
///   recovers with trauma, plus rotational chaos (`SHAKE_VIEW_JITTER_DEG`) on
///   top. Scaled down while scoped so ADS stays controllable.
///
/// The gun *also* shudders relative to the camera — `weapon_recoil_shudder`
/// punches the view model back toward the eye (`SHAKE_WEAPON_KICK`) and climbs
/// the muzzle (`SHAKE_WEAPON_KICK_DEG`) as trauma decays. That one never touches
/// aim.
///
/// The forward / back move is separate again: a single backward *kick* on each
/// shot, on the `CameraRecoil` node, which carries only the cameras (not the
/// gun). It snaps the eye back and eases home over ~a tenth of a second, so the
/// scope's rear lens — which the fire animation yanks toward the face — stays in
/// front of the eye instead of sliding past it. All of this is live-tunable
/// (`ShakeSettings`, "Camera shake" panel section).
pub(crate) const SHAKE_ADD: f32 = 0.85;
pub(crate) const SHAKE_DECAY: f32 = 2.0;
pub(crate) const SHAKE_FREQ: f32 = 75.0;
/// Peak camera translation (world units) on the X and Y axes at full trauma.
pub(crate) const SHAKE_POS_MAX: f32 = 0.075;
/// Peak upward view-punch pitch (degrees) at full trauma — the CoD "kick".
pub(crate) const SHAKE_VIEW_PUNCH_DEG: f32 = 6.6;
/// Amplitude (degrees) of the random yaw / pitch / roll chaos on top of the
/// punch, scaled by `trauma²`.
pub(crate) const SHAKE_VIEW_JITTER_DEG: f32 = 2.9;
/// Metres the view model shudders back toward the eye at full trauma.
pub(crate) const SHAKE_WEAPON_KICK: f32 = 0.2;
/// Muzzle-climb rotation (degrees) applied to the view model at full trauma.
pub(crate) const SHAKE_WEAPON_KICK_DEG: f32 = 7.5;
/// Metres the camera (not the gun) snaps backward on each shot.
pub(crate) const SHAKE_RECOIL_KICK: f32 = 0.05;
/// How fast that backward kick eases back to zero (larger = snappier return).
pub(crate) const SHAKE_RECOIL_RETURN: f32 = 5.0;
/// Fraction of the camera shake that survives at full ADS; it ramps linearly
/// back to the full effect (`1.0`) at the hip so scoped aim stays controllable.
pub(crate) const SHAKE_ADS_SCALE: f32 = 0.15;

/// Parent of the cameras *and* the view model. Its local transform is
/// overwritten each frame by `camera_shake` with the up/down + side/side shake
/// offset (or identity), so the gun and the view shake together.
#[derive(Component)]
pub(crate) struct CameraShake;

/// Child of [`CameraShake`] that carries only the cameras (not the gun).
/// `camera_shake` sets its local Z to the current backward recoil kick, pulling
/// the eye off the scope's rear lens when a shot's fire animation drags the lens
/// toward the face.
#[derive(Component)]
pub(crate) struct CameraRecoil;

/// Camera-shake state. `trauma` (0..1) is bumped on each shot and decays; it
/// only scales offsets that are rebuilt from zero every frame — so the shake
/// (translation *and* the view-punch rotation) always settles back to exactly
/// identity and can never accumulate into the aim. `phase` just advances the
/// oscillation and resets at rest. `recoil` is the current backward (+Z local)
/// camera offset in metres, snapped up on a shot and eased back to zero.
#[derive(Resource, Default)]
pub(crate) struct Shake {
    pub(crate) trauma: f32,
    pub(crate) phase: f32,
    pub(crate) recoil: f32,
}

/// Panel-adjustable camera-shake tuning. The oscillation + view-punch half
/// mirrors the `SHAKE_*` consts; the recoil half is the forward/back kick that
/// keeps the eye behind the scope lens when firing.
#[derive(Resource)]
pub(crate) struct ShakeSettings {
    /// Trauma added per shot (result capped at 1).
    pub(crate) trauma_per_shot: f32,
    /// Trauma lost per second.
    pub(crate) decay: f32,
    /// Oscillation speed.
    pub(crate) frequency: f32,
    /// Peak up/down + side/side camera translation at full trauma (m).
    pub(crate) pos_max: f32,
    /// Peak upward view-punch pitch at full trauma (°). Rotates gun + cameras
    /// together; scaled down while scoped.
    pub(crate) view_punch_deg: f32,
    /// Amplitude of the random yaw/pitch/roll chaos on the view punch (°).
    pub(crate) view_jitter_deg: f32,
    /// Metres the view model shudders back toward the eye at full trauma.
    pub(crate) weapon_kick: f32,
    /// Muzzle-climb rotation applied to the view model at full trauma (°).
    pub(crate) weapon_kick_deg: f32,
    /// Metres the camera snaps *backward* on each shot (gun stays put).
    pub(crate) recoil_kick: f32,
    /// How fast the backward kick eases back to zero (1/s; larger = snappier).
    pub(crate) recoil_return: f32,
    /// Fraction of the camera shake left at full ADS (`0` = none, `1` = full).
    /// Ramps linearly to the full effect as the player aims back out.
    pub(crate) ads_scale: f32,
}

impl Default for ShakeSettings {
    fn default() -> Self {
        Self {
            trauma_per_shot: SHAKE_ADD,
            decay: SHAKE_DECAY,
            frequency: SHAKE_FREQ,
            pos_max: SHAKE_POS_MAX,
            view_punch_deg: SHAKE_VIEW_PUNCH_DEG,
            view_jitter_deg: SHAKE_VIEW_JITTER_DEG,
            weapon_kick: SHAKE_WEAPON_KICK,
            weapon_kick_deg: SHAKE_WEAPON_KICK_DEG,
            recoil_kick: SHAKE_RECOIL_KICK,
            recoil_return: SHAKE_RECOIL_RETURN,
            ads_scale: SHAKE_ADS_SCALE,
        }
    }
}

/// The one multiplier that scales *every* per-shot shake — camera positional
/// jitter, view punch, and the weapon shudder — down as the player scopes in:
/// `1.0` at the hip, ramping linearly to `cfg.ads_scale` at full ADS.
pub(crate) fn shake_ads_scale(cfg: &ShakeSettings, ads_t: f32) -> f32 {
    1.0 - (1.0 - cfg.ads_scale.clamp(0.0, 1.0)) * ads_t.clamp(0.0, 1.0)
}

/// Pure: the `CameraShake` node's local transform for a given shake state — the
/// positional jitter and view punch — a directional pitch-up plus rotational
/// chaos, both scaled by `trauma` / `trauma²` and eased down while scoped. At
/// rest (`trauma <= 0`) this is exactly identity.
///
/// Shared by the live [`camera_shake`] system and the kill-cam replay
/// (`killcam::drive_killcam`), so a replayed `(trauma, phase)` reproduces
/// on-screen exactly what the shooter saw.
pub(crate) fn shake_camera_pose(
    cfg: &ShakeSettings,
    ads_t: f32,
    trauma: f32,
    phase: f32,
) -> Transform {
    if trauma <= 0.0 {
        return Transform::IDENTITY;
    }
    let s = phase;
    let amt = trauma * trauma;

    // Scale the whole shake down as the player scopes in, so ADS stays
    // controllable while the hip still kicks hard (see [`shake_ads_scale`]).
    let ads_scale = shake_ads_scale(cfg, ads_t);

    // Positional jitter: up / down + side / side, no Z.
    let translation = Vec3::new(
        (s * 1.53 + 0.4).sin() * cfg.pos_max * amt * ads_scale,
        (s * 1.19 + 3.3).sin() * cfg.pos_max * amt * ads_scale,
        0.0,
    );

    // View punch: a directional pitch-up that recovers with `trauma`, plus
    // rotational chaos (`trauma²`) on top. +X rotation looks up.
    let punch = trauma * cfg.view_punch_deg.to_radians() * ads_scale;
    let jitter = cfg.view_jitter_deg.to_radians() * amt * ads_scale;
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        (s * 0.91 + 1.7).sin() * jitter,       // yaw
        punch + (s * 0.63).sin() * jitter,     // pitch (up)
        (s * 1.27 + 2.1).sin() * jitter * 0.6, // roll
    );

    Transform {
        translation,
        rotation,
        scale: Vec3::ONE,
    }
}

/// Rebuild the shake nodes' local transforms each frame from the current state,
/// always from zero, so they settle back to exactly identity and never drift aim.
///
/// * `CameraShake` (gun + cameras): the positional jitter and view punch from
///   [`shake_camera_pose`]. Rotating this node turns the gun and the cameras as
///   one, so the gun stays screen-locked while the world swings.
/// * `CameraRecoil` (cameras only): local +Z (straight back along the view axis)
///   set to the current recoil kick, which snaps up on a shot and eases home.
pub(crate) fn camera_shake(
    time: Res<Time>,
    cfg: Res<ShakeSettings>,
    ads: Res<Ads>,
    mut shake: ResMut<Shake>,
    mut rig: Single<&mut Transform, (With<CameraShake>, Without<CameraRecoil>)>,
    mut recoil_node: Single<&mut Transform, (With<CameraRecoil>, Without<CameraShake>)>,
) {
    let dt = time.delta_secs();

    // --- forward / back recoil: snap back on a shot, ease home fast ---------
    if shake.recoil > 0.0 {
        shake.recoil *= (-cfg.recoil_return * dt).exp();
        if shake.recoil < 1.0e-5 {
            shake.recoil = 0.0;
        }
    }
    let want_recoil = Transform::from_xyz(0.0, 0.0, shake.recoil); // +Z = backward
    if **recoil_node != want_recoil {
        **recoil_node = want_recoil;
    }

    // --- up / down + side / side oscillation -------------------------------
    shake.trauma = (shake.trauma - cfg.decay * dt).max(0.0);
    if shake.trauma > 0.0 {
        shake.phase += dt * cfg.frequency;
    } else {
        shake.phase = 0.0;
    }

    let want = shake_camera_pose(&cfg, ads.t, shake.trauma, shake.phase);
    if **rig != want {
        **rig = want;
    }
}

/// A short, sharp shudder layered onto the view model on top of the sway pose:
/// the gun punches back toward the eye and the muzzle climbs, then settles as
/// `Shake::trauma` decays. Separate from the view punch in [`camera_shake`] —
/// this moves the gun *relative* to the camera, the way a CoD weapon recoils,
/// and never touches aim. Pre-multiplied in camera space (+Z toward the eye,
/// +Y up), so it is independent of the view model's own orientation.
/// Pure: the shudder kick premultiplied onto the view model — the gun punches
/// back toward the eye and the muzzle climbs, scaled by `trauma` and by the same
/// [`shake_ads_scale`] as the camera shake. Shared by the live
/// [`weapon_recoil_shudder`] system and the kill-cam replay.
pub(crate) fn weapon_kick_pose(
    cfg: &ShakeSettings,
    ads_t: f32,
    trauma: f32,
    phase: f32,
) -> Transform {
    if trauma <= 0.0 {
        return Transform::IDENTITY;
    }
    let amt = trauma * trauma;
    let s = phase;
    let ads_scale = shake_ads_scale(cfg, ads_t);

    let back = amt * cfg.weapon_kick * ads_scale;
    let rise = amt * cfg.weapon_kick * 0.5 * ads_scale;
    let wobble = (s * 0.8).sin() * cfg.weapon_kick * 0.25 * amt * ads_scale;

    Transform {
        translation: Vec3::new(wobble, rise, back),
        rotation: Quat::from_rotation_x(trauma * cfg.weapon_kick_deg.to_radians() * ads_scale),
        scale: Vec3::ONE,
    }
}

pub(crate) fn weapon_recoil_shudder(
    cfg: Res<ShakeSettings>,
    ads: Res<Ads>,
    shake: Res<Shake>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let kick = weapon_kick_pose(&cfg, ads.t, shake.trauma, shake.phase);
    **view_model = kick * **view_model;
}
