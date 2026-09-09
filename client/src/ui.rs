//! Shared `bevy_ui` building blocks — the dark/orange palette and a few spawn
//! helpers used by both the settings menu (`menu.rs`) and the main-menu / lobby
//! screens (`lobby_ui.rs`).
//!
//! A real font can be dropped in later: load `assets/fonts/<x>.ttf` and thread a
//! `Handle<Font>` into `TextFont`. Until then this uses Bevy's embedded default.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;

// ---- palette --------------------------------------------------------------
pub const BACKDROP: Color = Color::srgba(0.03, 0.04, 0.055, 0.9);
pub const PANEL: Color = Color::srgb(0.072, 0.083, 0.10);
pub const PANEL_SOLID: Color = Color::srgb(0.03, 0.035, 0.05);
pub const ROW: Color = Color::srgb(0.11, 0.125, 0.150);
pub const ROW_HOVER: Color = Color::srgb(0.17, 0.19, 0.23);
pub const TRACK: Color = Color::srgb(0.16, 0.175, 0.205);
pub const ACCENT: Color = Color::srgb(0.96, 0.62, 0.12);
pub const ACCENT_DIM: Color = Color::srgb(0.42, 0.30, 0.10);
pub const TEXT: Color = Color::srgb(0.90, 0.92, 0.94);
pub const TEXT_DIM: Color = Color::srgb(0.55, 0.58, 0.63);

/// Installs the shared hover-tint system. Added once from `main`.
pub struct UiKitPlugin;

impl Plugin for UiKitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, hover_tint);
    }
}

/// Put on any `Button` to have [`hover_tint`] swap its background on hover.
#[derive(Component)]
pub struct Hoverable {
    pub normal: Color,
    pub hover: Color,
}

fn hover_tint(
    mut q: Query<(&Interaction, &Hoverable, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, hover, mut bg) in &mut q {
        bg.0 = match interaction {
            Interaction::Hovered | Interaction::Pressed => hover.hover,
            Interaction::None => hover.normal,
        };
    }
}

/// A plain text node.
pub fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

/// A text node set in [`crate::HUD_FONT`] — the bold condensed face used for
/// the in-game HUD (score / ammo / fps) and the main-menu / lobby screens.
pub fn label_hud(
    asset_server: &AssetServer,
    text: impl Into<String>,
    size: f32,
    color: Color,
) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: asset_server.load(crate::HUD_FONT),
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

/// A bordered input-looking box, `width` px wide.
pub fn field_box(width: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(width),
            height: Val::Px(44.0),
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(Val::Px(14.0)),
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(ROW),
        BorderColor(ACCENT),
        BorderRadius::all(Val::Px(4.0)),
    )
}

/// Spawn a labelled button carrying `marker` (any component — each screen uses
/// its own click-intent enum).
pub fn spawn_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    size: f32,
    marker: M,
    normal: Color,
    hover: Color,
    text_color: Color,
) {
    parent
        .spawn((
            Button,
            Interaction::default(),
            marker,
            Hoverable { normal, hover },
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(9.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(normal),
            BorderRadius::all(Val::Px(4.0)),
        ))
        .with_children(|b| {
            b.spawn(label(text, size, text_color));
        });
}

/// [`spawn_button`], but its label is set in [`crate::HUD_FONT`] (see
/// [`label_hud`]).
#[allow(clippy::too_many_arguments)]
pub fn spawn_button_hud<M: Component>(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    text: &str,
    size: f32,
    marker: M,
    normal: Color,
    hover: Color,
    text_color: Color,
) {
    parent
        .spawn((
            Button,
            Interaction::default(),
            marker,
            Hoverable { normal, hover },
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(9.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(normal),
            BorderRadius::all(Val::Px(4.0)),
        ))
        .with_children(|b| {
            b.spawn(label_hud(asset_server, text, size, text_color));
        });
}

/// Full-screen absolute container, centred content. `solid` = opaque panel
/// colour (no world behind it); otherwise a translucent backdrop.
pub fn overlay_root(solid: bool) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(if solid { PANEL_SOLID } else { BACKDROP }),
    )
}
