//! Sound playback on [kira](https://docs.rs/kira), behind the same ECS shape
//! bevy_audio had (`bevy_audio` is switched off in `Cargo.toml`): spawn an
//! [`AudioPlayer`] with [`PlaybackSettings`], and once it starts it gets an
//! [`AudioSink`] (or a [`SpatialAudioSink`] for `spatial` ones) to steer it
//! by. Despawning the entity stops the sound, and `PlaybackMode::Despawn` /
//! `Remove` clean up after it the same way bevy_audio did.
//!
//! Positional sounds each play on their own kira spatial track, panned
//! around the [`SpatialListener`]. How they fade with distance (and lose
//! their highs to the air) is their [`mix::Profile`]'s, applied here every
//! frame — features play them at their plain volume and leave distance to
//! this.
//!
//! On top of the volume each sound's played at, master volume
//! ([`GlobalVolume`]) and the [`mix`] set its level (bus, folders, clip —
//! tuned live in the audio [`mixer`] and saved with the game), both live,
//! and whether it's a world sound: one that also feeds the shared
//! reverb, shaped by the space the listener is in ([`room`]). By default
//! positional sounds are world sounds and the rest are dry; the mix
//! overrides that per folder / clip (our own shots and footsteps are world
//! sounds; UI, music and announcers stay dry).
//!
//! Clips decode as they play ([`decoder`]) rather than being held decoded in
//! memory.

mod decoder;
pub(crate) mod mix;
mod mixer;
mod room;

use std::sync::Arc;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::transform::TransformSystem;
use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::listener::ListenerHandle;
use kira::track::{
    MainTrackBuilder, SendTrackBuilder, SendTrackHandle, SpatialTrackBuilder, SpatialTrackHandle,
    TrackBuilder, TrackHandle,
};
use kira::{AudioManager, AudioManagerSettings, Capacities, Decibels, DefaultBackend, Tween};

use decoder::{DecodingSoundData, Playback};
use mix::{clip_gain, clip_key, BusMonitor, Mix, AIR_OFF_HZ};
pub(crate) use mixer::AudioMixer;
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
        app.add_plugins((room::RoomReverbPlugin, mixer::MixerPlugin))
            .insert_resource(Mix::load())
            .init_resource::<BusMonitor>()
            .init_asset::<AudioSource>()
            .register_asset_loader(AudioLoader)
            .init_resource::<GlobalVolume>()
            .add_systems(
                PostUpdate,
                (update_listener, update_emitters, play_queued, apply_mix, apply_profiles, cleanup_finished)
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
        let reverb = send.add_effect(ReverbBuilder::new().mix(kira::Mix::WET));
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

/// Master volume, applied live on top of every sound (what's already
/// playing too) — never multiply it in yourself.
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
    /// Its own volume, before master volume, the mix and (positional) its
    /// profile's distance fade.
    pub(crate) volume: Volume,
    /// Playback rate — also shifts the pitch.
    pub(crate) speed: f32,
    pub(crate) paused: bool,
    /// Panned from the entity's `GlobalTransform` around the
    /// [`SpatialListener`], following it as it moves.
    pub(crate) spatial: bool,
    /// How far into the clip (0..1) it starts — so several copies of one
    /// loop don't all play the same thing at once.
    pub(crate) start: f32,
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
        start: 0.0,
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

    pub(crate) const fn with_start(self, start: f32) -> Self {
        Self { start, ..self }
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
    /// Its clip's key in the [`Mix`].
    clip: Arc<str>,
}

/// Steers a playing spatial sound; its emitter position follows the
/// entity's `GlobalTransform`. Dropping it stops the sound.
#[derive(Component)]
pub(crate) struct SpatialAudioSink {
    playback: Arc<Playback>,
    /// Its clip's key in the [`Mix`].
    clip: Arc<str>,
    /// Its [`mix::Profile`]'s name.
    profile: String,
    looping: bool,
    track: SpatialTrackHandle,
    /// Its air-absorption low-pass.
    filter: FilterHandle,
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
            /// The volume it was played / last set at.
            pub(crate) fn volume(&self) -> Volume {
                Volume::Linear(self.playback.volume())
            }

            /// Change the volume (eased across one audio buffer, a few
            /// milliseconds). Master volume, the mix and the distance fade
            /// still go on top.
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
#[allow(clippy::too_many_arguments)]
fn play_queued(
    mut commands: Commands,
    engine: Option<ResMut<AudioEngine>>,
    sources: Res<Assets<AudioSource>>,
    global: Res<GlobalVolume>,
    mix: Res<Mix>,
    monitor: Res<BusMonitor>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    queued: Query<
        (Entity, &AudioPlayer, &PlaybackSettings, Option<&GlobalTransform>),
        (Without<AudioSink>, Without<SpatialAudioSink>),
    >,
) {
    let Some(mut engine) = engine else { return };
    let engine = &mut *engine;
    let ears = listener.single().ok().map(GlobalTransform::translation);
    for (entity, player, settings, transform) in &queued {
        let Some(source) = sources.get(&player.0) else {
            continue;
        };
        let key: Arc<str> = player.0.path().map_or("".into(), |p| clip_key(&p.path().to_string_lossy()).into());
        let (mix_gain, resolved) = clip_gain(&mix, &monitor, &key, settings.spatial);
        let position = transform.map_or(Vec3::ZERO, GlobalTransform::translation);
        let profile = mix.profile(&resolved.profile);
        let distance = ears.map_or(0.0, |e| e.distance(position));
        // A positional one-shot that starts out of earshot is never heard.
        if settings.spatial && settings.mode == PlaybackMode::Despawn && profile.gain(distance) <= 0.0 {
            end_playback(commands.entity(entity), settings.mode);
            continue;
        }
        let mut clip = match decoder::open(&source.bytes) {
            Ok(clip) => clip,
            Err(e) => {
                warn!("can't decode {:?}: {e}", player.0.path());
                end_playback(commands.entity(entity), settings.mode);
                continue;
            }
        };
        if settings.start > 0.0 {
            clip.skip_to(settings.start);
        }
        let reverb = resolved.reverb.unwrap_or(settings.spatial);
        let looping = settings.mode == PlaybackMode::Loop;
        let playback = Arc::new(Playback::new(
            settings.volume.to_linear().max(0.0),
            mix_gain * global.volume.to_linear(),
            settings.paused,
        ));
        let data = DecodingSoundData {
            clip,
            speed: settings.speed,
            looping,
            playback: playback.clone(),
        };
        let mut entity = commands.entity(entity);
        if settings.spatial {
            playback.set_distance(profile.gain(distance));
            let mut builder = SpatialTrackBuilder::new()
                .sound_capacity(1)
                .sub_track_capacity(1)
                .persist_until_sounds_finish(true)
                .attenuation_function(None)
                .spatialization_strength(profile.width.clamp(0.0, 1.0))
                .volume(Decibels(20.0 * SPATIAL_GAIN.log10()));
            // Air absorption — bypassed (dry) until the profile wants it.
            let filter = builder.add_effect(FilterBuilder::new().cutoff(AIR_OFF_HZ as f64).mix(kira::Mix::DRY));
            if reverb {
                builder = builder.with_send(engine.reverb_send.id(), Decibels::IDENTITY);
            }
            let track = engine.manager.add_spatial_sub_track(&engine.listener, mint_vec(position), builder);
            let Ok(mut track) = track else {
                warn!("too many sounds playing; dropped {:?}", player.0.path());
                end_playback(entity, settings.mode);
                continue;
            };
            if track.play(data).is_err() {
                end_playback(entity, settings.mode);
                continue;
            }
            entity.insert(SpatialAudioSink {
                playback,
                clip: key,
                profile: resolved.profile,
                looping,
                track,
                filter,
            });
        } else {
            let played = if reverb {
                engine.world.play(data).is_ok()
            } else {
                engine.manager.play(data).is_ok()
            };
            if !played {
                warn!("too many sounds playing; dropped {:?}", player.0.path());
                end_playback(entity, settings.mode);
                continue;
            }
            entity.insert(AudioSink { playback, clip: key });
        }
    }
}

/// Re-apply the mix (and master volume) to everything playing whenever it,
/// mute / solo or master volume changes — what makes the mixer's faders
/// live. Positional sounds pick up a changed profile assignment here too.
fn apply_mix(
    mix: Res<Mix>,
    monitor: Res<BusMonitor>,
    global: Res<GlobalVolume>,
    sinks: Query<&AudioSink>,
    mut spatial: Query<&mut SpatialAudioSink>,
) {
    if !mix.is_changed() && !monitor.is_changed() && !global.is_changed() {
        return;
    }
    let master = global.volume.to_linear();
    for sink in &sinks {
        sink.playback.set_mix(clip_gain(&mix, &monitor, &sink.clip, false).0 * master);
    }
    for mut sink in &mut spatial {
        let (gain, resolved) = clip_gain(&mix, &monitor, &sink.clip, true);
        sink.playback.set_mix(gain * master);
        if sink.profile != resolved.profile {
            sink.profile = resolved.profile;
        }
    }
}

/// Every frame, each positional sound's profile against its distance from
/// the listener: its fade, the highs it's lost to the air, how widely it
/// pans — and a looping one out of earshot held silent (not decoding).
fn apply_profiles(
    mix: Res<Mix>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut spatial: Query<(&mut SpatialAudioSink, Option<&GlobalTransform>)>,
) {
    let Ok(ears) = listener.single() else { return };
    let ears = ears.translation();
    for (mut sink, transform) in &mut spatial {
        let profile = mix.profile(&sink.profile);
        let distance = ears.distance(transform.map_or(Vec3::ZERO, GlobalTransform::translation));
        let gain = profile.gain(distance);
        sink.playback.set_distance(gain);
        sink.playback.set_culled(sink.looping && gain <= 0.0);
        let tween = Tween::default();
        match profile.air_cutoff(distance) {
            Some(hz) => {
                sink.filter.set_cutoff(hz as f64, tween);
                sink.filter.set_mix(kira::Mix::WET, tween);
            }
            None => sink.filter.set_mix(kira::Mix::DRY, tween),
        }
        sink.track.set_spatialization_strength(profile.width.clamp(0.0, 1.0), tween);
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
