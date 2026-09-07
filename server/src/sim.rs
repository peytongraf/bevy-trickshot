//! The authoritative tick: accept client-authoritative movement, then resolve
//! every fire request server-side.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::ballistics::{resolve_shot, Target};
use shared::hitbox::Capsule;
use shared::weapon::WeaponId;
use shared::{GameChannel, PlayerId, PlayerInput, PlayerPose, ShotOutcome, ShotResolved};

/// Nominal player dimensions for hitbox construction. Replace with per-character
/// values once real models exist.
const PLAYER_HEIGHT: f32 = 1.8;
const PLAYER_RADIUS: f32 = 0.35;
const HEAD_RADIUS: f32 = 0.12;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (apply_client_pose, resolve_shots).chain());
    }
}

/// Client-authoritative movement: publish the owner's reported pose as-is.
fn apply_client_pose(mut players: Query<(&mut PlayerPose, &ActionState<PlayerInput>)>) {
    for (mut pose, input) in &mut players {
        let i = &input.0;
        pose.translation = Vec3::from_array(i.translation);
        pose.yaw = i.yaw;
        pose.pitch = i.pitch;
    }
}

/// Server-authoritative shots: for each client that fired this tick, ray-cast
/// against every other player and broadcast the outcome.
fn resolve_shots(
    timeline: Single<&LocalTimeline, With<Server>>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    shooters: Query<(&PlayerId, &ActionState<PlayerInput>)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
) {
    let tick = timeline.tick().0;
    let server = server.into_inner();

    for (shooter, input) in &shooters {
        let i = &input.0;
        if !i.fire {
            continue;
        }
        let Some(weapon) = WeaponId::from_u8(i.weapon) else {
            continue;
        };

        let targets: Vec<Target> = poses
            .iter()
            .filter(|(id, _)| id.0 != shooter.0)
            .map(|(id, pose)| Target {
                id: id.0.to_bits(),
                body: Capsule::standing(pose.translation, PLAYER_HEIGHT, PLAYER_RADIUS),
                head: Capsule::head(pose.translation, PLAYER_HEIGHT, HEAD_RADIUS),
            })
            .collect();

        let outcome = match resolve_shot(
            weapon,
            Vec3::from_array(i.fire_origin),
            Vec3::from_array(i.fire_dir),
            &targets,
            // TODO: swap for your map's occlusion test — shared::map::CollisionWorld.
            |_from, _to| false,
        ) {
            Some(hit) => ShotOutcome::Hit {
                target: hit.target,
                headshot: hit.headshot,
                point: hit.point.to_array(),
                damage: hit.damage,
            },
            None => ShotOutcome::Miss,
        };

        if let ShotOutcome::Hit {
            target, headshot, ..
        } = outcome
        {
            info!(
                "tick {tick}: {:?} {} player {target}",
                shooter.0,
                if headshot { "HEADSHOT on" } else { "hit" }
            );
        }

        let msg = ShotResolved {
            shooter: shooter.0,
            tick,
            outcome,
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::All) {
            error!("failed to broadcast shot result: {e:?}");
        }
    }
}
