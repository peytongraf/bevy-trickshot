//! The debug "teleport home" key: a saved position/facing the player can
//! snap back to, and the on-screen toast confirming a new point was saved.

use bevy::prelude::*;

use crate::keybinds::KeyBindings;
use crate::{AppState, GameSounds, HUD_FONT};

use super::camera::PlayerHead;
use super::movement::{Player, PlayerPhysics, EYE_HEIGHT};

/// Where the player spawns, and the initial [`TeleportPoint`] the teleport key
/// returns to. Ground level, facing the basic map's ramp. Follows
/// `MapSettings`'s default placement — if you move the map far from its default,
/// update this too.
pub(crate) const SPAWN_POS: Vec3 = Vec3::new(26.0, EYE_HEIGHT, -15.0);

/// The position and facing the teleport key snaps the player back to. Starts
/// at [`SPAWN_POS`] facing -Z (the spawn orientation); the "save teleport
/// point" key resets it to wherever the player is standing and looking.
/// Runtime-only — back to spawn on each launch.
#[derive(Resource)]
pub(crate) struct TeleportPoint {
    position: Vec3,
    /// Body yaw (rotation about Y), radians — matches `Player`'s `Transform`.
    yaw: f32,
    /// Head pitch, radians — matches `PlayerHead`'s `Transform`.
    pitch: f32,
}

impl Default for TeleportPoint {
    fn default() -> Self {
        Self {
            position: SPAWN_POS,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// Snaps the player back to the current [`TeleportPoint`], restoring the
/// saved facing, and plays the teleport sound.
pub(crate) fn teleport_home(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    point: Res<TeleportPoint>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
    mut player: Single<(&mut Transform, &mut PlayerPhysics), (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    if binds.teleport_home.just_pressed(&keys, &mouse) {
        let (transform, physics) = &mut *player;
        transform.translation = point.position;
        transform.rotation = Quat::from_rotation_y(point.yaw);
        head.rotation = Quat::from_rotation_x(point.pitch);
        physics.horizontal_velocity = Vec3::ZERO;
        physics.vertical_velocity = 0.0;
        physics.grounded = true;
        commands.spawn((
            AudioPlayer::new(sounds.teleport.clone()),
            PlaybackSettings::DESPAWN,
        ));
    }
}

/// Reset the [`TeleportPoint`] to the player's current position and facing,
/// and flash a "Teleport point saved" toast in the centre of the screen.
pub(crate) fn save_teleport_point(
    (keys, mouse): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    binds: Res<KeyBindings>,
    asset_server: Res<AssetServer>,
    mut point: ResMut<TeleportPoint>,
    player: Single<&Transform, (With<Player>, Without<PlayerHead>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<Player>)>,
    existing: Query<Entity, With<TeleportToast>>,
    mut commands: Commands,
) {
    if !binds.save_teleport_point.just_pressed(&keys, &mouse) {
        return;
    }
    point.position = player.translation;
    point.yaw = player.rotation.to_euler(EulerRot::YXZ).0;
    point.pitch = head.rotation.to_euler(EulerRot::YXZ).1;

    for e in &existing {
        commands.entity(e).despawn();
    }
    commands
        .spawn((
            TeleportToast { age: 0.0 },
            StateScoped(AppState::InGame),
            GlobalZIndex(9),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(47.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_child((
            Text::new("Teleport point saved"),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 30.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// The centre-screen "Teleport point saved" message: held briefly, then faded.
#[derive(Component)]
pub(crate) struct TeleportToast {
    age: f32,
}

pub(crate) const TELEPORT_TOAST_HOLD: f32 = 1.0;
pub(crate) const TELEPORT_TOAST_TTL: f32 = 1.8;

/// Hold the toast, then fade it out and despawn — mirrors [`update_score_popups`].
pub(crate) fn update_teleport_toast(
    time: Res<Time>,
    mut toasts: Query<(Entity, &mut TeleportToast, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (entity, mut toast, children) in &mut toasts {
        toast.age += time.delta_secs();
        if toast.age >= TELEPORT_TOAST_TTL {
            commands.entity(entity).despawn();
            continue;
        }
        let a = if toast.age < TELEPORT_TOAST_HOLD {
            1.0
        } else {
            1.0 - (toast.age - TELEPORT_TOAST_HOLD) / (TELEPORT_TOAST_TTL - TELEPORT_TOAST_HOLD)
        };
        for child in children {
            if let Ok(mut tc) = texts.get_mut(*child) {
                tc.0 = Color::WHITE.with_alpha(a.clamp(0.0, 1.0));
            }
        }
    }
}
