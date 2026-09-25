//! The bottom-right ammo HUD, Call-of-Duty style, left to right: the current
//! weapon's icon; the rounds in the mag (big) with the reserve count (small)
//! and a bullet icon stacked beside it; a thin divider; then the throwing
//! knife — its count above its icon, its keybind in a key cap below.
//!
//! No panel behind it: every text, icon and line carries a slight black drop
//! shadow instead, so the white stays readable over bright ground and sky.
//!
//! Sizes below are for a window [`HUD_REF_HEIGHT`] logical px tall;
//! [`scale_ammo_hud`] scales the whole thing with the window height so it
//! keeps the same share of the screen at any resolution.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::keybinds::KeyBindings;
use crate::{menu, Weapon, WeaponSlot, HUD_FONT};

/// Window height (logical px) at which the HUD is drawn at its authored
/// sizes. Measured off a Call of Duty screenshot: there the weapon icon is
/// ~15% of the screen's height, and ours is 128 px wide — 128 / 0.151 ≈ 848.
const HUD_REF_HEIGHT: f32 = 848.0;

/// The root of the ammo HUD — [`scale_ammo_hud`] scales it and everything
/// under it.
#[derive(Component)]
pub(crate) struct AmmoHudRoot;

/// An ammo-HUD entity's authored (unscaled) layout and font size, captured
/// the first time [`scale_ammo_hud`] sees it.
#[derive(Component)]
pub(crate) struct HudBase {
    node: Node,
    font_size: Option<f32>,
}

/// The big rounds-in-mag number.
#[derive(Component)]
pub(crate) struct AmmoText;

/// The small reserve count beside [`AmmoText`].
#[derive(Component)]
pub(crate) struct ReserveText;

/// The mag / reserve / bullet group — hidden while the knife is drawn (it has
/// no ammo), taking its layout space with it.
#[derive(Component)]
pub(crate) struct AmmoGroup;

/// The weapon icon (and its drop-shadow copy) — [`update_weapon_icon`] swaps
/// both textures to match `Weapon::slot`.
#[derive(Component, Clone)]
pub(crate) struct WeaponIcon;

/// Throwing knives left, above the knife icon.
#[derive(Component)]
pub(crate) struct KnifeCountText;

/// The throwing-knife keybind label, in its key cap.
#[derive(Component)]
pub(crate) struct KnifeKeyText;

/// The white key cap behind [`KnifeKeyText`].
#[derive(Component)]
pub(crate) struct KnifeKeyCap;

/// The whole throwing-knife group — dimmed once there are none left.
#[derive(Component)]
pub(crate) struct KnifeGroup;

/// Fixed on-screen size of [`WeaponIcon`], regardless of which texture is
/// showing — both `icons/weapons/sniper.png` and `icons/weapons/knife.png` are drawn to this
/// same 1774×887 (2:1) canvas specifically so a weapon swap can never resize
/// this box and shift the ammo readout beside it.
const WEAPON_ICON_SIZE: (f32, f32) = (128.0, 64.0);
/// `icons/weapons/bullet.png` is 1402×1122.
const BULLET_ICON_SIZE: (f32, f32) = (25.0, 20.0);
/// `icons/weapons/throwing_knife.png` is 1536×1024.
const KNIFE_ICON_SIZE: (f32, f32) = (42.0, 28.0);

/// Drop shadow behind the white HUD text and icons.
const SHADOW_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.7);
const SHADOW_OFFSET: f32 = 2.0;
/// Opacity of the knife group once it's out of knives.
const EMPTY_ALPHA: f32 = 0.35;

/// `Weapon::slot` → its HUD icon path under `assets/textures/icons/weapons/`.
fn weapon_icon_path(slot: WeaponSlot) -> &'static str {
    match slot {
        WeaponSlot::Primary => "textures/icons/weapons/sniper.png",
        WeaponSlot::Secondary => "textures/icons/weapons/knife.png",
    }
}

fn text_shadow() -> TextShadow {
    TextShadow {
        offset: Vec2::splat(SHADOW_OFFSET),
        color: SHADOW_COLOR,
    }
}

/// An icon with a drop shadow: a black-tinted copy of the same image, offset
/// down-right, drawn under the real one. `marker` goes on both copies.
fn spawn_shadowed_icon(
    parent: &mut ChildSpawnerCommands,
    image: Handle<Image>,
    size: (f32, f32),
    marker: impl Component + Clone,
) {
    parent
        .spawn(Node {
            width: Val::Px(size.0),
            height: Val::Px(size.1),
            ..default()
        })
        .with_children(|icon| {
            for (offset, color) in [(SHADOW_OFFSET, SHADOW_COLOR), (0.0, Color::WHITE)] {
                icon.spawn((
                    marker.clone(),
                    ImageNode {
                        image: image.clone(),
                        color,
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(offset),
                        top: Val::Px(offset),
                        width: Val::Px(size.0),
                        height: Val::Px(size.1),
                        ..default()
                    },
                ));
            }
        });
}

/// `Clone`-able stand-in for "no extra marker" on an icon.
#[derive(Component, Clone)]
struct PlainIcon;

pub(crate) fn setup_ammo_ui(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    binds: Res<KeyBindings>,
) {
    let font = asset_server.load(HUD_FONT);
    let text = |size: f32| {
        (
            TextFont {
                font: font.clone(),
                font_size: size,
                ..default()
            },
            TextColor(Color::WHITE),
            text_shadow(),
        )
    };

    commands
        .spawn((
            menu::HudElement,
            AmmoHudRoot,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(28.0),
                bottom: Val::Px(20.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(16.0),
                ..default()
            },
        ))
        .with_children(|row| {
            spawn_shadowed_icon(
                row,
                asset_server.load(weapon_icon_path(WeaponSlot::Primary)),
                WEAPON_ICON_SIZE,
                WeaponIcon,
            );

            // Mag (big) | reserve over a bullet (small).
            row.spawn((
                AmmoGroup,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                },
            ))
            .with_children(|ammo| {
                ammo.spawn((AmmoText, Text::new(""), text(48.0)));
                ammo.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((ReserveText, Text::new(""), text(18.0)));
                    spawn_shadowed_icon(
                        col,
                        asset_server.load("textures/icons/weapons/bullet.png"),
                        BULLET_ICON_SIZE,
                        PlainIcon,
                    );
                });
            });

            // Divider.
            row.spawn((
                Node {
                    width: Val::Px(2.0),
                    height: Val::Px(44.0),
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                BoxShadow::new(
                    SHADOW_COLOR,
                    Val::Px(SHADOW_OFFSET),
                    Val::Px(SHADOW_OFFSET),
                    Val::ZERO,
                    Val::Px(1.0),
                ),
            ));

            // Throwing knife: count, icon, key cap.
            row.spawn((
                KnifeGroup,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    ..default()
                },
            ))
            .with_children(|knife| {
                knife.spawn((KnifeCountText, Text::new(""), text(18.0)));
                spawn_shadowed_icon(
                    knife,
                    asset_server.load("textures/icons/weapons/throwing_knife.png"),
                    KNIFE_ICON_SIZE,
                    PlainIcon,
                );
                knife
                    .spawn((
                        KnifeKeyCap,
                        Node {
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                            min_width: Val::Px(24.0),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::WHITE),
                        BorderRadius::all(Val::Px(3.0)),
                        BoxShadow::new(
                            SHADOW_COLOR,
                            Val::Px(SHADOW_OFFSET),
                            Val::Px(SHADOW_OFFSET),
                            Val::ZERO,
                            Val::Px(1.0),
                        ),
                    ))
                    .with_child((
                        KnifeKeyText,
                        Text::new(binds.throwing_knife.label().to_uppercase()),
                        TextFont {
                            font: font.clone(),
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::BLACK),
                    ));
            });
        });
}

/// Keep the mag / reserve / knife counts in sync, and hide the ammo group
/// while the knife is drawn (`Display::None`, so it also gives up its layout
/// space rather than leaving an empty gap).
#[allow(clippy::type_complexity)]
pub(crate) fn update_ammo_ui(
    weapon: Res<Weapon>,
    mut group: Single<&mut Node, With<AmmoGroup>>,
    mut texts: ParamSet<(
        Single<&mut Text, With<AmmoText>>,
        Single<&mut Text, With<ReserveText>>,
        Single<&mut Text, With<KnifeCountText>>,
    )>,
) {
    let display = if weapon.slot == WeaponSlot::Secondary {
        Display::None
    } else {
        Display::Flex
    };
    if group.display != display {
        group.display = display;
    }
    set_text(&mut texts.p0(), weapon.mag.to_string());
    set_text(&mut texts.p1(), weapon.reserve.to_string());
    set_text(&mut texts.p2(), weapon.throwing_knives.to_string());
}

fn set_text(text: &mut Text, wanted: String) {
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// Dim the throwing-knife group once it's out of knives, and keep its key cap
/// matching the current binding.
#[allow(clippy::type_complexity)]
pub(crate) fn update_knife_hud(
    weapon: Res<Weapon>,
    binds: Res<KeyBindings>,
    group: Single<Entity, With<KnifeGroup>>,
    children: Query<&Children>,
    mut key: Single<&mut Text, With<KnifeKeyText>>,
    mut colors: ParamSet<(
        Query<(&mut TextColor, Option<&mut TextShadow>)>,
        Query<&mut ImageNode>,
        // Only the key cap's — every UI `Node` carries a (transparent)
        // `BackgroundColor`, which must stay transparent.
        Query<&mut BackgroundColor, With<KnifeKeyCap>>,
    )>,
    mut last_empty: Local<Option<bool>>,
) {
    if binds.is_changed() {
        set_text(&mut key, binds.throwing_knife.label().to_uppercase());
    }
    let empty = weapon.throwing_knives == 0;
    if *last_empty == Some(empty) {
        return;
    }
    *last_empty = Some(empty);
    let alpha = if empty { EMPTY_ALPHA } else { 1.0 };
    for e in children.iter_descendants(*group) {
        if let Ok((mut c, shadow)) = colors.p0().get_mut(e) {
            c.0.set_alpha(alpha);
            if let Some(mut shadow) = shadow {
                shadow.color.set_alpha(SHADOW_COLOR.alpha() * alpha);
            }
        }
        if let Ok(mut i) = colors.p1().get_mut(e) {
            // The shadow copy keeps its own (lower) opacity, scaled the same.
            let base = if i.color.to_srgba().red < 0.5 { SHADOW_COLOR.alpha() } else { 1.0 };
            i.color.set_alpha(base * alpha);
        }
        if let Ok(mut b) = colors.p2().get_mut(e) {
            b.0.set_alpha(alpha);
        }
    }
}

/// Swap [`WeaponIcon`]'s texture (both it and its shadow) to match
/// `Weapon::slot` whenever it changes.
pub(crate) fn update_weapon_icon(
    weapon: Res<Weapon>,
    asset_server: Res<AssetServer>,
    mut icons: Query<&mut ImageNode, With<WeaponIcon>>,
    mut applied: Local<Option<WeaponSlot>>,
) {
    if applied.is_some_and(|s| s == weapon.slot) {
        return;
    }
    *applied = Some(weapon.slot);
    let image = asset_server.load(weapon_icon_path(weapon.slot));
    for mut icon in &mut icons {
        icon.image = image.clone();
    }
}

fn scale_val(v: Val, s: f32) -> Val {
    match v {
        Val::Px(px) => Val::Px(px * s),
        other => other,
    }
}

fn scale_rect(r: UiRect, s: f32) -> UiRect {
    UiRect {
        left: scale_val(r.left, s),
        right: scale_val(r.right, s),
        top: scale_val(r.top, s),
        bottom: scale_val(r.bottom, s),
    }
}

/// Scale the ammo HUD with the window height (see [`HUD_REF_HEIGHT`]): every
/// pixel size, gap, offset and font size under [`AmmoHudRoot`] is its
/// authored value × `window height / HUD_REF_HEIGHT`. Only the size fields
/// are written — `display` (toggled by [`update_ammo_ui`]) is left alone.
#[allow(clippy::type_complexity)]
pub(crate) fn scale_ammo_hud(
    mut commands: Commands,
    window: Single<&Window, With<PrimaryWindow>>,
    root: Single<Entity, With<AmmoHudRoot>>,
    children: Query<&Children>,
    mut nodes: Query<(&mut Node, Option<&HudBase>)>,
    mut fonts: Query<(&mut TextFont, Option<&HudBase>, Option<&mut TextShadow>)>,
    mut box_shadows: Query<&mut BoxShadow, With<HudBase>>,
    mut radii: Query<&mut BorderRadius, With<KnifeKeyCap>>,
    mut applied: Local<f32>,
) {
    let s = (window.height() / HUD_REF_HEIGHT).max(0.1);
    let entities: Vec<Entity> =
        std::iter::once(*root).chain(children.iter_descendants(*root)).collect();

    // First sighting: remember the authored sizes, then scale next frame.
    let mut captured = false;
    for &e in &entities {
        if let Ok((node, None)) = nodes.get(e) {
            commands.entity(e).insert(HudBase {
                node: node.clone(),
                font_size: fonts.get(e).ok().map(|(f, _, _)| f.font_size),
            });
            captured = true;
        }
    }
    if captured {
        *applied = 0.0;
        return;
    }
    if (*applied - s).abs() < 1e-4 {
        return;
    }
    *applied = s;

    for &e in &entities {
        if let Ok((mut node, Some(base))) = nodes.get_mut(e) {
            let b = &base.node;
            node.left = scale_val(b.left, s);
            node.right = scale_val(b.right, s);
            node.top = scale_val(b.top, s);
            node.bottom = scale_val(b.bottom, s);
            node.width = scale_val(b.width, s);
            node.height = scale_val(b.height, s);
            node.min_width = scale_val(b.min_width, s);
            node.min_height = scale_val(b.min_height, s);
            node.column_gap = scale_val(b.column_gap, s);
            node.row_gap = scale_val(b.row_gap, s);
            node.padding = scale_rect(b.padding, s);
            node.margin = scale_rect(b.margin, s);
        }
        if let Ok((mut font, Some(base), shadow)) = fonts.get_mut(e) {
            if let Some(size) = base.font_size {
                font.font_size = size * s;
            }
            if let Some(mut shadow) = shadow {
                shadow.offset = Vec2::splat(SHADOW_OFFSET * s);
            }
        }
        if let Ok(mut shadow) = box_shadows.get_mut(e) {
            for style in &mut shadow.0 {
                style.x_offset = Val::Px(SHADOW_OFFSET * s);
                style.y_offset = Val::Px(SHADOW_OFFSET * s);
                style.blur_radius = Val::Px(s);
            }
        }
        if let Ok(mut radius) = radii.get_mut(e) {
            *radius = BorderRadius::all(Val::Px(3.0 * s));
        }
    }
}
