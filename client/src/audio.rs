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
    /// `audio/throwing_knife/throw.mp3` — the knife leaving the hand.
    pub(crate) knife_throw: Handle<AudioSource>,
    /// `throwing_knife_hit_enemy.mp3` — a thrown knife killing a bot / player.
    pub(crate) knife_hit: Handle<AudioSource>,
    /// `throwing_knife_in_air.mp3` — the whoosh that follows a thrown knife.
    pub(crate) knife_in_air: Handle<AudioSource>,
    /// `audio/knife/equip-knife-sound.mp3` — switching to the regular knife.
    pub(crate) knife_equip: Handle<AudioSource>,
    /// `audio/equip_sniper.mp3` — switching to the sniper.
    pub(crate) sniper_equip: Handle<AudioSource>,
    /// `audio/not-used-yet/heartbeat-sound.mp3` — looped while the player is
    /// hurt (`health::update_heartbeat`).
    pub(crate) heartbeat: Handle<AudioSource>,
    /// `audio/footsteps/footstep_1..N.wav` — `footsteps` picks one at random
    /// per step.
    pub(crate) footsteps: Vec<Handle<AudioSource>>,
}

/// One clip from a [`SoundSet`], with its own volume multiplier.
pub(crate) struct SoundClip {
    /// File name without the extension — the label in the debug panel.
    pub(crate) name: String,
    pub(crate) handle: Handle<AudioSource>,
    /// Multiplier on the clip's built-in level ("Sound volumes" panel).
    pub(crate) volume: f32,
}

/// Every audio file (`.mp3` / `.wav` / `.ogg` / `.flac`) found in one folder
/// under the assets dir when the game started, sorted by name so the order is
/// the same on every client. The server picks which clip plays for an event
/// (`variant % clips.len()`), so the whole lobby hears the same one. Found by
/// listing the folder rather than hard-coding names, so dropping clips in / out
/// of it needs no code change; each clip gets its own slider.
#[derive(Default)]
pub(crate) struct SoundSet {
    pub(crate) clips: Vec<SoundClip>,
}

impl SoundSet {
    /// Load the folder `dir` (relative to the assets dir), every clip starting
    /// at `volume`.
    fn load(asset_server: &AssetServer, dir: &str, volume: f32) -> Self {
        let path = bevy::asset::io::file::FileAssetReader::get_base_path()
            .join("assets")
            .join(dir);
        let mut names: Vec<String> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let ext = p.extension()?.to_str()?.to_ascii_lowercase();
                matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "flac")
                    .then(|| p.file_name()?.to_str().map(str::to_owned))
                    .flatten()
            })
            .collect();
        names.sort();
        if names.is_empty() {
            warn!("no sounds found in {}", path.display());
        }
        Self {
            clips: names
                .into_iter()
                .map(|file| SoundClip {
                    name: file
                        .rsplit_once('.')
                        .map_or(file.clone(), |(stem, _)| stem.to_owned()),
                    handle: asset_server.load(format!("{dir}/{file}")),
                    volume,
                })
                .collect(),
        }
    }

    /// The clip the server's random `variant` picks, if the set isn't empty.
    pub(crate) fn pick(&self, variant: u8) -> Option<&SoundClip> {
        (!self.clips.is_empty()).then(|| &self.clips[variant as usize % self.clips.len()])
    }
}

/// The knife sounds that live in folders: the throwing knife's surface
/// impacts, and the regular knife's stabs and swings.
#[derive(Resource)]
pub(crate) struct KnifeSounds {
    /// `audio/throwing_knife/impact/` — a thrown knife striking a surface.
    pub(crate) impact: SoundSet,
    /// `audio/knife/stab/` — a stab landing on a bot / player.
    pub(crate) stab: SoundSet,
    /// `audio/knife/swing/` — a stab that hits nothing.
    pub(crate) swing: SoundSet,
}

impl KnifeSounds {
    fn load(asset_server: &AssetServer) -> Self {
        Self {
            impact: SoundSet::load(asset_server, "audio/throwing_knife/impact", 0.3),
            stab: SoundSet::load(asset_server, "audio/knife/stab", 1.0),
            swing: SoundSet::load(asset_server, "audio/knife/swing", 1.0),
        }
    }
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
    pub(crate) knife_throw: f32,
    pub(crate) knife_hit: f32,
    pub(crate) knife_in_air: f32,
    pub(crate) knife_equip: f32,
    pub(crate) sniper_equip: f32,
    /// Loudness of the heartbeat at zero health (it fades toward silence as
    /// health recovers) — see `health::update_heartbeat`.
    pub(crate) heartbeat: f32,
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
            knife_throw: 1.0,
            knife_hit: 1.5,
            knife_in_air: 1.0,
            knife_equip: 1.0,
            sniper_equip: 1.0,
            heartbeat: 1.0,
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
            (sounds.knife_throw.id(), self.knife_throw),
            (sounds.knife_hit.id(), self.knife_hit),
            (sounds.knife_in_air.id(), self.knife_in_air),
            (sounds.knife_equip.id(), self.knife_equip),
            (sounds.sniper_equip.id(), self.sniper_equip),
        ]
        .into_iter()
        .find_map(|(hid, vol)| (hid == id).then_some(vol))
    }
}

/// The looping ambient-nature bed. `StateScoped(InGame)`, so it starts when the
/// player enters the world and stops on the way out.
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

/// Playback settings for a sound emitted from a point in the world: spatial
/// (panned by where it is relative to the listener) with rodio's own
/// `1 / distance²` curve shrunk to near-flat, so the loudness-by-distance is
/// whatever `volume` already bakes in — see `net::receive_remote_sounds`, which
/// explains why.
pub(crate) fn positional_playback(volume: Volume) -> PlaybackSettings {
    PlaybackSettings::DESPAWN
        .with_spatial(true)
        .with_spatial_scale(bevy::audio::SpatialScale::new(0.01))
        .with_volume(volume)
}

/// The `[0, 1]` loudness factor for a remote sound `distance` metres from the
/// listener: a squared linear fade to silence at
/// [`RemoteSoundSettings::max_distance`].
pub(crate) fn distance_falloff(distance: f32, remote: &RemoteSoundSettings) -> f32 {
    if distance >= remote.max_distance {
        0.0
    } else {
        (1.0 - distance / remote.max_distance).powi(2)
    }
}

pub(crate) fn setup_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(KnifeSounds::load(&asset_server));
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
        knife_throw: asset_server.load("audio/throwing_knife/throw.mp3"),
        knife_hit: asset_server.load("audio/throwing_knife/throwing_knife_hit_enemy.mp3"),
        knife_in_air: asset_server.load("audio/throwing_knife/throwing_knife_in_air.mp3"),
        knife_equip: asset_server.load("audio/knife/equip-knife-sound.mp3"),
        sniper_equip: asset_server.load("audio/equip_sniper.mp3"),
        heartbeat: asset_server.load("audio/not-used-yet/heartbeat-sound.mp3"),
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
        shared::MapId::Shipment | shared::MapId::ShipmentDay => {
            (sounds.shipment_ambient.clone(), vols.shipment_ambient)
        }
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
        shared::MapId::Shipment | shared::MapId::ShipmentDay => vols.shipment_ambient,
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
