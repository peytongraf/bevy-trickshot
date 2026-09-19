//! View-model sway: the turn-lag "weapon sway", the idle breathing drift, and
//! the aim-idle drift that actually moves the real aim while scoped.

use bevy::prelude::*;

use crate::player::{LookDelta, Player, PlayerPhysics, WorldModelCamera};

use super::ads::Ads;
use super::knife_view_model::KnifeViewModel;
use super::view_model::ViewModel;

/// Cheap "breathing" motion shared by [`idle_weapon_sway`] and [`update_scope`]'s
/// aim sway: two sine waves at different frequencies per axis trace a slow
/// Lissajous loop instead of a straight back-and-forth line, which reads as
/// more organic than a single shared frequency would.
pub(crate) fn breathing_offset(clock: f32, freq_hz: Vec2, amp_rad: Vec2) -> Vec2 {
    Vec2::new(
        (clock * freq_hz.x * std::f32::consts::TAU).sin() * amp_rad.x,
        (clock * freq_hz.y * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2).sin() * amp_rad.y,
    )
}

/// Running weapon-sway offset (radians), a low-passed lag behind the view.
#[derive(Resource, Default)]
pub(crate) struct WeaponSwayState {
    /// `x` = yaw offset, `y` = pitch offset, applied on top of the ADS pose.
    pub(crate) offset: Vec2,
}

/// Panel-adjustable weapon sway: the view model trails the direction you turn,
/// then springs back to centre — angling *away* from the turn (look left, the
/// gun tips right) the way modern Call of Duty titles show it. A translation
/// rides along with the same lag angle, so the gun also shifts a touch the
/// same way it tips, instead of just rotating in place. This is the only
/// visible "you're moving the mouse" feedback while scoped, on the model
/// itself — the scope reticle only moves for the aim-in slide and the optional
/// counter-sway (see [`update_scope`], [`CrosshairSettings`]).
#[derive(Resource)]
pub(crate) struct WeaponSwaySettings {
    /// Seconds of lag at the hip: sway angle ≈ turn rate (rad/s) × this.
    pub(crate) hip_strength: f32,
    /// Seconds of lag at full ADS. The live value lerps `hip → ads` by `Ads::t`,
    /// so 50% aimed is exactly halfway between the two. Kept noticeably higher
    /// than it needs to be at the hip, since scoped is where this now carries
    /// all the "you're touching the mouse" feedback the reticle used to.
    pub(crate) ads_strength: f32,
    /// How fast the weapon catches back up to centre (larger = snappier).
    pub(crate) return_speed: f32,
    /// Hard cap on the sway angle in any direction (degrees).
    pub(crate) max_offset_deg: f32,
    /// Metres of translation per radian of the current lag angle, at the hip.
    pub(crate) hip_shift_m: f32,
    /// Metres of translation per radian of the current lag angle, at full ADS
    /// — bigger than `hip_shift_m` so the shift reads clearly once scoped.
    pub(crate) ads_shift_m: f32,
}

impl Default for WeaponSwaySettings {
    fn default() -> Self {
        Self {
            hip_strength: 0.03,
            ads_strength: 0.08,
            return_speed: 6.0,
            max_offset_deg: 12.0,
            hip_shift_m: 0.01,
            ads_shift_m: 0.005,
        }
    }
}

/// Running clock + blend for [`idle_weapon_sway`].
#[derive(Resource, Default)]
pub(crate) struct IdleSwayState {
    /// Seconds, advanced only while `blend` is above ~0 so the motion doesn't
    /// jump mid-cycle when it resumes after a walk.
    clock: f32,
    /// Eases `0` (moving) → `1` (settled at rest).
    blend: f32,
}

/// Panel-adjustable idle sway: a slow procedural "breathing" drift added to the
/// view model on top of [`WeaponSwaySettings`] while the player stands still —
/// that one only reacts to turning, this one is present even dead still. Fades
/// in on stopping / out on moving, and fades toward zero approaching full ADS,
/// where [`AimSwaySettings`] takes over the job of keeping the aim alive.
#[derive(Resource)]
pub(crate) struct IdleSwaySettings {
    /// Peak yaw/pitch drift (degrees), fully settled at the hip.
    pub(crate) amplitude_deg: Vec2,
    /// Cycles per second of the yaw / pitch drift — different so the weapon
    /// traces a slow loop instead of a straight back-and-forth line.
    pub(crate) frequency_hz: Vec2,
    /// How fast the effect blends in/out around a stop/start (larger = snappier).
    pub(crate) blend_speed: f32,
}

impl Default for IdleSwaySettings {
    fn default() -> Self {
        Self {
            amplitude_deg: Vec2::new(0.35, 0.22),
            frequency_hz: Vec2::new(0.18, 0.26),
            blend_speed: 2.5,
        }
    }
}

/// Running clock + live offset for [`aim_idle_sway`], shared with
/// [`update_scope`] so the scope camera's own framing rotates by the same
/// amount as the real aim.
#[derive(Resource, Default)]
pub(crate) struct AimSwayState {
    clock: f32,
    /// Current yaw/pitch drift (radians), written by [`aim_idle_sway`].
    /// `pub(crate)` so `killcam::start_killcam` can zero it — nothing else
    /// runs to reset it once [`aim_idle_sway`] is gated off for a replay.
    pub(crate) offset: Vec2,
}

/// Panel-adjustable "aiming idle sway": a slow breathing drift on the
/// player's *actual* aim while scoped, scaled up toward full ADS instead of
/// down (the opposite of [`IdleSwaySettings`], which fades out approaching
/// ADS). Unlike every other sway in this file, this one really does move the
/// world camera — [`aim_idle_sway`] rotates it directly, so it changes where a
/// shot actually lands (see [`weapon_system`]'s aim ray) — while the scope
/// reticle itself stays pinned to the centre of the screen. That combination
/// is the point: the reticle never moves, but the *world drifts under it*,
/// exactly like a real scope's picture drifts with natural body sway while
/// the reticle stays fixed to the optic.
#[derive(Resource)]
pub(crate) struct AimSwaySettings {
    /// Peak yaw/pitch drift (degrees of scope view) at full ADS.
    pub(crate) amplitude_deg: Vec2,
    /// Cycles per second of the yaw / pitch drift.
    pub(crate) frequency_hz: Vec2,
}

impl Default for AimSwaySettings {
    fn default() -> Self {
        Self {
            amplitude_deg: Vec2::new(0.12, 0.09),
            frequency_hz: Vec2::new(0.2, 0.31),
        }
    }
}

/// Make the weapon trail the direction the player turns and then catch up.
///
/// Runs after [`apply_ads`] has written the base pose and multiplies a small
/// rotation (plus a proportional translation — see [`WeaponSwaySettings`])
/// about the view origin onto it. The target angle is the view's angular
/// velocity this frame scaled by `strength` (seconds of lag), negated so the
/// gun lags *behind* the turn — turn left and the offset goes negative, which
/// tips the gun right and (via the shift below) nudges it right too, matching
/// modern CoD's ADS sway. A frame-rate-independent ease pulls the live offset
/// toward the target, so releasing the turn (target → 0) lets the gun spring
/// back. `strength`/shift both lerp `hip → ads` by `Ads::t`.
/// [`consume_look_delta`] clears this frame's turn once this and
/// [`track_trick`] have read it.
pub(crate) fn weapon_sway(
    time: Res<Time>,
    tuning: Res<WeaponSwaySettings>,
    ads: Res<Ads>,
    look: Res<LookDelta>,
    mut state: ResMut<WeaponSwayState>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let dt = time.delta_secs().max(1e-5);
    let applied = look.applied;
    let t = ads.t.clamp(0.0, 1.0);

    let strength = tuning.hip_strength.lerp(tuning.ads_strength, t);

    let max = tuning.max_offset_deg.to_radians();
    // `-applied / dt` is the view's angular velocity, opposite the turn.
    let target = (-applied / dt * strength).clamp(Vec2::splat(-max), Vec2::splat(max));

    let k = 1.0 - (-tuning.return_speed * dt).exp();
    state.offset = state.offset.lerp(target, k);

    **view_model = sway_pose(state.offset, &tuning, t) * **view_model;
}

/// The sway transform for a lag `offset` (yaw, pitch in radians): the tip
/// rotation, plus a shift the same way the weapon is currently tipped, so
/// the two read as one coherent motion instead of a rotation with an
/// unrelated wobble on top. Shared by [`weapon_sway`] (sniper) and
/// [`knife_weapon_sway`] so both view models sway identically off the same
/// [`WeaponSwaySettings`].
fn sway_pose(offset: Vec2, tuning: &WeaponSwaySettings, ads_t: f32) -> Transform {
    let sway = Quat::from_euler(EulerRot::YXZ, offset.x, offset.y, 0.0);
    let shift_m = tuning.hip_shift_m.lerp(tuning.ads_shift_m, ads_t);
    let shift = Vec3::new(-offset.x, -offset.y, 0.0) * shift_m;
    Transform::from_translation(shift) * Transform::from_rotation(sway)
}

/// The same turn-lag sway on the knife view model. Runs right after
/// [`weapon_sway`] and reuses the offset it just updated (`WeaponSwayState`)
/// rather than tracking its own, so the two models can never disagree — and
/// so `WeaponSwaySettings` (the "Weapon sway" debug panel section) tunes both
/// at once. Has to run after `apply_knife_transform`, which rewrites the
/// knife's base pose from scratch every frame (see `main.rs`'s schedule).
pub(crate) fn knife_weapon_sway(
    tuning: Res<WeaponSwaySettings>,
    ads: Res<Ads>,
    state: Res<WeaponSwayState>,
    mut knife: Single<&mut Transform, With<KnifeViewModel>>,
) {
    **knife = sway_pose(state.offset, &tuning, ads.t.clamp(0.0, 1.0)) * **knife;
}

/// Layer a slow procedural "breathing" drift onto the view model while the
/// player stands still — runs after [`weapon_sway`] and multiplies its own
/// offset on top of that one's. Blends toward zero the instant the player
/// moves (and back in once they settle) and fades out approaching full ADS,
/// where [`update_scope`]'s aim sway takes over instead.
pub(crate) fn idle_weapon_sway(
    time: Res<Time>,
    tuning: Res<IdleSwaySettings>,
    ads: Res<Ads>,
    player: Single<&PlayerPhysics, With<Player>>,
    mut state: ResMut<IdleSwayState>,
    mut view_model: Single<&mut Transform, With<ViewModel>>,
) {
    let dt = time.delta_secs();
    let idle = player.grounded && player.horizontal_velocity.length() < 0.1;
    let target = if idle { 1.0 } else { 0.0 };
    let k = 1.0 - (-tuning.blend_speed * dt).exp();
    state.blend = state.blend.lerp(target, k);

    if state.blend > 1e-3 {
        state.clock += dt;
    }

    let amp = Vec2::new(
        tuning.amplitude_deg.x.to_radians(),
        tuning.amplitude_deg.y.to_radians(),
    ) * state.blend
        * (1.0 - ads.t.clamp(0.0, 1.0));
    let offset = breathing_offset(state.clock, tuning.frequency_hz, amp);

    let sway = Quat::from_euler(EulerRot::YXZ, offset.x, offset.y, 0.0);
    **view_model = Transform::from_rotation(sway) * **view_model;
}

/// Advance the aim-breathing clock and rotate the *real* world camera by it —
/// see [`AimSwaySettings`]. Sets (never accumulates) [`WorldModelCamera`]'s
/// local rotation directly, the same "replaced every frame" rule
/// [`CameraShake`] uses, so it can never drift. [`update_scope`] reads the
/// resulting [`AimSwayState::offset`] back to rotate the scope camera by the
/// identical amount, so the magnified picture drifts in lockstep with the
/// real aim while the reticle itself stays fixed to the screen.
///
/// Gated off during a kill cam (like [`weapon_sway`]) since it writes real
/// camera state that `killcam::drive_killcam` doesn't know about — otherwise
/// a *viewer's own* current breathing would leak into their view of someone
/// else's replay. `killcam::start_killcam` resets both cameras' rotation to
/// identity when a replay begins, since nothing here will while it's gated
/// off.
pub(crate) fn aim_idle_sway(
    time: Res<Time>,
    tuning: Res<AimSwaySettings>,
    ads: Res<Ads>,
    mut state: ResMut<AimSwayState>,
    mut world_cam: Single<&mut Transform, With<WorldModelCamera>>,
) {
    state.clock += time.delta_secs();

    let e = ads.t.clamp(0.0, 1.0);
    let amp = Vec2::new(
        tuning.amplitude_deg.x.to_radians(),
        tuning.amplitude_deg.y.to_radians(),
    ) * e;
    state.offset = breathing_offset(state.clock, tuning.frequency_hz, amp);

    world_cam.rotation = Quat::from_euler(EulerRot::YXZ, state.offset.x, state.offset.y, 0.0);
}
