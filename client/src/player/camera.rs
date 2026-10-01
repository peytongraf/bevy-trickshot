//! Mouse look: yaw on the player body, pitch on the head, sensitivity scaled
//! by the ADS zoom ([`zoom_sens_scale`]) — plus the render-camera markers
//! hanging off the head.

use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::settings::Settings;
use crate::util::ease;
use crate::Ads;

use super::movement::Player;

pub(crate) const MOUSE_SENSITIVITY: Vec2 = Vec2::new(0.003, 0.002);
pub(crate) const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.02;

/// Child of `Player`. Carries pitch (vertical look); cameras and the gun hang
/// off of this so movement stays level with the ground.
#[derive(Component)]
pub(crate) struct PlayerHead;

/// The camera that renders the world (layer 0 only).
#[derive(Component)]
pub(crate) struct WorldModelCamera;

/// The camera that renders the first-person gun (layer 1 only, drawn on top).
#[derive(Component)]
pub(crate) struct ViewModelCamera;

/// The yaw / pitch the view actually rotated by this frame (radians), written by
/// [`look_around`]. Read by [`weapon_sway`] and [`track_trick`] (the yaw
/// component — the exact, unwrapped turn, immune to the aliasing a
/// `Transform`-based diff would have), then zeroed by [`consume_look_delta`]
/// so a frame with no mouse input (or a paused game) reads as "no turn" —
/// `look_around` early-returns on a still frame without touching it.
#[derive(Resource, Default)]
pub(crate) struct LookDelta {
    /// `x` = yaw (positive = turned left), `y` = pitch (positive = looked up).
    pub(crate) applied: Vec2,
}

/// Call of Duty style "relative" ADS sensitivity: how much slower the look
/// should turn at vertical FOV `fov` than at the hip's `hip` (both radians),
/// so the same mouse movement sweeps the same distance on screen at a chosen
/// point — `coefficient` vertical half-screens out from the middle (`0` the
/// centre itself: the ratio of the zooms; `1.33` CoD's default). A barely
/// zoomed iron sight stays near 1, a big scope drops far below it.
pub(crate) fn zoom_sens_scale(fov: f32, hip: f32, coefficient: f32) -> f32 {
    let half_tan = |f: f32| (f * 0.5).tan();
    let scale = if coefficient <= 1e-3 {
        half_tan(fov) / half_tan(hip)
    } else {
        (coefficient * half_tan(fov)).atan() / (coefficient * half_tan(hip)).atan()
    };
    if scale.is_finite() {
        scale.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn look_around(
    mouse_motion: Res<AccumulatedMouseMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    ads: Res<Ads>,
    settings: Res<Settings>,
    world_projection: Single<&Projection, With<WorldModelCamera>>,
    mut look_delta: ResMut<LookDelta>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    // Ignore look input while the cursor is free.
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }

    let delta = mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    // Base sensitivity × the player's multiplier, slowed by however much the
    // view's zoomed right now (it follows the ADS blend, and each weapon's own
    // zoom — the sniper's scope, the AK's iron sights), with the player's ADS
    // multiplier eased in on top as they aim.
    let fov = match *world_projection {
        Projection::Perspective(ref p) => p.fov,
        _ => settings.fov.to_radians(),
    };
    let zoom = zoom_sens_scale(fov, settings.fov.to_radians(), settings.ads_sens_coefficient);
    let sens = MOUSE_SENSITIVITY
        * settings.sensitivity
        * zoom
        * 1.0f32.lerp(settings.ads_sens_multiplier, ease(ads.t));

    // Yaw on the body...
    let yaw = -delta.x * sens.x;
    player.rotate_y(yaw);

    // ...pitch on the head, clamped so we can't flip over.
    let (_, current_pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
    let new_pitch = (current_pitch - delta.y * sens.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    head.rotation = Quat::from_rotation_x(new_pitch);

    // Hand the actual applied rotation to the sway (pitch measured after the
    // clamp so pinning against the limit doesn't keep feeding it).
    look_delta.applied = Vec2::new(yaw, new_pitch - current_pitch);
}

/// Clear this frame's [`LookDelta`] after [`weapon_sway`] and [`track_trick`]
/// have read it. `look_around` early-returns on a still frame without
/// writing, so without this the last turn would keep feeding them forever.
pub(crate) fn consume_look_delta(mut look: ResMut<LookDelta>) {
    look.applied = Vec2::ZERO;
}
