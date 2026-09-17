//! Preloaded sound handles, per-category volume multipliers, and the
//! looping ambience bed — plus pushing `Settings::master_volume` and the
//! "Sound volumes" panel onto whatever's actually playing.

use bevy::audio::Volume;
use bevy::prelude::*;

use crate::environment::CurrentMap;
use crate::player::FOOTSTEP_CLIPS;
use crate::settings::Settings;
use crate::AppState;

/// Preloaded sounds. Loaded once at startup so playback has no first-use hitch.
#[derive(Resource)]
pub(crate) struct GameSounds {
    pub(crate) shot: Handle<AudioSource>,
    pub(crate) rechamber: Handle<AudioSource>,
    pub(crate) reload: Handle<AudioSource>,
    ambient: Handle<AudioSource>,
    /// `Shipment`'s own ambience bed — a cargo ship out on open water, so
    /// nothing like `ambient`'s outdoor-nature loop. `start_ambient` picks
    /// between the two by [`CurrentMap`]; `basic_map.glb` keeps `ambient`.
    shipment_ambient: Handle<AudioSource>,
    pub(crate) aim_in: Handle<AudioSource>,
    pub(crate) aim_out: Handle<AudioSource>,
    pub(crate) out_of_ammo: Handle<AudioSource>,
    pub(crate) slide: Handle<AudioSource>,
    pub(crate) dive: Handle<AudioSource>,
    pub(crate) kill_enemy: Handle<AudioSource>,
    pub(crate) jump_land: Handle<AudioSource>,
    pub(crate) teleport: Handle<AudioSource>,
    /// `audio/footsteps/footstep_1..N.wav` — `footsteps` picks one at random
    /// per step.
    pub(crate) footsteps: Vec<Handle<AudioSource>>,
}

/// Linear volume shared by both ambience loops (`ambient` and
/// `shipment_ambient`) before their own "Sound volumes" multiplier.
pub(crate) const AMBIENT_VOLUME: f32 = 0.5;

/// Panel-adjustable per-sound volume multipliers ("Sound volumes" panel
/// section). `1.0` leaves a sound at its built-in level; every one-shot is
/// scaled by its entry when it spawns (`apply_sound_volumes`), and whichever
/// ambience loop is currently playing by `ambient` / `shipment_ambient`
/// (folded into `apply_master_volume`). Footsteps have their own controls in
/// the "Footsteps" section and aren't here.
#[derive(Resource)]
pub(crate) struct SoundVolumes {
    pub(crate) shot: f32,
    pub(crate) rechamber: f32,
    pub(crate) reload: f32,
    pub(crate) ambient: f32,
    pub(crate) shipment_ambient: f32,
    pub(crate) aim_in: f32,
    pub(crate) aim_out: f32,
    pub(crate) out_of_ammo: f32,
    pub(crate) slide: f32,
    pub(crate) dive: f32,
    pub(crate) kill_enemy: f32,
    pub(crate) jump_land: f32,
    pub(crate) teleport: f32,
}

impl Default for SoundVolumes {
    fn default() -> Self {
        Self {
            shot: 1.0,
            rechamber: 1.0,
            reload: 1.5,
            ambient: 2.5,
            shipment_ambient: 2.5,
            aim_in: 1.0,
            aim_out: 1.0,
            out_of_ammo: 1.0,
            slide: 1.0,
            dive: 1.0,
            kill_enemy: 5.5,
            jump_land: 0.5,
            teleport: 1.0,
        }
    }
}

impl SoundVolumes {
    /// The multiplier for `handle`, or `None` if it isn't a one-shot this
    /// resource covers (footstep clips, the ambient loop).
    pub(crate) fn oneshot_for(
        &self,
        handle: &Handle<AudioSource>,
        sounds: &GameSounds,
    ) -> Option<f32> {
        let id = handle.id();
        [
            (sounds.shot.id(), self.shot),
            (sounds.rechamber.id(), self.rechamber),
            (sounds.reload.id(), self.reload),
            (sounds.aim_in.id(), self.aim_in),
            (sounds.aim_out.id(), self.aim_out),
            (sounds.out_of_ammo.id(), self.out_of_ammo),
            (sounds.slide.id(), self.slide),
            (sounds.dive.id(), self.dive),
            (sounds.kill_enemy.id(), self.kill_enemy),
            (sounds.jump_land.id(), self.jump_land),
            (sounds.teleport.id(), self.teleport),
        ]
        .into_iter()
        .find_map(|(hid, vol)| (hid == id).then_some(vol))
    }
}

/// The looping ambient-nature bed. `StateScoped(InGame)`, so it starts when the
/// player enters the world (Practice or a game) and stops on the way out.
#[derive(Component)]
pub(crate) struct AmbientAudio;

/// Tags a sound spawned by `net::receive_remote_sounds` for another player's
/// action (reload, footstep, shot, ...), positioned at their world location.
/// `apply_sound_volumes` skips these — their volume is already fully baked in
/// at spawn time (category volume × distance falloff × the "Remote sounds"
/// panel's master volume × global volume), since re-deriving it post-spawn
/// would need the same distance calculation all over again for no benefit.
#[derive(Component)]
pub(crate) struct RemoteSoundEmitter;

/// Distance falloff for other players' positional sounds ("Remote sounds"
/// debug-panel section). Applied by `net::receive_remote_sounds` on top of
/// this same clip's normal `SoundVolumes` category multiplier.
#[derive(Resource, Clone, Copy)]
pub(crate) struct RemoteSoundSettings {
    /// Overall gain on every remote-player sound, on top of its usual
    /// per-category volume.
    pub(crate) volume: f32,
    /// Distance (m) at which a remote sound has faded to silence.
    pub(crate) max_distance: f32,
}

impl Default for RemoteSoundSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            max_distance: 60.0,
        }
    }
}

pub(crate) fn setup_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(GameSounds {
        shot: asset_server.load("audio/sniper_shot.wav"),
        rechamber: asset_server.load("audio/rechamber.wav"),
        reload: asset_server.load("audio/reload.wav"),
        ambient: asset_server.load("audio/ambient_nature.ogg"),
        shipment_ambient: asset_server.load("audio/shipment_ambient.ogg"),
        aim_in: asset_server.load("audio/aim-in-sound.mp3"),
        aim_out: asset_server.load("audio/aim-out-sound.mp3"),
        out_of_ammo: asset_server.load("audio/out-of-ammo-sound.mp3"),
        slide: asset_server.load("audio/slide-sound.mp3"),
        dive: asset_server.load("audio/dive-sound.mp3"),
        kill_enemy: asset_server.load("audio/kill-enemy-sound.mp3"),
        jump_land: asset_server.load("audio/jump-landing-sound.mp3"),
        teleport: asset_server.load("audio/teleport.wav"),
        footsteps: (1..=FOOTSTEP_CLIPS)
            .map(|i| asset_server.load(format!("audio/footsteps/footstep_{i}.wav")))
            .collect(),
    });
}

/// Start the map's looping ambience bed when the player enters the world —
/// `basic_map.glb`'s outdoor-nature loop, or `shipment.glb`'s own cargo-ship
/// one, by [`CurrentMap`]. Ordered `.after(lobby_ui::sync_current_map)`
/// (both run on `OnEnter(AppState::InGame)`) so this always sees the map
/// just selected, not whatever `CurrentMap` was left at after the previous
/// match.
pub(crate) fn start_ambient(
    mut commands: Commands,
    sounds: Res<GameSounds>,
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    current: Res<CurrentMap>,
) {
    let (clip, volume_mult) = match current.0 {
        shared::MapId::BasicMap => (sounds.ambient.clone(), vols.ambient),
        shared::MapId::Shipment => (sounds.shipment_ambient.clone(), vols.shipment_ambient),
    };
    commands.spawn((
        AmbientAudio,
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(
            AMBIENT_VOLUME * volume_mult * settings.master_volume,
        )),
    ));
}

/// Push `Settings::master_volume` onto Bevy's `GlobalVolume`, which scales
/// every one-shot sound spawned from here on (shots, footsteps, UI, ...) with
/// no per-call-site changes needed. `GlobalVolume` doesn't retroactively touch
/// audio that's already playing, though, so the looping ambience needs its own
/// direct nudge here too — also picking up the "Sound volumes" panel's
/// multiplier for whichever ambience loop `start_ambient` actually started.
pub(crate) fn apply_master_volume(
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    current: Res<CurrentMap>,
    mut global_volume: ResMut<GlobalVolume>,
    mut ambient: Query<&mut AudioSink, With<AmbientAudio>>,
) {
    if !settings.is_changed() && !vols.is_changed() {
        return;
    }
    global_volume.volume = Volume::Linear(settings.master_volume);
    let ambient_mult = match current.0 {
        shared::MapId::BasicMap => vols.ambient,
        shared::MapId::Shipment => vols.shipment_ambient,
    };
    for mut sink in &mut ambient {
        sink.set_volume(Volume::Linear(
            AMBIENT_VOLUME * ambient_mult * settings.master_volume,
        ));
    }
}

/// Scale each freshly-started one-shot to its "Sound volumes" multiplier.
/// bevy_audio bakes `PlaybackSettings::volume * GlobalVolume` into the sink when
/// it starts it, so we re-derive the same product with the per-sound factor
/// mixed in. Sounds not covered here (footsteps, the ambient loop) are left be.
pub(crate) fn apply_sound_volumes(
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    global_volume: Res<GlobalVolume>,
    mut fresh: Query<
        (&AudioPlayer, &mut AudioSink),
        (Added<AudioSink>, Without<RemoteSoundEmitter>),
    >,
) {
    let Some(sounds) = sounds else { return };
    for (player, mut sink) in &mut fresh {
        if let Some(mult) = vols.oneshot_for(&player.0, &sounds) {
            sink.set_volume(Volume::Linear(mult.max(0.0)) * global_volume.volume);
        }
    }
}
