//! Thrown throwing knives, client side: send the throw request when the
//! knife leaves the player's hand, and draw every [`shared::ThrownKnife`] the
//! server replicates (the local player's and everyone else's alike).
//!
//! The server owns the whole flight — arc, bounces off the map's collision
//! mesh, spin, hits, when it stops and is removed (`server::knives`,
//! `shared::throwing_knife`); the client only shows the interpolated result.

use bevy::math::Mat3;
use bevy::prelude::*;
use lightyear::prelude::*;

use shared::ThrownKnife;

use crate::killcam::ActiveKillCam;
use crate::net::GameClient;
use crate::{AppState, ThrowingKnife};

/// Uniform scale of the knife model in the world. `throwing_knife.glb` is
/// ~3.7 units long; this makes it ~26 cm, the same size as the knife shown in
/// the throwing arms' hand (arms scale 0.01 × knife scale 7).
const KNIFE_WORLD_SCALE: f32 = 0.07;

pub struct ThrownKnifePlugin;

impl Plugin for ThrownKnifePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                send_throw_requests,
                spawn_knife_avatars,
                follow_knife_avatars,
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

/// The model's own axes → the replicated rotation's frame (`-Z` blade tip,
/// `Y` the flat face's normal): the glb's blade tip points along `-X` and its
/// flat face's normal is `Z`, its width `Y`.
fn model_correction() -> Quat {
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
            ));
    }
}

/// Follow the replicated (interpolated) state; drop the avatar once the
/// server removes the knife. Hidden during a kill cam, which replays its own
/// frozen world.
fn follow_knife_avatars(
    knives: Query<&ThrownKnife>,
    mut avatars: Query<(Entity, &KnifeAvatar, &mut Transform, &mut Visibility)>,
    killcam: Res<ActiveKillCam>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for (entity, avatar, mut tf, mut vis) in &mut avatars {
        match knives.get(avatar.src) {
            Ok(knife) => {
                tf.translation = knife.pos;
                tf.rotation = knife.rot;
                vis.set_if_neq(wanted);
            }
            Err(_) => commands.entity(entity).try_despawn(),
        }
    }
}
