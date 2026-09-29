//! `Zombies`' own music bed: an ambience loop ([`ZombiesAmbient`]) under the
//! whole game (on top of the map's usual ambience), and the game-over music
//! ([`ZombiesGameOverMusic`]) played once the moment the game ends — when the
//! server's `MatchEnding` starts the end-of-match freeze — which is also when
//! the ambience stops.
//!
//! Both are `StateScoped(InGame)`, so leaving the game cuts them off; neither
//! is positional. [`ZombiesGameOverPlayed`] is cleared whenever a game is
//! running (and on entering the game), so the next game plays it again.

use bevy::audio::Volume;
use bevy::prelude::*;
use lightyear::prelude::LocalId;
use shared::GameMode;

use crate::match_end::MatchEndFreeze;
use crate::net::GameClient;
use crate::settings::Settings;
use crate::zombies_hud::my_lobby;
use crate::{AppState, GameSounds, SoundVolumes};

/// Linear volume of the ambience loop (before its "Sound volumes" multiplier
/// and master volume).
const AMBIENT_VOLUME: f32 = 0.5;
/// Linear volume of the game-over music (likewise).
const GAME_OVER_VOLUME: f32 = 0.7;

#[derive(Component)]
struct ZombiesAmbient;

#[derive(Component)]
struct ZombiesGameOverMusic;

/// This game's game-over music has been started.
#[derive(Resource, Default)]
struct ZombiesGameOverPlayed(bool);

pub(crate) struct ZombiesAudioPlugin;

impl Plugin for ZombiesAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZombiesGameOverPlayed>()
            .add_systems(OnEnter(AppState::InGame), reset_game_over)
            .add_systems(
                Update,
                (sync_zombies_audio, apply_zombies_audio_volume)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_game_over(mut played: ResMut<ZombiesGameOverPlayed>) {
    played.0 = false;
}

/// Ambience while a `Zombies` game runs; at its end, the ambience out and the
/// game-over music in (once).
#[allow(clippy::too_many_arguments)]
fn sync_zombies_audio(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    freeze: Res<MatchEndFreeze>,
    sounds: Res<GameSounds>,
    vols: Res<SoundVolumes>,
    mut played: ResMut<ZombiesGameOverPlayed>,
    ambient: Query<Entity, With<ZombiesAmbient>>,
    mut commands: Commands,
) {
    let Some(lobby) = my_lobby(&local, &lobbies).filter(|l| l.mode == GameMode::Zombies) else {
        for e in &ambient {
            commands.entity(e).despawn();
        }
        return;
    };
    let running = lobby.started && !freeze.active;
    if running {
        played.0 = false;
        if ambient.is_empty() {
            commands.spawn((
                ZombiesAmbient,
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.zombies_ambient.clone()),
                // (`GlobalVolume` — master volume — is multiplied in at spawn.)
                PlaybackSettings::LOOP
                    .with_volume(Volume::Linear(AMBIENT_VOLUME * vols.zombies_ambient)),
            ));
        }
        return;
    }
    for e in &ambient {
        commands.entity(e).despawn();
    }
    if !played.0 {
        played.0 = true;
        commands.spawn((
            ZombiesGameOverMusic,
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.zombies_game_over.clone()),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(GAME_OVER_VOLUME * vols.zombies_game_over)),
        ));
    }
}

/// Follow master volume and the "Sound volumes" sliders while they play
/// (`GlobalVolume` only applies when a sound starts).
fn apply_zombies_audio_volume(
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    mut sinks: Query<
        (&mut AudioSink, Has<ZombiesAmbient>),
        Or<(With<ZombiesAmbient>, With<ZombiesGameOverMusic>)>,
    >,
) {
    if !settings.is_changed() && !vols.is_changed() {
        return;
    }
    for (mut sink, ambient) in &mut sinks {
        let base = if ambient {
            AMBIENT_VOLUME * vols.zombies_ambient
        } else {
            GAME_OVER_VOLUME * vols.zombies_game_over
        };
        sink.set_volume(Volume::Linear(base * settings.master_volume));
    }
}
