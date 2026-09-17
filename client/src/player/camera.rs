//! Mouse look: yaw on the player body, pitch on the head, ADS-scaled
//! sensitivity — plus the render-camera markers hanging off the head.

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

pub(crate) fn look_around(
    mouse_motion: Res<AccumulatedMouseMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    ads: Res<Ads>,
    settings: Res<Settings>,
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

    // Base sensitivity × the player's multiplier, eased toward the player's ADS
    // sensitivity multiplier as they zoom in.
    let sens = MOUSE_SENSITIVITY
        * settings.sensitivity
        * 1.0f32.lerp(settings.ads_sensitivity, ease(ads.t));

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
