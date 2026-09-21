//! Top-centre debug readout: where the player is and which way they're facing.
//! Only shown while Debug Mode is on (`Settings::debug_mode`, the same toggle
//! as the egui tuning panels).

use bevy::prelude::*;

use crate::killcam::ActiveKillCam;
use crate::settings::Settings;
use crate::{Player, PlayerHead, AppState, HUD_FONT};

/// The readout's background box.
#[derive(Component)]
pub(crate) struct DebugReadoutRoot;

/// Its text.
#[derive(Component)]
pub(crate) struct DebugReadoutText;

/// Spawn the (hidden) readout on entering a game. Sits just under the match
/// timer, which owns the very top of the screen.
pub(crate) fn spawn_debug_readout(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            DebugReadoutRoot,
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(58.0),
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                BorderRadius::all(Val::Px(6.0)),
            ))
            .with_child((
                DebugReadoutText,
                Text::new(""),
                TextFont {
                    font: asset_server.load(HUD_FONT),
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(JustifyText::Center),
            ));
        });
}

/// Show / hide the readout with Debug Mode and keep its numbers current: the
/// player's world position (the same point the `P` key logs — the eye), and
/// which way they're looking: **yaw** (left / right, degrees; `0` faces `-Z`,
/// positive turns left) and **pitch** (up / down, degrees; positive looks up).
pub(crate) fn update_debug_readout(
    settings: Res<Settings>,
    killcam: Res<ActiveKillCam>,
    player: Single<&Transform, (With<Player>, Without<PlayerHead>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<Player>)>,
    mut root: Single<&mut Visibility, With<DebugReadoutRoot>>,
    mut text: Single<&mut Text, With<DebugReadoutText>>,
) {
    // A kill cam flies the rig along someone else's recorded path — not "you".
    let show = settings.debug_mode && killcam.0.is_none();
    root.set_if_neq(if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    if !show {
        return;
    }

    let p = player.translation;
    let yaw = player.rotation.to_euler(EulerRot::YXZ).0.to_degrees();
    let pitch = head.rotation.to_euler(EulerRot::YXZ).1.to_degrees();
    let wanted = format!(
        "pos   x {:.2}   y {:.2}   z {:.2}\nlook  yaw {} (left / right)   pitch {} (up / down)",
        p.x,
        p.y,
        p.z,
        signed_label(yaw, "left", "right"),
        signed_label(pitch, "up", "down"),
    );
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// `"12.3° left"` / `"4.0° right"` — the angle's size and which way it points
/// (`positive` for a positive angle, `negative` for a negative one).
fn signed_label(degrees: f32, positive: &str, negative: &str) -> String {
    let side = if degrees >= 0.0 { positive } else { negative };
    format!("{:.1}° {side}", degrees.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_read_as_a_size_and_a_direction() {
        assert_eq!(signed_label(90.0, "left", "right"), "90.0° left");
        assert_eq!(signed_label(-20.04, "left", "right"), "20.0° right");
        assert_eq!(signed_label(0.0, "up", "down"), "0.0° up");
        assert_eq!(signed_label(-10.0, "up", "down"), "10.0° down");
    }
}
