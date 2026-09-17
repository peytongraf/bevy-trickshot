//! The top-left frames-per-second readout.

use bevy::prelude::*;

use crate::{menu, HUD_FONT};

/// The top-left FPS readout.
#[derive(Component)]
pub(crate) struct FpsText;

/// Top-left frames-per-second readout.
pub(crate) fn setup_fps_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(18.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_child((
            FpsText,
            Text::new(""),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// Accumulator for the FPS readout: frames and seconds since the last update.
#[derive(Default)]
pub(crate) struct FpsAccum {
    elapsed: f32,
    frames: u32,
}

/// Refresh the top-left readout twice a second with the average FPS over each
/// 500 ms window.
pub(crate) fn update_fps_ui(
    time: Res<Time>,
    mut acc: Local<FpsAccum>,
    mut text: Single<&mut Text, With<FpsText>>,
) {
    acc.elapsed += time.delta_secs();
    acc.frames += 1;

    if acc.elapsed >= 0.5 {
        let fps = acc.frames as f32 / acc.elapsed;
        let wanted = format!("{fps:.0} fps");
        if text.0 != wanted {
            text.0 = wanted;
        }
        acc.elapsed = 0.0;
        acc.frames = 0;
    }
}
