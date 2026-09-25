//! Gun-mounted flashlights for `BreakPointNight`: a spotlight on the local
//! player's view (a child of the [`WorldModelCamera`], offset to where a
//! rail light would sit on the gun, so it turns, shakes and recoils with the
//! view) and the same light on every other player — real or `FreeForAll` bot,
//! not `Zombies` zombies — following where their replicated pose is looking.
//!
//! Always on on that map and off everywhere else (the debug panel's
//! "Flashlight" section can force it on anywhere for tuning). The local light
//! is a child of the camera and the remote ones are `StateScoped(InGame)`, so
//! nothing survives the game.

use bevy::prelude::*;
use lightyear::prelude::LocalId;
use shared::{GameMode, Lobby, PlayerId, PlayerPose};

use crate::environment::CurrentMap;
use crate::net::{GameClient, RemoteAvatar};
use crate::player::WorldModelCamera;
use crate::util::color_from_parts;
use crate::AppState;

pub(crate) struct FlashlightPlugin;

impl Plugin for FlashlightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlashlightSettings>().add_systems(
            Update,
            (
                sync_local_flashlight,
                spawn_remote_flashlights,
                update_remote_flashlights,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Panel-tunable flashlight ("Flashlight" debug section).
#[derive(Resource, Clone)]
pub(crate) struct FlashlightSettings {
    /// Debug: on on every map, not just `BreakPointNight`.
    pub(crate) force_on: bool,
    /// Brightness (lumens, Bevy's `SpotLight::intensity`).
    pub(crate) intensity: f32,
    /// How far (m) it reaches.
    pub(crate) range: f32,
    /// Beam edge / fully-bright core half-angles (degrees).
    pub(crate) outer_angle_deg: f32,
    pub(crate) inner_angle_deg: f32,
    /// sRGB colour.
    pub(crate) color: [f32; 3],
    /// Where it sits relative to the eye (m): right, up, forward is -z.
    pub(crate) offset: Vec3,
    /// Whether the local light casts shadows.
    pub(crate) shadows: bool,
    /// Other players' lights: brightness as a multiple of ours...
    pub(crate) remote_intensity_mult: f32,
    /// ...and whether they cast shadows (costly with many players).
    pub(crate) remote_shadows: bool,
}

impl Default for FlashlightSettings {
    fn default() -> Self {
        Self {
            force_on: false,
            intensity: 10_000_000.0,
            range: 45.0,
            outer_angle_deg: 43.0,
            inner_angle_deg: 38.0,
            color: [1.0, 0.95, 0.85],
            offset: Vec3::new(0.0, 0.03, -0.22),
            shadows: true,
            remote_intensity_mult: 1.0,
            remote_shadows: false,
        }
    }
}

impl FlashlightSettings {
    /// Whether flashlights are on for `map`.
    pub(crate) fn on(&self, map: shared::MapId) -> bool {
        self.force_on || map == shared::MapId::BreakPointNight
    }

    fn light(&self, intensity_mult: f32, shadows: bool) -> SpotLight {
        let outer = self.outer_angle_deg.to_radians();
        SpotLight {
            color: color_from_parts(self.color),
            intensity: self.intensity * intensity_mult,
            range: self.range,
            outer_angle: outer,
            inner_angle: self.inner_angle_deg.to_radians().min(outer),
            shadows_enabled: shadows,
            ..default()
        }
    }
}

/// The local player's flashlight (a child of the world camera).
#[derive(Component)]
struct LocalFlashlight;

/// Another player's flashlight, following the [`PlayerPose`] on `src`.
#[derive(Component)]
struct RemoteFlashlight {
    src: Entity,
}

fn visibility(on: bool) -> Visibility {
    if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

/// Give the world camera its flashlight (the camera is respawned each game)
/// and keep it matching the settings and the map.
fn sync_local_flashlight(
    settings: Res<FlashlightSettings>,
    current: Res<CurrentMap>,
    camera: Query<Entity, With<WorldModelCamera>>,
    mut lights: Query<(&mut SpotLight, &mut Transform, &mut Visibility), With<LocalFlashlight>>,
    mut commands: Commands,
) {
    let on = settings.on(current.0);
    if lights.is_empty() {
        if let Ok(cam) = camera.single() {
            commands.spawn((
                LocalFlashlight,
                settings.light(1.0, settings.shadows),
                Transform::from_translation(settings.offset),
                visibility(on),
                ChildOf(cam),
            ));
        }
        return;
    }
    for (mut light, mut tf, mut vis) in &mut lights {
        *light = settings.light(1.0, settings.shadows);
        tf.translation = settings.offset;
        vis.set_if_neq(visibility(on));
    }
}

/// Whether `id` is a `Zombies` zombie (no flashlight on those).
fn is_zombie(id: &PlayerId, mode: Option<GameMode>) -> bool {
    mode == Some(GameMode::Zombies) && shared::bot_players::is_bot_peer(id.0)
}

/// A light for every other player's avatar that doesn't have one yet.
fn spawn_remote_flashlights(
    settings: Res<FlashlightSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<&RemoteAvatar>,
    players: Query<&PlayerId>,
    lights: Query<&RemoteFlashlight>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    let mode = me.and_then(|me| lobbies.iter().find(|l| l.has(me))).map(|l| l.mode);
    for avatar in &avatars {
        // Only players (not `Freestyle` target stand-ins) and not zombies.
        let Ok(id) = players.get(avatar.src) else {
            continue;
        };
        if is_zombie(id, mode) || lights.iter().any(|l| l.src == avatar.src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            RemoteFlashlight { src: avatar.src },
            settings.light(settings.remote_intensity_mult, settings.remote_shadows),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
}

/// Aim each remote light where its player is looking, from their gun; drop
/// it once they're gone. Off while they're dead, or off this map.
fn update_remote_flashlights(
    settings: Res<FlashlightSettings>,
    current: Res<CurrentMap>,
    poses: Query<&PlayerPose>,
    mut lights: Query<(Entity, &RemoteFlashlight, &mut SpotLight, &mut Transform, &mut Visibility)>,
    mut commands: Commands,
) {
    let on = settings.on(current.0);
    for (entity, flash, mut light, mut tf, mut vis) in &mut lights {
        let Ok(pose) = poses.get(flash.src) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        // Same composition as the local rig: yaw on the body, pitch on the head.
        let aim = Quat::from_rotation_y(pose.yaw) * Quat::from_rotation_x(pose.pitch);
        tf.translation = pose.translation + aim * settings.offset;
        tf.rotation = aim;
        *light = settings.light(settings.remote_intensity_mult, settings.remote_shadows);
        vis.set_if_neq(visibility(on && pose.alive));
    }
}
