//! The bottom-right ammo readout, plus the weapon icon shown next to it.

use bevy::prelude::*;

use crate::{menu, Weapon, WeaponSlot, HUD_FONT};

/// The bottom-right ammo readout (`mag / reserve`).
#[derive(Component)]
pub(crate) struct AmmoText;

/// The weapon icon next to [`AmmoText`] — [`update_weapon_icon`] swaps its
/// texture to match `Weapon::slot`.
#[derive(Component)]
pub(crate) struct WeaponIcon;

/// Fixed on-screen size of [`WeaponIcon`], regardless of which texture is
/// showing — both `sniper_icon.png` and `knife_icon.png` are drawn to this
/// same 1774×887 (2:1) canvas specifically so a weapon swap can never resize
/// this box and shift the ammo readout beside it.
const WEAPON_ICON_SIZE: (f32, f32) = (128.0, 64.0);

/// `Weapon::slot` → its HUD icon path under `assets/textures/icons/`.
fn weapon_icon_path(slot: WeaponSlot) -> &'static str {
    match slot {
        WeaponSlot::Primary => "textures/icons/sniper_icon.png",
        WeaponSlot::Secondary => "textures/icons/knife_icon.png",
    }
}

/// Bottom-right HUD: the current weapon's icon, then the ammo readout
/// (rounds in the mag, then rounds in reserve) — one row in one panel, so the
/// panel stays anchored to the bottom-right corner; only
/// [`update_weapon_icon`]'s texture swap and [`update_ammo_ui`]'s text change
/// inside it (the text is dropped entirely while the knife is out — it has no
/// ammo — so the panel shrinks to just the icon).
pub(crate) fn setup_ammo_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                bottom: Val::Px(18.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_children(|row| {
            row.spawn((
                WeaponIcon,
                ImageNode {
                    image: asset_server.load(weapon_icon_path(WeaponSlot::Primary)),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Node {
                    width: Val::Px(WEAPON_ICON_SIZE.0),
                    height: Val::Px(WEAPON_ICON_SIZE.1),
                    ..default()
                },
            ));
            row.spawn((
                AmmoText,
                Text::new(""),
                TextFont {
                    font: asset_server.load(HUD_FONT),
                    font_size: 30.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

/// Keep the bottom-right readout in sync with the ammo counts, and hide it
/// while the knife is drawn (`Display::None`, so it also gives up its layout
/// space rather than leaving an empty gap in the panel).
pub(crate) fn update_ammo_ui(
    weapon: Res<Weapon>,
    mut text: Single<(&mut Text, &mut Node), With<AmmoText>>,
) {
    let (text, node) = &mut *text;
    let display = if weapon.slot == WeaponSlot::Secondary {
        Display::None
    } else {
        Display::Flex
    };
    if node.display != display {
        node.display = display;
    }
    let wanted = format!("{} / {}", weapon.mag, weapon.reserve);
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// Swap [`WeaponIcon`]'s texture to match `Weapon::slot` whenever it changes.
pub(crate) fn update_weapon_icon(
    weapon: Res<Weapon>,
    asset_server: Res<AssetServer>,
    mut icon: Single<&mut ImageNode, With<WeaponIcon>>,
    mut applied: Local<Option<WeaponSlot>>,
) {
    if applied.is_some_and(|s| s == weapon.slot) {
        return;
    }
    *applied = Some(weapon.slot);
    icon.image = asset_server.load(weapon_icon_path(weapon.slot));
}
