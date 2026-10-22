//! `Zombies`' own music bed: an ambience loop ([`ZombiesAmbient`]) under the
//! whole game (in place of the map's usual ambience — `audio::start_ambient`
//! skips it in `Zombies`), and the game-over music
//! ([`ZombiesGameOverMusic`]) played once the moment the game ends — when the
//! server's `MatchEnding` starts the end-of-match freeze — which is also when
//! the ambience stops.
//!
//! The exfil music ([`ZombiesExfilMusic`]) loops from the moment the exfil's
//! called to the game's end (the exfil escaped or failed, or the game over
//! some other way), then fades out over [`EXFIL_FADE_SECS`] — and only once
//! it's gone does the game-over music start.
//!
//! All three are `StateScoped(InGame)`, so leaving the game cuts them off;
//! none is positional. [`ZombiesGameOverPlayed`] is cleared whenever a game is
//! running (and on entering the game), so the next game plays it again.

use bevy::prelude::*;
use bevy::time::Real;

use crate::audio_engine::prelude::*;
use lightyear::prelude::LocalId;
use shared::GameMode;

use crate::match_end::MatchEndFreeze;
use crate::net::GameClient;
use crate::zombies_hud::my_lobby;
use crate::{AppState, GameSounds};

/// Linear volume of the ambience loop (before its level in the mix
/// and master volume).
const AMBIENT_VOLUME: f32 = 0.5;
/// Linear volume of the game-over music (likewise).
const GAME_OVER_VOLUME: f32 = 0.7;
/// Linear volume of the exfil music (likewise)...
const EXFIL_VOLUME: f32 = 0.7;
/// ...and how long (s) it takes to fade out once the game's over.
const EXFIL_FADE_SECS: f32 = 2.0;

#[derive(Component)]
struct ZombiesAmbient;

#[derive(Component)]
struct ZombiesGameOverMusic;

/// The exfil music; `fade` is how far it's faded out (0 → 1, gone at 1).
#[derive(Component, Default)]
struct ZombiesExfilMusic {
    fade: f32,
}

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
                sync_zombies_audio.run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_game_over(mut played: ResMut<ZombiesGameOverPlayed>) {
    played.0 = false;
}

/// Ambience while a `Zombies` game runs, and the exfil music once it's
/// called; at its end, the ambience out, the exfil music faded out, and then
/// the game-over music in (once).
#[allow(clippy::too_many_arguments)]
fn sync_zombies_audio(
    time: Res<Time<Real>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    freeze: Res<MatchEndFreeze>,
    sounds: Res<GameSounds>,
    mut played: ResMut<ZombiesGameOverPlayed>,
    ambient: Query<Entity, With<ZombiesAmbient>>,
    mut exfil: Query<(Entity, &mut ZombiesExfilMusic, Option<&mut AudioSink>)>,
    mut commands: Commands,
) {
    let Some(lobby) = my_lobby(&local, &lobbies).filter(|l| l.mode == GameMode::Zombies) else {
        for e in ambient.iter().chain(exfil.iter().map(|(e, ..)| e)) {
            commands.entity(e).despawn();
        }
        return;
    };
    let running = lobby.started && !freeze.active;

    // The exfil music: in as it's called, faded out as the game ends.
    let exfil_on = running && lobby.exfil.running();
    if exfil_on && exfil.is_empty() {
        commands.spawn((
            ZombiesExfilMusic::default(),
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.zombies_exfil.clone()),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(EXFIL_VOLUME)),
        ));
    }
    for (e, mut music, sink) in &mut exfil {
        if exfil_on {
            continue;
        }
        music.fade = (music.fade + time.delta_secs() / EXFIL_FADE_SECS).min(1.0);
        if music.fade >= 1.0 {
            commands.entity(e).despawn();
        } else if let Some(mut sink) = sink {
            sink.set_volume(Volume::Linear(EXFIL_VOLUME * (1.0 - music.fade)));
        }
    }

    if running {
        played.0 = false;
        if ambient.is_empty() {
            commands.spawn((
                ZombiesAmbient,
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.zombies_ambient.clone()),
                PlaybackSettings::LOOP
                    .with_volume(Volume::Linear(AMBIENT_VOLUME)),
            ));
        }
        return;
    }
    for e in &ambient {
        commands.entity(e).despawn();
    }
    // (Not until the exfil music's faded out.)
    if !played.0 && exfil.is_empty() {
        played.0 = true;
        commands.spawn((
            ZombiesGameOverMusic,
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.zombies_game_over.clone()),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(GAME_OVER_VOLUME)),
        ));
    }
}

