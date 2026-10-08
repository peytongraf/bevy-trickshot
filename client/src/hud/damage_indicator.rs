//! The damage indicator, Call of Duty's: when someone hurts us, a red arc
//! (`textures/hud/damage_indicator.png`) appears on a ring around the middle
//! of the screen on the side the hit came from, its point toward them. It
//! keeps pointing at where they were as we move and turn — swinging round
//! the ring — and fades out after a few seconds. Another hit from about the
//! same way refreshes it rather than stacking a second.
//!
//! The server says where each hit came from ([`shared::DamageTaken`]); size,
//! placement and timing are [`HurtEffectSettings`]' (the debug panel's "Hurt
//! effects"). Everything is `StateScoped(InGame)` — nothing to reset between
//! games.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use lightyear::prelude::MessageReceiver;

use crate::health::HurtEffectSettings;
use crate::killcam::ActiveKillCam;
use crate::{AppState, WorldModelCamera};

const IMAGE: &str = "textures/hud/damage_indicator.png";
/// The image's width over its height.
const ASPECT: f32 = 2172.0 / 724.0;
/// A new hit within this angle (radians) of one already showing refreshes
/// that one instead of adding another.
const MERGE_ANGLE: f32 = 0.35;

pub(crate) struct DamageIndicatorPlugin;

impl Plugin for DamageIndicatorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), spawn_layer)
            .add_systems(
                Update,
                (receive_damage, update_indicators).chain().run_if(in_state(AppState::InGame)),
            );
    }
}

/// The full-screen layer the indicators sit on.
#[derive(Component)]
struct IndicatorLayer;

/// One indicator: where the hit came from, and how long it's been showing.
#[derive(Component)]
struct DamageIndicator {
    from: Vec3,
    age: f32,
}

fn spawn_layer(mut commands: Commands) {
    commands.spawn((
        Name::new("DamageIndicators"),
        IndicatorLayer,
        StateScoped(AppState::InGame),
        // (Over the blood overlay (9), under the death overlay (10).)
        GlobalZIndex(9),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

/// Where on the ring (radians clockwise from straight up — straight ahead)
/// something at `from` is, for the camera at `cam`.
fn screen_angle(cam: &GlobalTransform, from: Vec3) -> f32 {
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z).normalize_or_zero();
    let to = flat(from - cam.translation());
    let forward = flat(cam.forward().into());
    let right = flat(cam.right().into());
    to.dot(right).atan2(to.dot(forward))
}

/// A hit from somewhere: an indicator pointing there (or the one already
/// pointing about that way, refreshed).
fn receive_damage(
    mut receivers: Query<&mut MessageReceiver<shared::DamageTaken>>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    layer: Query<Entity, With<IndicatorLayer>>,
    mut existing: Query<&mut DamageIndicator>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let (Ok(cam), Ok(layer)) = (cam.single(), layer.single()) else {
        for mut rx in &mut receivers {
            rx.receive().for_each(drop);
        }
        return;
    };
    for mut rx in &mut receivers {
        for hit in rx.receive() {
            let from = Vec3::from_array(hit.from);
            let angle = screen_angle(cam, from);
            let near = existing.iter_mut().find(|d| {
                let diff = (screen_angle(cam, d.from) - angle + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                diff.abs() < MERGE_ANGLE
            });
            if let Some(mut d) = near {
                d.from = from;
                d.age = 0.0;
                continue;
            }
            commands.entity(layer).with_child((
                DamageIndicator { from, age: 0.0 },
                ImageNode::new(asset_server.load(IMAGE)).with_color(Color::srgba(1.0, 1.0, 1.0, 0.0)),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
    }
}

/// Age, place, turn and fade every indicator; remove the ones done.
fn update_indicators(
    time: Res<Time>,
    settings: Res<HurtEffectSettings>,
    killcam: Res<ActiveKillCam>,
    window: Query<&Window, With<PrimaryWindow>>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    mut layer: Query<&mut Visibility, With<IndicatorLayer>>,
    mut indicators: Query<(Entity, &mut DamageIndicator, &mut Node, &mut Transform, &mut ImageNode)>,
    mut commands: Commands,
) {
    if let Ok(mut vis) = layer.single_mut() {
        vis.set_if_neq(if killcam.0.is_some() { Visibility::Hidden } else { Visibility::Inherited });
    }
    let (Ok(window), Ok(cam)) = (window.single(), cam.single()) else {
        return;
    };
    let (w, h) = (window.width(), window.height());
    let width = settings.indicator_size * h;
    let height = width / ASPECT;
    let radius = settings.indicator_radius * h;
    let fade = settings.indicator_fade_secs.clamp(0.01, settings.indicator_secs.max(0.01));
    for (entity, mut d, mut node, mut tf, mut image) in &mut indicators {
        d.age += time.delta_secs();
        if d.age >= settings.indicator_secs {
            commands.entity(entity).despawn();
            continue;
        }
        let angle = screen_angle(cam, d.from);
        // On the ring, centred where it points from; turned to face out.
        let centre = Vec2::new(w * 0.5, h * 0.5) + Vec2::new(angle.sin(), -angle.cos()) * radius;
        node.left = Val::Px(centre.x - width * 0.5);
        node.top = Val::Px(centre.y - height * 0.5);
        node.width = Val::Px(width);
        node.height = Val::Px(height);
        // (UI space runs y down, so a positive turn is clockwise on screen.)
        tf.rotation = Quat::from_rotation_z(angle);
        let left = settings.indicator_secs - d.age;
        let alpha = settings.indicator_opacity * (left / fade).min(1.0);
        image.color = Color::srgba(1.0, 1.0, 1.0, alpha.clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hit_from_ahead_is_up_one_from_the_right_is_clockwise() {
        // A camera at the origin facing -Z (Bevy's forward).
        let cam = GlobalTransform::IDENTITY;
        assert!(screen_angle(&cam, Vec3::new(0.0, 0.0, -10.0)).abs() < 1e-5);
        let right = screen_angle(&cam, Vec3::new(10.0, 3.0, 0.0));
        assert!((right - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{right}");
        let behind = screen_angle(&cam, Vec3::new(0.0, 0.0, 10.0)).abs();
        assert!((behind - std::f32::consts::PI).abs() < 1e-5, "{behind}");
        // Turning to face it brings it round to the top.
        let turned = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)));
        assert!(screen_angle(&turned, Vec3::new(10.0, 0.0, 0.0)).abs() < 1e-4);
    }
}
