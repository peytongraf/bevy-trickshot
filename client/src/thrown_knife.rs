//! Thrown throwing knives, client side: send the throw request when the
//! knife leaves the player's hand, and draw every [`shared::ThrownKnife`] the
//! server replicates (the local player's and everyone else's alike).
//!
//! Sounds: the whoosh that follows each knife ([`KnifeAirSound`], spatial,
//! riding the knife's avatar and re-scaled by distance every frame) and the
//! hit sound the server announces at the spot a knife kills someone
//! ([`receive_knife_hits`]), and one of the impact clips from where a knife
//! strikes a wall / the ground / a crate ([`receive_knife_impacts`]). The throw sound itself rides the one-shot sound
//! bits (`killcam::SND_THROW`), so it needs nothing here.
//!
//! The server owns the whole flight — arc, bounces off the map's collision
//! mesh, spin, hits, when it stops and is removed (`server::knives`,
//! `shared::throwing_knife`); the client only shows the interpolated result.

use bevy::math::Mat3;
use bevy::prelude::*;
use lightyear::prelude::*;

use shared::ThrownKnife;

use bevy::audio::Volume;

use crate::killcam::ActiveKillCam;
use crate::net::GameClient;
use crate::{
    distance_falloff, lay_knife_trail, positional_playback, update_knife_trails, AppState,
    GameSounds, KnifeSounds, KnifeTrailHead, RemoteSoundEmitter, RemoteSoundSettings,
    SoundVolumes, ThrowingKnife, TracerAssets, TracerSettings, WorldModelCamera,
};

/// Uniform scale of the knife model in the world. `throwing_knife.glb` is
/// ~3.7 units long; this makes it ~26 cm, the same size as the knife shown in
/// the throwing arms' hand (arms scale 0.01 × knife scale 7).
pub(crate) const KNIFE_WORLD_SCALE: f32 = 0.07;

pub struct ThrownKnifePlugin;

impl Plugin for ThrownKnifePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                send_throw_requests,
                spawn_knife_avatars,
                follow_knife_avatars,
                update_knife_trails,
                update_knife_air_sounds,
                receive_knife_hits,
                receive_knife_impacts,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Stands in for one server-owned [`ThrownKnife`], as `models/throwing_knife.glb`.
#[derive(Component)]
struct KnifeAvatar {
    src: Entity,
}

/// The whoosh following one thrown knife: a child of its avatar (so it moves
/// with it) that plays the clip once, panned by where the knife is relative to
/// the listener. `src` is the replicated [`ThrownKnife`] entity; the sound is
/// cut off when the knife stops or is removed.
#[derive(Component)]
struct KnifeAirSound {
    src: Entity,
}

/// The model's own axes → the replicated rotation's frame (`-Z` blade tip,
/// `Y` the flat face's normal): the glb's blade tip points along `-X` and its
/// flat face's normal is `Z`, its width `Y`.
pub(crate) fn model_correction() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y))
}

/// Send the throw request `weapon_system` filed when the knife left the hand.
fn send_throw_requests(
    mut knife: ResMut<ThrowingKnife>,
    mut sender: Query<&mut TriggerSender<shared::ThrowKnife>, With<GameClient>>,
) {
    if !knife.has_throw_request() {
        return;
    }
    let Some((origin, dir)) = knife.take_throw_request() else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::ThrowKnife {
            origin: origin.to_array(),
            dir: dir.to_array(),
        });
    }
}

fn spawn_knife_avatars(
    knives: Query<Entity, (With<ThrownKnife>, With<Interpolated>)>,
    avatars: Query<&KnifeAvatar>,
    asset_server: Res<AssetServer>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    let have: std::collections::HashSet<Entity> = avatars.iter().map(|a| a.src).collect();
    for src in &knives {
        if have.contains(&src) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                KnifeAvatar { src },
                KnifeTrailHead::default(),
                Transform::default(),
                Visibility::default(),
            ))
            .with_child((
                SceneRoot(
                    asset_server
                        .load(GltfAssetLabel::Scene(0).from_asset("models/throwing_knife.glb")),
                ),
                Transform {
                    rotation: model_correction(),
                    scale: Vec3::splat(KNIFE_WORLD_SCALE),
                    ..default()
                },
            ))
            // The whoosh starts the moment the knife shows up. It's spawned
            // silent; `update_knife_air_sounds` sets its real volume (by
            // distance) every frame from the first one on.
            .with_child((
                KnifeAirSound { src },
                RemoteSoundEmitter,
                Transform::default(),
                AudioPlayer::new(sounds.knife_in_air.clone()),
                positional_playback(Volume::Linear(0.0)),
            ));
    }
}

/// Follow the replicated (interpolated) state, laying its faint trail while
/// it flies; drop the avatar once the server removes the knife. Hidden (and
/// trail-less) during a kill cam, which replays its own frozen world.
#[allow(clippy::too_many_arguments)]
fn follow_knife_avatars(
    knives: Query<&ThrownKnife>,
    mut avatars: Query<(
        Entity,
        &KnifeAvatar,
        &mut KnifeTrailHead,
        &mut Transform,
        &mut Visibility,
    )>,
    killcam: Res<ActiveKillCam>,
    trail_assets: Res<TracerAssets>,
    trail_settings: Res<TracerSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for (entity, avatar, mut head, mut tf, mut vis) in &mut avatars {
        match knives.get(avatar.src) {
            Ok(knife) => {
                tf.translation = knife.pos;
                tf.rotation = knife.rot;
                vis.set_if_neq(wanted);
                lay_knife_trail(
                    &mut head,
                    knife.pos,
                    !knife.resting && killcam.0.is_none(),
                    &trail_assets,
                    &trail_settings,
                    &mut materials,
                    &mut commands,
                );
            }
            Err(_) => commands.entity(entity).try_despawn(),
        }
    }
}

/// Keep every knife whoosh's loudness matched to its distance from the
/// listener, and cut it off once the knife has stopped or been removed. Muted
/// during a kill cam (the replay has its own world).
#[allow(clippy::too_many_arguments)]
fn update_knife_air_sounds(
    knives: Query<&ThrownKnife>,
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    killcam: Res<ActiveKillCam>,
    sound_vol: Res<SoundVolumes>,
    remote: Res<RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut air: Query<(Entity, &KnifeAirSound, &GlobalTransform, Option<&mut SpatialAudioSink>)>,
    mut commands: Commands,
) {
    let ear = listener.single().ok().map(|t| t.translation());
    for (entity, sound, gt, sink) in &mut air {
        let alive = knives.get(sound.src).is_ok_and(|k| !k.resting);
        if !alive {
            commands.entity(entity).try_despawn();
            continue;
        }
        let (Some(mut sink), Some(ear)) = (sink, ear) else {
            continue; // not started yet (still loading) — or no listener
        };
        let loudness = if killcam.0.is_some() {
            0.0
        } else {
            sound_vol.knife_in_air
                * distance_falloff(ear.distance(gt.translation()), &remote)
                * remote.volume
        };
        sink.set_volume(Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

/// The server announced a thrown knife killing someone at some point: play
/// the hit sound from there, fading with distance like other players' sounds.
#[allow(clippy::too_many_arguments)]
fn receive_knife_hits(
    mut receivers: Query<&mut MessageReceiver<shared::ThrowingKnifeHit>>,
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    killcam: Res<ActiveKillCam>,
    sounds: Res<GameSounds>,
    sound_vol: Res<SoundVolumes>,
    remote: Res<RemoteSoundSettings>,
    mut commands: Commands,
) {
    let ear = listener.single().ok().map(|t| t.translation());
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(ear) = ear else { continue };
            if killcam.0.is_some() {
                continue; // a replay is playing; it's the live world's hit
            }
            let pos = Vec3::from_array(msg.point);
            let loudness =
                sound_vol.knife_hit * distance_falloff(ear.distance(pos), &remote) * remote.volume;
            if loudness <= 0.0 {
                continue;
            }
            commands.spawn((
                RemoteSoundEmitter,
                AudioPlayer::new(sounds.knife_hit.clone()),
                Transform::from_translation(pos),
                // (Bevy multiplies in `GlobalVolume` itself when the sound starts.)
                positional_playback(Volume::Linear(loudness)),
            ));
        }
    }
}

/// The server announced a thrown knife striking a surface (not a bot or
/// player): play one of the impact clips from that point. The server picked
/// the clip (`variant`), so every player hears the same one.
#[allow(clippy::too_many_arguments)]
fn receive_knife_impacts(
    mut receivers: Query<&mut MessageReceiver<shared::ThrowingKnifeImpact>>,
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    killcam: Res<ActiveKillCam>,
    knife_sounds: Res<KnifeSounds>,
    remote: Res<RemoteSoundSettings>,
    mut commands: Commands,
) {
    let ear = listener.single().ok().map(|t| t.translation());
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(ear) = ear else { continue };
            if killcam.0.is_some() {
                continue;
            }
            let Some(clip) = knife_sounds.impact.pick(msg.variant) else {
                continue;
            };
            let pos = Vec3::from_array(msg.point);
            let loudness = clip.volume * distance_falloff(ear.distance(pos), &remote) * remote.volume;
            if loudness <= 0.0 {
                continue;
            }
            commands.spawn((
                RemoteSoundEmitter,
                AudioPlayer::new(clip.handle.clone()),
                Transform::from_translation(pos),
                positional_playback(Volume::Linear(loudness)),
            ));
        }
    }
}
