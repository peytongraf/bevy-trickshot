//! Tracks the player's spin for trickshot style points.

use bevy::prelude::*;

use super::camera::LookDelta;
use super::movement::{Player, PlayerPhysics};

/// A yaw run of at least this many degrees before a direction reversal is
/// "banked" toward the trick total (roughly a clean 180).
pub(crate) const TRICK_MIN_RUN_DEG: f32 = 135.0;
/// Per-frame yaw change below this (deg) is treated as not turning.
pub(crate) const TRICK_TURN_EPS_DEG: f32 = 0.05;
/// Seconds of not-really-turning that ends the current trick.
pub(crate) const TRICK_IDLE_RESET_SECS: f32 = 0.4;

/// Tracks the player's spin for style points: degrees turned in the current
/// run, plus banked degrees from earlier runs of the same trick (so a 180 one
/// way then a 180 the other still adds up). Reset on every shot fired and
/// whenever the player stops turning for a moment.
#[derive(Resource, Default)]
pub(crate) struct TrickState {
    /// Sign of the current run (+1 / -1 / 0).
    dir: f32,
    /// Unsigned degrees turned in the current run.
    run_deg: f32,
    /// Sum of completed runs (each ≥ `TRICK_MIN_RUN_DEG`) this trick.
    banked_deg: f32,
    /// Any part of this trick happened airborne.
    pub(crate) airborne: bool,
    /// Seconds spent below the turn threshold.
    idle: f32,
}

impl TrickState {
    /// Total spin credited if a shot lands right now.
    pub(crate) fn total_deg(&self) -> f32 {
        self.banked_deg + self.run_deg
    }

    /// Start a fresh trick.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(crate) fn reset_trick(mut trick: ResMut<TrickState>) {
    *trick = TrickState::default();
}

/// Accumulate the player's yaw spin for style points (see [`TrickState`]).
///
/// Reads the exact yaw turned this frame straight from [`LookDelta`] — the
/// unwrapped value `look_around` just applied via `rotate_y` — rather than
/// diffing two `Transform::rotation` readings frame to frame. A `Transform`'s
/// yaw is wrapped to `(-180°, 180°]`, so re-deriving a delta from it can only
/// ever recover a turn *modulo 360°, folded to its shortest equivalent*: a
/// hard spin that turns the player more than 180° in a single frame (easy to
/// do at real trickshot speed, especially at higher sensitivity) reads back as
/// a much smaller turn *the other way*. That looked like a phantom direction
/// reversal, which got banked as an extra "run" on top of the real spin —
/// turning one clean 360 into wildly inflated totals like 1260°.
/// [`LookDelta::applied`] can't alias like that: it's the exact value applied,
/// never round-tripped through a wrapped angle.
pub(crate) fn track_trick(
    time: Res<Time>,
    look: Res<LookDelta>,
    physics: Single<&PlayerPhysics, With<Player>>,
    mut trick: ResMut<TrickState>,
) {
    let deg = look.applied.x.to_degrees();
    if deg.abs() < TRICK_TURN_EPS_DEG {
        trick.idle += time.delta_secs();
        if trick.idle >= TRICK_IDLE_RESET_SECS {
            trick.reset();
        }
        return;
    }
    trick.idle = 0.0;
    if !physics.grounded {
        trick.airborne = true;
    }

    let s = deg.signum();
    if trick.dir == 0.0 || s == trick.dir {
        trick.dir = s;
        trick.run_deg += deg.abs();
    } else {
        // Direction reversed — bank a clean-enough run, then start a new one.
        if trick.run_deg >= TRICK_MIN_RUN_DEG {
            trick.banked_deg += trick.run_deg;
        }
        trick.dir = s;
        trick.run_deg = deg.abs();
    }
}
