//! The bottom-right ammo readout.

use bevy::prelude::*;

use crate::{menu, Weapon, HUD_FONT};

/// The bottom-right ammo readout (`mag / reserve`).
#[derive(Component)]
pub(crate) struct AmmoText;

/// Bottom-right ammo readout: rounds in the mag, then rounds in reserve.
pub(crate) fn setup_ammo_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                bottom: Val::Px(18.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_child((
            AmmoText,
            Text::new(""),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 30.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// Keep the bottom-right readout in sync with the ammo counts.
pub(crate) fn update_ammo_ui(weapon: Res<Weapon>, mut text: Single<&mut Text, With<AmmoText>>) {
    let wanted = format!("{} / {}", weapon.mag, weapon.reserve);
    if text.0 != wanted {
        text.0 = wanted;
    }
}
