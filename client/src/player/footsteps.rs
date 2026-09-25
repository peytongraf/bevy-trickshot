//! Footstep cadence + audio for the local player (Call-of-Duty style: a
//! distance accumulator that fires a step every `stride` metres).

use bevy::audio::Volume;
use bevy::prelude::*;

use crate::killcam;
use crate::util::rand01;
use crate::GameSounds;

use super::movement::{Player, PlayerPhysics, Sprinting};
use super::slide::{Slide, Stance};

/// How many `audio/movement/footsteps/footstep_N.wav` clips there are (1-indexed).
pub(crate) const FOOTSTEP_CLIPS: usize = 10;

/// Panel-adjustable footstep audio ("Footsteps" panel section). One step plays
/// every `*_stride` metres travelled on foot, so cadence rises with speed and
/// each stance (crouch / walk / sprint / prone) gets its own pace and loudness —
/// the Call-of-Duty model.
#[derive(Resource)]
pub(crate) struct FootstepSettings {
    /// Master on / off for footstep audio.
    pub(crate) enabled: bool,
    /// Overall volume, multiplying every per-stance level below.
    pub(crate) volume: f32,
    /// Metres travelled between steps, per stance.
    pub(crate) walk_stride: f32,
    pub(crate) sprint_stride: f32,
    pub(crate) crouch_stride: f32,
    pub(crate) prone_stride: f32,
    /// Per-stance loudness (linear, before `volume`).
    pub(crate) walk_volume: f32,
    pub(crate) sprint_volume: f32,
    pub(crate) crouch_volume: f32,
    pub(crate) prone_volume: f32,
    /// Random playback-rate (pitch) spread, ± this around 1.0, so repeats of the
    /// same clip don't sound identical.
    pub(crate) pitch_jitter: f32,
    /// Planar speed (m/s) below which the player counts as stopped.
    pub(crate) min_speed: f32,
}

impl Default for FootstepSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 3.0,
            walk_stride: 2.0,
            sprint_stride: 2.5,
            crouch_stride: 1.5,
            prone_stride: 1.2,
            walk_volume: 0.5,
            sprint_volume: 0.85,
            crouch_volume: 0.28,
            prone_volume: 0.2,
            pitch_jitter: 0.12,
            min_speed: 0.5,
        }
    }
}

/// Running footstep cadence state for the local player.
#[derive(Resource, Default)]
pub(crate) struct FootstepState {
    /// Distance (m) covered since the last step.
    accum: f32,
    /// Whether the player was moving on foot last frame — a fresh start fires a
    /// step immediately rather than after a full stride of silence.
    was_moving: bool,
    /// Index of the last clip played, so the next pick avoids an instant repeat.
    last: usize,
    /// Bumped each step, seeds the clip pick + pitch jitter.
    seq: u32,
}

/// Spawn one random footstep clip: a fresh pick that isn't an instant repeat, at
/// `volume`, with a small random pitch wobble so back-to-back steps differ.
pub(crate) fn play_footstep(
    commands: &mut Commands,
    clips: &[Handle<AudioSource>],
    state: &mut FootstepState,
    volume: f32,
    pitch_jitter: f32,
) {
    if clips.is_empty() {
        return;
    }
    state.seq = state.seq.wrapping_add(1);
    let s = state.seq.wrapping_mul(2_654_435_761).wrapping_add(0xf007);
    let mut idx = (rand01(s) * clips.len() as f32) as usize % clips.len();
    if clips.len() > 1 && idx == state.last {
        idx = (idx + 1) % clips.len();
    }
    state.last = idx;
    let pitch = 1.0 + (rand01(s ^ 0x5bd1_e995) * 2.0 - 1.0) * pitch_jitter;
    commands.spawn((
        AudioPlayer::new(clips[idx].clone()),
        PlaybackSettings::DESPAWN
            .with_volume(Volume::Linear(volume.max(0.0)))
            .with_speed(pitch.clamp(0.1, 4.0)),
    ));
}

/// Footstep cadence for the local player, Call-of-Duty style: a distance
/// accumulator releases a step every `stride` metres, so the pace tracks the
/// player's actual speed and each stance (crouch-walk / walk / sprint / prone)
/// gets its own stride length and loudness. No steps while airborne, sliding,
/// diving or standing still; the first step after a standstill fires at once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn footsteps(
    time: Res<Time>,
    cfg: Res<FootstepSettings>,
    sounds: Res<GameSounds>,
    slide: Res<Slide>,
    sprinting: Res<Sprinting>,
    physics: Single<&PlayerPhysics, With<Player>>,
    mut state: ResMut<FootstepState>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    mut commands: Commands,
) {
    let planar = Vec3::new(
        physics.horizontal_velocity.x,
        0.0,
        physics.horizontal_velocity.z,
    );
    let speed = planar.length();
    let on_foot = cfg.enabled
        && physics.grounded
        && !matches!(slide.stance, Stance::Sliding | Stance::Diving)
        && speed > cfg.min_speed;
    if !on_foot {
        state.accum = 0.0;
        state.was_moving = false;
        return;
    }

    let (stride, stance_vol) = match slide.stance {
        Stance::Crouching => (cfg.crouch_stride, cfg.crouch_volume),
        Stance::Prone => (cfg.prone_stride, cfg.prone_volume),
        _ if sprinting.0 => (cfg.sprint_stride, cfg.sprint_volume),
        _ => (cfg.walk_stride, cfg.walk_volume),
    };
    let stride = stride.max(0.1);
    let volume = (stance_vol * cfg.volume).max(0.0);

    if !state.was_moving {
        state.was_moving = true;
        state.accum = 0.0;
        play_footstep(
            &mut commands,
            &sounds.footsteps,
            &mut state,
            volume,
            cfg.pitch_jitter,
        );
        snd.note(killcam::SND_FOOTSTEP);
        return;
    }

    state.accum += speed * time.delta_secs();
    // `while`, not `if`, so a big frame hitch at speed still spaces steps evenly
    // rather than dropping them; cap the catch-up so it can't spam on a stall.
    let mut budget = 4;
    while state.accum >= stride && budget > 0 {
        state.accum -= stride;
        budget -= 1;
        play_footstep(
            &mut commands,
            &sounds.footsteps,
            &mut state,
            volume,
            cfg.pitch_jitter,
        );
        snd.note(killcam::SND_FOOTSTEP);
    }
    state.accum = state.accum.min(stride);
}
