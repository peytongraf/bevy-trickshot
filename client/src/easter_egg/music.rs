//! The easter egg's music (`audio/music/disastrous.mp3`): as the ritual
//! nears its end, the game's ambience fades out, and, just before the
//! Hierophant's "every drop given freely" from the sarcophagus
//! (`shared::easter_egg::RITUAL_VOICE_AT_SECS`), this fades in over the
//! silence — looping through it going wrong, him waking and the fight,
//! for as long as he's loose (`EggStep::is_unleashed`). Once he isn't (he's
//! dead, the egg's reset) or the game's over, it fades out and the ambience
//! comes back — after his death's played out (`EggStep::HisDeath`), if
//! that's why.
//!
//! The fight changes track with his phase ([`TRACKS`]): as he goes into
//! phase 2, what's playing fades out, then `hierophant_phase_2.mp3` starts
//! (fading in quickly); the same into phase 3 with
//! `hierophant_phase_3.mp3`. A debug heal back to an earlier phase
//! goes back to that phase's track the same way.
//!
//! A debug jump straight past the ritual brings it (and takes the ambience)
//! at once, faded in the same way. The music's `StateScoped(InGame)` and
//! its level starts at nothing each game; the ambience's level
//! (`zombies_audio::ZombiesAmbientLevel`) is back to full on entering one.
//! Tuned in the debug panel's "Easter egg (Der Tempel)" → "Music".

use bevy::prelude::*;
use bevy::time::Real;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::easter_egg::EggStep;
use shared::Lobby;

use crate::audio_engine::prelude::*;
use crate::match_end::MatchEndFreeze;
use crate::net::GameClient;
use crate::zombies_audio::ZombiesAmbientLevel;
use crate::AppState;

use super::{egg_game, EggRitual};

/// Its tracks, one for each phase of the fight (the first from the ritual
/// on), each looped.
const TRACKS: [&str; 3] = [
    "audio/music/disastrous.mp3",
    "audio/music/hierophant_phase_2.mp3",
    "audio/music/hierophant_phase_3.mp3",
];

pub(super) struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggMusicSettings>()
            .init_resource::<EggMusicLevels>()
            .add_systems(Startup, |asset_server: Res<AssetServer>, mut commands: Commands| {
                commands.insert_resource(EggMusicTracks(TRACKS.map(|path| asset_server.load(path))));
            })
            .add_systems(OnExit(AppState::InGame), |mut levels: ResMut<EggMusicLevels>| *levels = default())
            .add_systems(Update, run_music.run_if(in_state(AppState::InGame)));
    }
}

#[derive(Resource)]
struct EggMusicTracks([Handle<AudioSource>; 3]);

/// Panel-tunable music ("Music").
#[derive(Resource, Clone, Debug)]
pub(crate) struct EggMusicSettings {
    /// Its volume (linear, before the mix's music level).
    pub(crate) volume: f32,
    /// The ambience starts fading out this far (s) into the ritual, taking
    /// this long (s) to go...
    pub(crate) ambient_out_at: f32,
    pub(crate) ambient_out_secs: f32,
    /// ...and the music starts fading in this far (s) into the ritual (his
    /// line's at 13 s), taking this long (s) to reach full.
    pub(crate) music_in_at: f32,
    pub(crate) music_in_secs: f32,
    /// Once he's no longer loose: the music fades out over this long (s),
    /// and the ambience comes back in over this long (s).
    pub(crate) music_out_secs: f32,
    pub(crate) ambient_in_secs: f32,
    /// His phase changing: the track playing fades out over this long (s),
    /// then the next fades in over this long (s).
    pub(crate) phase_out_secs: f32,
    pub(crate) phase_in_secs: f32,
}

impl Default for EggMusicSettings {
    fn default() -> Self {
        Self {
            volume: 0.8,
            ambient_out_at: 9.5,
            ambient_out_secs: 2.5,
            music_in_at: 12.2,
            music_in_secs: 3.0,
            music_out_secs: 4.0,
            ambient_in_secs: 4.0,
            phase_out_secs: 2.0,
            phase_in_secs: 1.0,
        }
    }
}

/// How far in (0..=1) the music is, and which of [`TRACKS`] it is, this
/// game.
#[derive(Resource, Default)]
pub(crate) struct EggMusicLevels {
    music: f32,
    track: usize,
}

/// The music playing: which of [`TRACKS`].
#[derive(Component)]
struct EggMusic(usize);

/// The track wanted for `lobby`'s fight: his phase's (the first's until
/// he's in).
fn wanted_track(lobby: Option<&Lobby>) -> usize {
    lobby
        .filter(|l| l.easter_egg.step == EggStep::BossFight)
        .and_then(|l| l.easter_egg.hierophant)
        .map_or(0, |h| (h.phase.max(1) as usize - 1).min(TRACKS.len() - 1))
}

/// What's wanted now: (the ambience's level, the music on).
fn wanted(lobby: Option<&Lobby>, ritual: &EggRitual, s: &EggMusicSettings, running: bool) -> (f32, bool) {
    let Some(step) = lobby.map(|l| l.easter_egg.step).filter(|_| running) else {
        return (1.0, false);
    };
    match (step, ritual.t) {
        (EggStep::Ritual, Some(t)) => (if t >= s.ambient_out_at { 0.0 } else { 1.0 }, t >= s.music_in_at),
        (step, _) if step.is_unleashed() => (0.0, true),
        // He's fallen: the music dies with him, the ambience back once his
        // spirit's in the pyramid.
        // (And it stays quiet while the souls are up in the pyramid, with
        // its own hum — `ascension`.)
        (EggStep::HisDeath | EggStep::Ascension, _) => (0.0, false),
        _ => (1.0, false),
    }
}

/// Move `level` toward `target`, all the way over `secs`.
fn approach(level: &mut f32, target: f32, secs: f32, dt: f32) {
    let step = dt / secs.max(0.01);
    *level = if *level < target { (*level + step).min(target) } else { (*level - step).max(target) };
}

#[allow(clippy::too_many_arguments)]
fn run_music(
    time: Res<Time<Real>>,
    settings: Res<EggMusicSettings>,
    tracks: Res<EggMusicTracks>,
    ritual: Res<EggRitual>,
    freeze: Res<MatchEndFreeze>,
    mut levels: ResMut<EggMusicLevels>,
    mut ambient: ResMut<ZombiesAmbientLevel>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut music: Query<(Entity, &EggMusic, Option<&mut AudioSink>)>,
    mut commands: Commands,
) {
    let s = &*settings;
    let dt = time.delta_secs();
    let lobby = egg_game(&local, &lobbies);
    let running = lobby.is_some_and(|l| l.started) && !freeze.active;
    let (ambient_target, music_on) = wanted(lobby, &ritual, s, running);

    let ambient_secs = if ambient_target < ambient.0 { s.ambient_out_secs } else { s.ambient_in_secs };
    // (Not back in while the music's still fading out over it.)
    if ambient_target >= ambient.0 && levels.music > 0.0 && !music_on {
        // Wait for it.
    } else if ambient.0 != ambient_target {
        approach(&mut ambient.0, ambient_target, ambient_secs, dt);
    }

    // His phase changed: the one playing fades out first, then the next.
    let track = wanted_track(lobby);
    let switching = music_on && track != levels.track;
    let (target, secs) = if switching {
        (0.0, s.phase_out_secs)
    } else if music_on {
        (1.0, if levels.track == 0 { s.music_in_secs } else { s.phase_in_secs })
    } else {
        (0.0, s.music_out_secs)
    };
    approach(&mut levels.music, target, secs, dt);
    if levels.music <= 0.0 && switching {
        levels.track = track;
    }
    // (Anything but the current track is gone — and that too, once faded
    // right out with nothing wanted.)
    let silent = levels.music <= 0.0 && !music_on;
    for (e, m, _) in &music {
        if silent || m.0 != levels.track {
            commands.entity(e).despawn();
        }
    }
    if silent || (levels.music <= 0.0 && switching) {
        return;
    }
    if !music.iter().any(|(_, m, _)| m.0 == levels.track) {
        commands.spawn((
            EggMusic(levels.track),
            StateScoped(AppState::InGame),
            AudioPlayer::new(tracks.0[levels.track].clone()),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(s.volume * levels.music)),
        ));
        return;
    }
    for (_, m, sink) in &mut music {
        if let Some(mut sink) = sink.filter(|_| m.0 == levels.track) {
            sink.set_volume(Volume::Linear(s.volume * levels.music));
        }
    }
}

// --- the debug panel --------------------------------------------------------------------

/// The debug panel's controls for it.
pub(crate) fn music_section(ui: &mut egui::Ui, s: &mut EggMusicSettings, levels: &EggMusicLevels, ambient: f32) {
    ui.label(format!(
        "now: music {:.0}% (track {} of {}), ambience {:.0}%",
        levels.music * 100.0,
        levels.track + 1,
        TRACKS.len(),
        ambient * 100.0
    ));
    ui.label("(times are seconds into the 15 s ritual; his line from the sarcophagus is at 13 s)");
    ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=2.0).text("music volume"));
    ui.add(egui::Slider::new(&mut s.ambient_out_at, 0.0f32..=15.0).text("ambience fades out from (s)"));
    ui.add(egui::Slider::new(&mut s.ambient_out_secs, 0.1f32..=10.0).text("over (s)"));
    ui.add(egui::Slider::new(&mut s.music_in_at, 0.0f32..=15.0).text("music fades in from (s)"));
    ui.add(egui::Slider::new(&mut s.music_in_secs, 0.1f32..=10.0).text("over (s) "));
    ui.add(egui::Slider::new(&mut s.music_out_secs, 0.1f32..=15.0).text("after him: music fades out over (s)"));
    ui.add(egui::Slider::new(&mut s.ambient_in_secs, 0.1f32..=15.0).text("then the ambience back in over (s)"));
    ui.add(egui::Slider::new(&mut s.phase_out_secs, 0.1f32..=10.0).text("new phase: track fades out over (s)"));
    ui.add(egui::Slider::new(&mut s.phase_in_secs, 0.1f32..=10.0).text("then the next fades in over (s)"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = EggMusicSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
