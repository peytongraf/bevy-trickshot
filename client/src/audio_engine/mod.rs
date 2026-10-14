//! Sound playback on [kira](https://docs.rs/kira), behind the same ECS shape
//! bevy_audio had (`bevy_audio` is switched off in `Cargo.toml`): spawn an
//! [`AudioPlayer`] with [`PlaybackSettings`], and once it starts it gets an
//! [`AudioSink`] (or a [`SpatialAudioSink`] for `spatial` ones) to steer it
//! by. Despawning the entity stops the sound, and `PlaybackMode::Despawn` /
//! `Remove` clean up after it the same way bevy_audio did.
//!
//! Positional sounds each play on their own kira spatial track, panned
//! around the [`SpatialListener`]. Their loudness by distance is still the
//! game's own business (each feature's falloff, baked into its volume), as
//! it was under rodio: the tracks have no distance attenuation of their own.
//!
//! Every sound in the world — every positional one, plus non-positional ones
//! flagged [`PlaybackSettings::reverb`] (our own shots, footsteps, ...) —
//! also feeds one shared reverb, shaped by the space the listener is in
//! ([`room`]). UI, music and announcers stay dry.
//!
//! Clips decode as they play ([`decoder`]) rather than being held decoded in
//! memory.

mod decoder;
mod room;

use std::sync::Arc;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::transform::TransformSystem;
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::listener::ListenerHandle;
use kira::track::{
    MainTrackBuilder, SendTrackBuilder, SendTrackHandle, SpatialTrackBuilder, SpatialTrackHandle,
    TrackBuilder, TrackHandle,
};
use kira::{AudioManager, AudioManagerSettings, Capacities, Decibels, DefaultBackend, Mix, Tween};

use decoder::{DecodingSoundData, Playback};
pub(crate) use room::ReverbDebug;

/// What call sites need: `use crate::audio_engine::prelude::*;`.
pub(crate) mod prelude {
    pub(crate) use super::{
        AudioPlayer, AudioSink, AudioSource, GlobalVolume, PlaybackMode, PlaybackSettings,
        SpatialAudioSink, Volume,
    };
}

/// Most sounds (and so tracks) that can be playing at once; past it a new
/// sound is dropped with a warning. Well above anything a busy `Zombies`
/// round has going.
const MAX_SOUNDS: usize = 1024;

/// Gain on every positional sound, so they sit at the level they did under
/// rodio. Its pan law put a sound dead ahead at half volume in each ear and
/// one off to the side at 0.75 / 0.25; kira's (at its default 0.75
/// spatialization strength) gives ~0.77 ahead and ~0.97 / 0.28 to the side,
/// so this pulls it back down by about the same ratio.
const SPATIAL_GAIN: f32 = 0.65;

pub(crate) struct AudioEnginePlugin;

impl Plugin for AudioEnginePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(room::RoomReverbPlugin)
            .init_asset::<AudioSource>()
            .register_asset_loader(AudioLoader)
            .init_resource::<GlobalVolume>()
            .add_systems(
                PostUpdate,
                (update_listener, update_emitters, play_queued, cleanup_finished)
                    .chain()
                    .after(TransformSystem::TransformPropagate),
            );
        match AudioEngine::new() {
            Ok(engine) => {
                app.insert_resource(engine);
            }
            Err(e) => warn!("no audio output, sounds won't play: {e}"),
        }
    }
}

/// The kira mixer, and the one listener every spatial track pans around.
/// Missing if no output device could be opened (nothing plays then, as with
/// bevy_audio).
#[derive(Resource)]
struct AudioEngine {
    manager: AudioManager,
    listener: ListenerHandle,
    /// The shared reverb: a send track, fully wet, that every world sound
    /// feeds at full level — its volume is the reverb's level. Both are
    /// steered by [`room`].
    reverb_send: SendTrackHandle,
    reverb: ReverbHandle,
    /// Where non-positional world sounds play, so they feed the reverb too
    /// (the rest play straight on the main track).
    world: TrackHandle,
}

impl AudioEngine {
    fn new() -> Result<Self, String> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings {
            capacities: Capacities {
                // (+1: the `world` track.)
                sub_track_capacity: MAX_SOUNDS + 1,
                ..default()
            },
            main_track_builder: MainTrackBuilder::new().sound_capacity(MAX_SOUNDS),
            ..default()
        })
        .map_err(|e| e.to_string())?;
        let listener = manager
            .add_listener(mint_vec(Vec3::ZERO), mint_quat(Quat::IDENTITY))
            .map_err(|e| e.to_string())?;
        let mut send = SendTrackBuilder::new().volume(Decibels::SILENCE);
        let reverb = send.add_effect(ReverbBuilder::new().mix(Mix::WET));
        let reverb_send = manager.add_send_track(send).map_err(|e| e.to_string())?;
        let world = manager
            .add_sub_track(
                TrackBuilder::new()
                    .sound_capacity(MAX_SOUNDS)
                    .sub_track_capacity(1)
                    .with_send(reverb_send.id(), Decibels::IDENTITY),
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            manager,
            listener,
            reverb_send,
            reverb,
            world,
        })
    }
}

/// An audio file (`.mp3` / `.wav` / `.ogg` / `.flac`), kept as its encoded
/// bytes — decoded only while it plays.
#[derive(Asset, TypePath)]
pub(crate) struct AudioSource {
    bytes: Arc<[u8]>,
}

struct AudioLoader;

impl AssetLoader for AudioLoader {
    type Asset = AudioSource;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<AudioSource, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let bytes: Arc<[u8]> = bytes.into();
        // Fail the load now on a file we can't decode, rather than silently
        // at every play.
        decoder::open(&bytes).map_err(std::io::Error::other)?;
        Ok(AudioSource { bytes })
    }

    fn extensions(&self) -> &[&str] {
        &["mp3", "wav", "ogg", "oga", "flac"]
    }
}

/// A linear volume (`1.0` = the clip's own level).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Volume {
    Linear(f32),
}

impl Volume {
    pub(crate) fn to_linear(self) -> f32 {
        match self {
            Volume::Linear(v) => v,
        }
    }
}

impl std::ops::Mul for Volume {
    type Output = Volume;

    fn mul(self, rhs: Volume) -> Volume {
        Volume::Linear(self.to_linear() * rhs.to_linear())
    }
}

/// Master volume, multiplied into every sound's [`PlaybackSettings::volume`]
/// as it starts. Doesn't touch what's already playing.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct GlobalVolume {
    pub(crate) volume: Volume,
}

impl Default for GlobalVolume {
    fn default() -> Self {
        Self {
            volume: Volume::Linear(1.0),
        }
    }
}

/// Plays a clip, starting as soon as it's loaded (see the module docs).
#[derive(Component, Clone, Debug)]
#[require(PlaybackSettings)]
pub(crate) struct AudioPlayer(pub(crate) Handle<AudioSource>);

impl AudioPlayer {
    pub(crate) fn new(clip: Handle<AudioSource>) -> Self {
        Self(clip)
    }
}

/// What happens when a sound reaches its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaybackMode {
    /// Play once, then leave the entity be.
    Once,
    /// Repeat forever.
    Loop,
    /// Despawn the entity (and its children).
    Despawn,
    /// Remove the audio components, leaving the rest of the entity.
    Remove,
}

/// How an [`AudioPlayer`] starts. Changing it afterwards does nothing — steer
/// a playing sound through its sink.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct PlaybackSettings {
    pub(crate) mode: PlaybackMode,
    /// Multiplied by [`GlobalVolume`] when it starts.
    pub(crate) volume: Volume,
    /// Playback rate — also shifts the pitch.
    pub(crate) speed: f32,
    pub(crate) paused: bool,
    /// Panned from the entity's `GlobalTransform` around the
    /// [`SpatialListener`], following it as it moves. Always feeds the
    /// room's reverb.
    pub(crate) spatial: bool,
    /// A non-positional sound that's still happening in the world (our own
    /// shot, footstep, reload, ...), so it feeds the room's reverb. Leave it
    /// off for UI, music, announcers and the like.
    pub(crate) reverb: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self::ONCE
    }
}

impl PlaybackSettings {
    pub(crate) const ONCE: Self = Self {
        mode: PlaybackMode::Once,
        volume: Volume::Linear(1.0),
        speed: 1.0,
        paused: false,
        spatial: false,
        reverb: false,
    };
    pub(crate) const LOOP: Self = Self {
        mode: PlaybackMode::Loop,
        ..Self::ONCE
    };
    pub(crate) const DESPAWN: Self = Self {
        mode: PlaybackMode::Despawn,
        ..Self::ONCE
    };

    pub(crate) const fn with_volume(self, volume: Volume) -> Self {
        Self { volume, ..self }
    }

    pub(crate) const fn with_speed(self, speed: f32) -> Self {
        Self { speed, ..self }
    }

    pub(crate) const fn with_spatial(self, spatial: bool) -> Self {
        Self { spatial, ..self }
    }

    pub(crate) const fn with_reverb(self, reverb: bool) -> Self {
        Self { reverb, ..self }
    }
}

/// The ears spatial sounds are heard from — on the player's camera. Kira
/// takes its orientation as-is (−Z ahead, +X right, Bevy's camera
/// convention). One at a time.
#[derive(Component, Clone, Copy, Debug, Default)]
pub(crate) struct SpatialListener;

/// Steers a playing non-spatial sound. Dropping it (despawning the entity,
/// or removing it) stops the sound.
#[derive(Component)]
pub(crate) struct AudioSink {
    playback: Arc<Playback>,
}

/// Steers a playing spatial sound; its emitter position follows the
/// entity's `GlobalTransform`. Dropping it stops the sound.
#[derive(Component)]
pub(crate) struct SpatialAudioSink {
    playback: Arc<Playback>,
    track: SpatialTrackHandle,
}

impl Drop for AudioSink {
    fn drop(&mut self) {
        self.playback.stop();
    }
}

impl Drop for SpatialAudioSink {
    fn drop(&mut self) {
        // (The track goes once the sound's faded out — it persists until
        // its sounds finish.)
        self.playback.stop();
    }
}

/// The controls on [`AudioSink`] and [`SpatialAudioSink`] (the names
/// bevy_audio's `AudioSinkPlayback` had).
macro_rules! sink_controls {
    ($sink:ty) => {
        // (Not every control is used on both kinds of sink.)
        #[allow(dead_code)]
        impl $sink {
            /// The volume it's playing at (`GlobalVolume` included, as set).
            pub(crate) fn volume(&self) -> Volume {
                Volume::Linear(self.playback.volume())
            }

            /// Change the volume (eased across one audio buffer, a few
            /// milliseconds). `GlobalVolume` isn't applied — multiply it in.
            pub(crate) fn set_volume(&mut self, volume: Volume) {
                self.playback.set_volume(volume.to_linear().max(0.0));
            }

            /// Resume after [`pause`](Self::pause).
            pub(crate) fn play(&self) {
                self.playback.set_paused(false);
            }

            /// Hold it where it is (silent, and not decoding).
            pub(crate) fn pause(&self) {
                self.playback.set_paused(true);
            }

            pub(crate) fn is_paused(&self) -> bool {
                self.playback.is_paused()
            }

            /// Whether it's done — played out, or stopped.
            pub(crate) fn empty(&self) -> bool {
                self.playback.is_finished()
            }
        }
    };
}

sink_controls!(AudioSink);
sink_controls!(SpatialAudioSink);

/// Start every [`AudioPlayer`] whose clip has loaded.
fn play_queued(
    mut commands: Commands,
    engine: Option<ResMut<AudioEngine>>,
    sources: Res<Assets<AudioSource>>,
    global: Res<GlobalVolume>,
    queued: Query<
        (Entity, &AudioPlayer, &PlaybackSettings, Option<&GlobalTransform>),
        (Without<AudioSink>, Without<SpatialAudioSink>),
    >,
) {
    let Some(mut engine) = engine else { return };
    let engine = &mut *engine;
    for (entity, player, settings, transform) in &queued {
        let Some(source) = sources.get(&player.0) else {
            continue;
        };
        let clip = match decoder::open(&source.bytes) {
            Ok(clip) => clip,
            Err(e) => {
                warn!("can't decode {:?}: {e}", player.0.path());
                end_playback(commands.entity(entity), settings.mode);
                continue;
            }
        };
        let playback = Arc::new(Playback::new(
            (settings.volume * global.volume).to_linear().max(0.0),
            settings.paused,
        ));
        let data = DecodingSoundData {
            clip,
            speed: settings.speed,
            looping: settings.mode == PlaybackMode::Loop,
            playback: playback.clone(),
        };
        let mut entity = commands.entity(entity);
        if settings.spatial {
            let position = transform.map_or(Vec3::ZERO, GlobalTransform::translation);
            let track = engine.manager.add_spatial_sub_track(
                &engine.listener,
                mint_vec(position),
                SpatialTrackBuilder::new()
                    .sound_capacity(1)
                    .sub_track_capacity(1)
                    .persist_until_sounds_finish(true)
                    .attenuation_function(None)
                    .volume(Decibels(20.0 * SPATIAL_GAIN.log10()))
                    .with_send(engine.reverb_send.id(), Decibels::IDENTITY),
            );
            let Ok(mut track) = track else {
                warn!("too many sounds playing; dropped {:?}", player.0.path());
                end_playback(entity, settings.mode);
                continue;
            };
            if track.play(data).is_err() {
                end_playback(entity, settings.mode);
                continue;
            }
            entity.insert(SpatialAudioSink { playback, track });
        } else {
            let played = if settings.reverb {
                engine.world.play(data).is_ok()
            } else {
                engine.manager.play(data).is_ok()
            };
            if !played {
                warn!("too many sounds playing; dropped {:?}", player.0.path());
                end_playback(entity, settings.mode);
                continue;
            }
            entity.insert(AudioSink { playback });
        }
    }
}

/// Follow the [`SpatialListener`] (the player's camera).
fn update_listener(
    engine: Option<ResMut<AudioEngine>>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
) {
    let (Some(mut engine), Ok(ears)) = (engine, listener.single()) else {
        return;
    };
    let (_, rotation, position) = ears.to_scale_rotation_translation();
    engine.listener.set_position(mint_vec(position), Tween::default());
    engine.listener.set_orientation(mint_quat(rotation), Tween::default());
}

/// Move each spatial sound's emitter with its entity.
fn update_emitters(
    mut emitters: Query<(&GlobalTransform, &mut SpatialAudioSink), Changed<GlobalTransform>>,
) {
    for (transform, mut sink) in &mut emitters {
        sink.track.set_position(mint_vec(transform.translation()), Tween::default());
    }
}

/// Despawn / strip each finished `Despawn` / `Remove` sound.
fn cleanup_finished(
    mut commands: Commands,
    sounds: Query<(Entity, &PlaybackSettings, Option<&AudioSink>, Option<&SpatialAudioSink>)>,
) {
    for (entity, settings, sink, spatial) in &sounds {
        if !matches!(settings.mode, PlaybackMode::Despawn | PlaybackMode::Remove) {
            continue;
        }
        let finished = sink.map(AudioSink::empty)
            .or(spatial.map(SpatialAudioSink::empty))
            .unwrap_or(false);
        if finished {
            end_playback(commands.entity(entity), settings.mode);
        }
    }
}

/// What a sound's [`PlaybackMode`] does once it's over — or once it's failed
/// to start at all, so it isn't retried every frame.
fn end_playback(mut entity: EntityCommands, mode: PlaybackMode) {
    match mode {
        PlaybackMode::Despawn => {
            entity.try_despawn();
        }
        PlaybackMode::Remove => {
            entity.remove::<(AudioPlayer, PlaybackSettings, AudioSink, SpatialAudioSink)>();
        }
        // (Leaves `Once` drained, as bevy_audio did; a failed `Loop` just
        // doesn't play.)
        PlaybackMode::Once | PlaybackMode::Loop => {
            entity.remove::<AudioPlayer>();
        }
    }
}

fn mint_vec(v: Vec3) -> mint::Vector3<f32> {
    mint::Vector3 { x: v.x, y: v.y, z: v.z }
}

fn mint_quat(q: Quat) -> mint::Quaternion<f32> {
    mint::Quaternion {
        v: mint::Vector3 { x: q.x, y: q.y, z: q.z },
        s: q.w,
    }
}
