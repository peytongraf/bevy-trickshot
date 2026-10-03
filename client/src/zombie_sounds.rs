//! `Zombies` zombie voices, all positional (`audio/zombies/enemy/`):
//!
//! * a spawn groan as each one climbs out of the ground,
//! * moans now and then while it's up — low / medium / high intensity by how
//!   fast it's been moving, on a random min..max timer each, never too many
//!   at once or too close together,
//! * a death cry where it drops — cutting off whatever it was still saying
//!   (groan, moan, final call), so the cry is all you hear,
//! * the swipe landing on someone (the server's [`shared::ZombieSwipeLanded`]
//!   — only it knows a swing connected), heard from where that player is,
//! * and once a round is down to its last zombie, its "final zombie" call on
//!   a loose timer, in place of its moans.
//!
//! Everything but the swipe is worked out by each client from what it sees
//! (Call of Duty's zombie vocals aren't synced either): every client sees the
//! same zombie rise or fall at the same moment, and the moans are ambience.
//! Moans and the final call ride along with their zombie; every clip's
//! loudness follows the listener's distance each frame.

use bevy::audio::{SpatialAudioSink, Volume};
use bevy::prelude::*;
use lightyear::prelude::*;
use shared::{Lobby, PlayerPose};

use crate::net::{GameClient, RemoteAvatar};
use crate::util::rand01;
use crate::{AppState, WorldModelCamera, ZombieMotion, ZombieVisual};

/// Where on a zombie (m above its feet) its voice comes from.
const MOUTH_HEIGHT: f32 = 1.6;

/// Panel-tunable zombie voice timing ("Zombie sounds" debug-panel section).
#[derive(Resource, Clone, Copy)]
pub(crate) struct ZombieSoundSettings {
    /// Distance (m) at which a zombie sound has faded to silence.
    pub(crate) max_distance: f32,
    /// Each zombie moans every `moan_min_secs..moan_max_secs` (random)...
    pub(crate) moan_min_secs: f32,
    pub(crate) moan_max_secs: f32,
    /// ...but never more than this many moans at once...
    pub(crate) max_moans_at_once: u32,
    /// ...nor two starting within this long (s) of each other.
    pub(crate) moan_min_gap_secs: f32,
    /// Moving slower than this (m/s): low-intensity moans; faster than
    /// `high_moan_above`: high; in between: medium.
    pub(crate) low_moan_below: f32,
    pub(crate) high_moan_above: f32,
    /// The last zombie of a round calls every `final_min_secs..final_max_secs`.
    pub(crate) final_min_secs: f32,
    pub(crate) final_max_secs: f32,
}

impl Default for ZombieSoundSettings {
    fn default() -> Self {
        Self {
            max_distance: 50.0,
            moan_min_secs: 4.0,
            moan_max_secs: 11.0,
            max_moans_at_once: 3,
            moan_min_gap_secs: 0.8,
            low_moan_below: 2.0,
            high_moan_above: 4.0,
            final_min_secs: 5.0,
            final_max_secs: 9.0,
        }
    }
}

/// The clips, loaded once.
#[derive(Resource)]
struct ZombieSounds {
    spawns: Vec<Handle<AudioSource>>,
    deaths: Vec<Handle<AudioSource>>,
    /// Low, medium, high intensity.
    moans: [Vec<Handle<AudioSource>>; 3],
    attack: Handle<AudioSource>,
    final_zombie: Handle<AudioSource>,
}

/// On a zombie avatar: its voice's state.
#[derive(Component)]
struct ZombieVoice {
    next_moan_at: f32,
    /// How fast it's been going lately (m/s) — its speed, easing down slowly,
    /// so a pause to swipe doesn't drop it to low moans.
    pace: f32,
    was_alive: bool,
}

/// A playing zombie sound: its loudness before the distance fade.
#[derive(Component)]
struct ZombieSound {
    loudness: f32,
}

/// A playing sound in a zombie's own voice (its spawn groan, a moan, the
/// final-zombie call): cut off the moment that zombie dies.
#[derive(Component)]
struct VoiceOf(Entity);

/// Marks a playing moan (to cap how many at once).
#[derive(Component)]
struct ZombieMoan;

/// The round's last zombie's next call (`None` while there's more than one).
#[derive(Resource, Default)]
struct FinalZombie {
    next_at: Option<f32>,
    /// When the last moan anywhere started.
    last_moan_at: f32,
}

pub(crate) struct ZombieSoundsPlugin;

impl Plugin for ZombieSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZombieSoundSettings>()
            .init_resource::<FinalZombie>()
            .add_systems(Startup, load_zombie_sounds)
            .add_systems(
                Update,
                (
                    zombie_rise_and_fall,
                    zombie_moans,
                    receive_zombie_swipes,
                    fade_zombie_sounds,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn load_zombie_sounds(mut commands: Commands, asset_server: Res<AssetServer>) {
    let set = |dir: &str, name: &str, n: usize| -> Vec<Handle<AudioSource>> {
        (1..=n)
            .map(|i| asset_server.load(format!("audio/zombies/{dir}/{name}_{i}.mp3")))
            .collect()
    };
    commands.insert_resource(ZombieSounds {
        spawns: set("enemy/spawns", "zombie_spawn", 5),
        deaths: set("enemy/deaths", "zombie_death", 7),
        moans: [
            set("enemy/moans/low", "moan", 6),
            set("enemy/moans/medium", "moan", 24),
            set("enemy/moans/high", "moan", 10),
        ],
        attack: asset_server.load("audio/zombies/enemy/attack.mp3"),
        final_zombie: asset_server.load("audio/zombies/rounds/final_zombie.mp3"),
    });
}

fn pick(clips: &[Handle<AudioSource>], roll: f32) -> Option<Handle<AudioSource>> {
    (!clips.is_empty()).then(|| clips[(roll * clips.len() as f32) as usize % clips.len()].clone())
}

/// A zombie sound at `at` (or riding along on `parent`, offset by `at`),
/// silent until [`fade_zombie_sounds`] sets its real volume next frame.
fn spawn_sound(commands: &mut Commands, clip: Handle<AudioSource>, loudness: f32, at: Vec3) -> Entity {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            ZombieSound { loudness },
            // Volume is ours (distance fade), not the one-shot pass's.
            crate::RemoteSoundEmitter,
            AudioPlayer::new(clip),
            Transform::from_translation(at),
            crate::positional_playback(Volume::Linear(0.0)),
        ))
        .id()
}

/// A random time in `min..max` from `now`.
fn after(now: f32, min: f32, max: f32, roll: f32) -> f32 {
    now + min.max(0.0) + (max - min).max(0.0) * roll
}

/// A new zombie avatar: its spawn groan (it's climbing out of the ground),
/// and a voice. A zombie that just dropped: its death cry.
#[allow(clippy::type_complexity)]
fn zombie_rise_and_fall(
    time: Res<Time>,
    sounds: Option<Res<ZombieSounds>>,
    settings: Res<ZombieSoundSettings>,
    vols: Res<crate::SoundVolumes>,
    poses: Query<&PlayerPose>,
    new: Query<(Entity, &RemoteAvatar), (With<ZombieVisual>, Without<ZombieVoice>)>,
    mut voices: Query<(Entity, &RemoteAvatar, &mut ZombieVoice)>,
    speaking: Query<(Entity, &VoiceOf)>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let Some(sounds) = sounds else { return };
    let now = time.elapsed_secs();
    let mut roll = || {
        *seq = seq.wrapping_add(1);
        rand01(seq.wrapping_mul(2_654_435_761) ^ now.to_bits())
    };
    for (entity, avatar) in &new {
        let Ok(pose) = poses.get(avatar.src) else { continue };
        let feet = pose.translation - Vec3::Y * crate::EYE_HEIGHT;
        if pose.alive {
            if let Some(clip) = pick(&sounds.spawns, roll()) {
                let id = spawn_sound(&mut commands, clip, vols.zombie_spawn, feet + Vec3::Y * 0.5);
                commands.entity(id).insert(VoiceOf(entity));
            }
        }
        commands.entity(entity).insert(ZombieVoice {
            // Not straight away — let the spawn groan have the stage.
            next_moan_at: after(now + 1.5, settings.moan_min_secs * 0.5, settings.moan_max_secs, roll()),
            pace: 0.0,
            was_alive: pose.alive,
        });
    }
    for (zombie, avatar, mut voice) in &mut voices {
        let Ok(pose) = poses.get(avatar.src) else { continue };
        if voice.was_alive && !pose.alive {
            for (sound, owner) in &speaking {
                if owner.0 == zombie {
                    commands.entity(sound).try_despawn();
                }
            }
            let feet = pose.translation - Vec3::Y * crate::EYE_HEIGHT;
            if let Some(clip) = pick(&sounds.deaths, roll()) {
                spawn_sound(&mut commands, clip, vols.zombie_death, feet + Vec3::Y * MOUTH_HEIGHT);
            }
        }
        voice.was_alive = pose.alive;
    }
}

/// Each living zombie moans on its own random timer, by how fast it's been
/// going — but only a few at a time, spaced out — and a round's last zombie
/// makes its final-zombie call instead.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn zombie_moans(
    time: Res<Time>,
    sounds: Option<Res<ZombieSounds>>,
    settings: Res<ZombieSoundSettings>,
    vols: Res<crate::SoundVolumes>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    poses: Query<&PlayerPose>,
    mut voices: Query<(Entity, &RemoteAvatar, &ZombieMotion, &mut ZombieVoice)>,
    moans: Query<(), With<ZombieMoan>>,
    mut last: ResMut<FinalZombie>,
    avatar_settings: Res<crate::ZombieAvatarSettings>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let Some(sounds) = sounds else { return };
    // Its voice's offset in the avatar's own (scaled) space.
    let mouth = Vec3::Y * MOUTH_HEIGHT / avatar_settings.scale.max(0.01);
    let Some(lobby) = crate::zombies_hud::zombies_game(&local, &lobbies) else {
        *last = FinalZombie::default();
        return;
    };
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let mut roll = || {
        *seq = seq.wrapping_add(1);
        rand01(seq.wrapping_mul(40_503) ^ now.to_bits())
    };

    let living: Vec<Entity> = voices
        .iter()
        .filter(|(_, a, ..)| poses.get(a.src).is_ok_and(|p| p.alive))
        .map(|(e, ..)| e)
        .collect();
    // The last one standing (nothing left to spawn this round either).
    let final_one = (lobby.enemies_left == 1 && living.len() == 1).then(|| living[0]);
    if final_one.is_none() {
        last.next_at = None;
    }

    let mut playing = moans.iter().count() as u32;
    for (entity, avatar, motion, mut voice) in &mut voices {
        voice.pace = motion.speed().max(voice.pace - 0.6 * dt);
        if lobby.paused || !poses.get(avatar.src).is_ok_and(|p| p.alive) {
            continue;
        }
        if Some(entity) == final_one {
            let due = *last.next_at.get_or_insert_with(|| after(now, 1.0, 2.5, roll()));
            if now >= due {
                let id = spawn_sound(&mut commands, sounds.final_zombie.clone(), vols.final_zombie, mouth);
                commands.entity(id).insert((VoiceOf(entity), ChildOf(entity)));
                last.next_at = Some(after(now, settings.final_min_secs, settings.final_max_secs, roll()));
            }
            continue;
        }
        if now < voice.next_moan_at {
            continue;
        }
        // Too many going, or one only just started: try again shortly.
        if playing >= settings.max_moans_at_once || now - last.last_moan_at < settings.moan_min_gap_secs {
            voice.next_moan_at = now + 0.4 + roll() * 1.2;
            continue;
        }
        let tier = if voice.pace < settings.low_moan_below {
            0
        } else if voice.pace <= settings.high_moan_above {
            1
        } else {
            2
        };
        if let Some(clip) = pick(&sounds.moans[tier], roll()) {
            let id = spawn_sound(&mut commands, clip, vols.zombie_moan, mouth);
            commands.entity(id).insert((ZombieMoan, VoiceOf(entity), ChildOf(entity)));
            playing += 1;
            last.last_moan_at = now;
        }
        voice.next_moan_at = after(now, settings.moan_min_secs, settings.moan_max_secs, roll());
    }
}

/// Server → us: a zombie's swipe landed on someone — the hit, from them.
fn receive_zombie_swipes(
    sounds: Option<Res<ZombieSounds>>,
    vols: Res<crate::SoundVolumes>,
    mut receivers: Query<&mut MessageReceiver<shared::ZombieSwipeLanded>>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(sounds) = sounds.as_ref() else { continue };
            let at = Vec3::from_array(msg.at) + Vec3::Y * 1.2;
            spawn_sound(&mut commands, sounds.attack.clone(), vols.zombie_attack, at);
        }
    }
}

/// Keep every zombie sound's loudness matched to how far the listener is
/// from it (they can move with their zombie) — a squared fade to silence at
/// `max_distance`.
fn fade_zombie_sounds(
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    settings: Res<ZombieSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut playing: Query<(&GlobalTransform, &ZombieSound, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else { return };
    let ear = ear.translation();
    let max = settings.max_distance.max(1.0);
    for (gt, sound, mut sink) in &mut playing {
        let fade = (1.0 - ear.distance(gt.translation()) / max).clamp(0.0, 1.0).powi(2);
        sink.set_volume(Volume::Linear((sound.loudness * fade).max(0.0)) * global_volume.volume);
    }
}
