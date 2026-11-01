//! The voice effect: an echo and a big reverb that spoken lines go through
//! on top of their dry sound (and the room's reverb) — the Hierophant's
//! lines, and any other clip the mix marks `voice_fx`. The pitch layers that
//! go with it are baked into the files beforehand
//! (`client/tools/voice_layers.sh`); this adds the space around them, so
//! every line gets exactly the same, tunable live.
//!
//! Two send tracks, each fully wet, fed at full level by every voice-FX
//! sound: one with a delay (a low-pass in its feedback, so each repeat
//! comes back darker), one with a reverb. Each send track's volume is how
//! loud that effect is. Being sends, their tails ring out after the line
//! itself has finished or been cut off.
//!
//! Tuned in the debug panel's "Audio → Voice effect". The settings are the
//! engine's, not a match's — nothing to reset between games.

use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use kira::effect::delay::{DelayBuilder, DelayHandle};
use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::track::{SendTrackBuilder, SendTrackHandle, TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, Tween};

use super::{AudioEngine, MAX_SOUNDS};

pub(super) struct VoiceFxPlugin;

impl Plugin for VoiceFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VoiceFxSettings>()
            .init_resource::<EchoRebuild>()
            .add_systems(Update, apply_voice_fx);
    }
}

/// "Audio → Voice effect" in the debug panel.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct VoiceFxSettings {
    pub(crate) enabled: bool,
    /// Time between repeats (ms). Changing it rebuilds the echo, which cuts
    /// off any voice playing at that moment.
    pub(crate) echo_ms: f32,
    /// How much of each repeat comes back for the next (dB; lower = fewer
    /// repeats).
    pub(crate) echo_feedback_db: f32,
    /// The low-pass in the echo's feedback (Hz): each repeat loses its
    /// highs above this.
    pub(crate) echo_tone_hz: f32,
    /// The echo's level next to the dry voice (dB).
    pub(crate) echo_level_db: f32,
    /// How long the reverb rings (kira's reverb feedback, `0`–`0.99`).
    pub(crate) reverb_decay: f32,
    /// How quickly its highs die away (`0` bright … `1` muffled).
    pub(crate) reverb_damping: f32,
    /// How wide it spreads (`0` mono … `1` full stereo).
    pub(crate) reverb_width: f32,
    /// The reverb's level next to the dry voice (dB).
    pub(crate) reverb_level_db: f32,
}

impl Default for VoiceFxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            echo_ms: 340.0,
            echo_feedback_db: -8.0,
            echo_tone_hz: 3_000.0,
            echo_level_db: -9.0,
            reverb_decay: 0.92,
            reverb_damping: 0.3,
            reverb_width: 1.0,
            reverb_level_db: -3.0,
        }
    }
}

/// The voice effect's tracks, held by the [`AudioEngine`].
pub(super) struct VoiceFxTracks {
    pub(super) echo_send: SendTrackHandle,
    echo: DelayHandle,
    echo_tone: FilterHandle,
    pub(super) reverb_send: SendTrackHandle,
    reverb: ReverbHandle,
    /// Where non-positional voice-FX sounds play, feeding both sends.
    pub(super) track: TrackHandle,
    /// The echo time it was built with (ms).
    built_ms: f32,
}

impl VoiceFxTracks {
    pub(super) fn new(manager: &mut AudioManager, s: &VoiceFxSettings) -> Result<Self, String> {
        let mut reverb_builder = SendTrackBuilder::new().volume(Decibels::SILENCE);
        let reverb = reverb_builder.add_effect(ReverbBuilder::new().mix(kira::Mix::WET));
        let reverb_send = manager.add_send_track(reverb_builder).map_err(|e| e.to_string())?;
        let (echo_send, echo, echo_tone, track) = Self::echo(manager, s, &reverb_send)?;
        let mut tracks = Self {
            echo_send,
            echo,
            echo_tone,
            reverb_send,
            reverb,
            track,
            built_ms: s.echo_ms,
        };
        tracks.apply(s);
        Ok(tracks)
    }

    /// A fresh echo send (at `s.echo_ms`), and the non-positional voice track
    /// feeding it and the reverb.
    #[allow(clippy::type_complexity)]
    fn echo(
        manager: &mut AudioManager,
        s: &VoiceFxSettings,
        reverb_send: &SendTrackHandle,
    ) -> Result<(SendTrackHandle, DelayHandle, FilterHandle, TrackHandle), String> {
        let mut delay = DelayBuilder::new()
            .delay_time(Duration::from_secs_f32(s.echo_ms.clamp(10.0, 2_000.0) / 1_000.0))
            .mix(kira::Mix::WET);
        let echo_tone = delay.add_feedback_effect(FilterBuilder::new().cutoff(s.echo_tone_hz as f64));
        let mut builder = SendTrackBuilder::new().volume(Decibels::SILENCE);
        let echo = builder.add_effect(delay);
        let echo_send = manager.add_send_track(builder).map_err(|e| e.to_string())?;
        let track = manager
            .add_sub_track(
                TrackBuilder::new()
                    .sound_capacity(MAX_SOUNDS)
                    .with_send(echo_send.id(), Decibels::IDENTITY)
                    .with_send(reverb_send.id(), Decibels::IDENTITY),
            )
            .map_err(|e| e.to_string())?;
        Ok((echo_send, echo, echo_tone, track))
    }

    /// Everything but the echo time, live.
    fn apply(&mut self, s: &VoiceFxSettings) {
        let tween = Tween::default();
        let level = |db: f32| if s.enabled { Decibels(db) } else { Decibels::SILENCE };
        self.echo_send.set_volume(level(s.echo_level_db), tween);
        self.echo.set_feedback(Decibels(s.echo_feedback_db.min(-0.5)), tween);
        self.echo_tone.set_cutoff(s.echo_tone_hz.clamp(100.0, 20_000.0) as f64, tween);
        self.reverb_send.set_volume(level(s.reverb_level_db), tween);
        self.reverb.set_feedback(s.reverb_decay.clamp(0.0, 0.99) as f64, tween);
        self.reverb.set_damping(s.reverb_damping.clamp(0.0, 1.0) as f64, tween);
        self.reverb.set_stereo_width(s.reverb_width.clamp(0.0, 1.0) as f64, tween);
    }
}

/// When the echo time was last changed (for rebuilding it once the slider's
/// settled, rather than on every step of a drag).
#[derive(Resource, Default)]
struct EchoRebuild {
    changed_at: Option<f32>,
}

/// How long the echo time has to stay put before the echo's rebuilt (s).
const REBUILD_AFTER_SECS: f32 = 0.4;

fn apply_voice_fx(
    time: Res<Time>,
    settings: Res<VoiceFxSettings>,
    mut rebuild: ResMut<EchoRebuild>,
    engine: Option<ResMut<AudioEngine>>,
) {
    let Some(mut engine) = engine else { return };
    let engine = &mut *engine;
    let s = *settings;
    let now = time.elapsed_secs();
    if settings.is_changed() {
        engine.voice.apply(&s);
        if s.echo_ms != engine.voice.built_ms {
            rebuild.changed_at = Some(now);
        }
    }
    if rebuild.changed_at.is_some_and(|at| now - at >= REBUILD_AFTER_SECS) {
        rebuild.changed_at = None;
        match VoiceFxTracks::echo(&mut engine.manager, &s, &engine.voice.reverb_send) {
            Ok((echo_send, echo, echo_tone, track)) => {
                // (The old ones go as their handles drop.)
                engine.voice.echo_send = echo_send;
                engine.voice.echo = echo;
                engine.voice.echo_tone = echo_tone;
                engine.voice.track = track;
                engine.voice.built_ms = s.echo_ms;
                engine.voice.apply(&s);
            }
            Err(e) => warn!("couldn't rebuild the voice echo: {e}"),
        }
    }
}

/// The debug panel's "Audio → Voice effect" section.
#[derive(SystemParam)]
pub(crate) struct VoiceFxDebug<'w> {
    settings: ResMut<'w, VoiceFxSettings>,
}

impl VoiceFxDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Echo + reverb on the spoken lines (clips the mixer marks \"voice FX\": the \
             Hierophant's voice/ folder). The ±3 semitone layers are baked into the files \
             by client/tools/voice_layers.sh. Changes apply live; the echo time takes a \
             moment and cuts off a line playing right then.",
        );
        // (Only write back on a real change, so the effect isn't re-applied
        // every frame the panel's open.)
        let mut s = *self.settings;
        ui.checkbox(&mut s.enabled, "enabled");
        ui.separator();
        ui.label("Echo");
        ui.add(egui::Slider::new(&mut s.echo_ms, 40.0f32..=1_200.0).text("time between repeats (ms)"));
        ui.add(egui::Slider::new(&mut s.echo_feedback_db, -40.0f32..=-1.0).text("feedback (dB)"));
        ui.add(
            egui::Slider::new(&mut s.echo_tone_hz, 300.0f32..=20_000.0)
                .logarithmic(true)
                .text("repeats darken above (Hz)"),
        );
        ui.add(egui::Slider::new(&mut s.echo_level_db, -40.0f32..=6.0).text("echo level (dB)"));
        ui.separator();
        ui.label("Reverb");
        ui.add(egui::Slider::new(&mut s.reverb_decay, 0.0f32..=0.99).text("decay (feedback)"));
        ui.add(egui::Slider::new(&mut s.reverb_damping, 0.0f32..=1.0).text("damping (darker)"));
        ui.add(egui::Slider::new(&mut s.reverb_width, 0.0f32..=1.0).text("stereo width"));
        ui.add(egui::Slider::new(&mut s.reverb_level_db, -40.0f32..=6.0).text("reverb level (dB)"));
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                s = VoiceFxSettings::default();
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
