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

/// Installs the shared hover-tint and button-sound systems. Added once from
/// `main`.
pub struct UiKitPlugin;

impl Plugin for UiKitPlugin {
    fn build(&self, app: &mut App) {
        app
            // `setup_ui_sfx` only loads asset handles into a resource — it
            // doesn't touch the 3D cameras — but it must still go in
            // `PostStartup`, not `Startup`: adding *any* extra system to
            // `main`'s `Startup` set (even a no-op) perturbs its fragile
            // ordering and blacks out the in-game 3D view.
            .add_systems(PostStartup, setup_ui_sfx)
            .add_systems(Update, (hover_tint, play_ui_sfx));
    }
}

// ---- click / hover sounds --------------------------------------------------

/// UI sound effects, loaded once at startup. See [`UiSound`] for how buttons
/// pick which of these to play.
#[derive(Resource)]
pub struct UiSfx {
    pub menu_hover: Handle<AudioSource>,
    pub menu_select: Handle<AudioSource>,
    pub button_hover: Handle<AudioSource>,
    pub button_click: Handle<AudioSource>,
    /// Loaded and ready, but not wired to anything yet — nothing in the UI
    /// currently has an unselectable option to play it for. Hook it up (spawn
    /// `AudioPlayer::new(sfx.denied.clone())`) once one exists.
    #[allow(dead_code)]
    pub denied: Handle<AudioSource>,
    pub menu_back: Handle<AudioSource>,
}

fn setup_ui_sfx(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(UiSfx {
        menu_hover: asset_server.load("audio/ui/menu-hover-sound.mp3"),
        menu_select: asset_server.load("audio/ui/menu-select-sound.mp3"),
        button_hover: asset_server.load("audio/ui/button-hover-sound.mp3"),
        button_click: asset_server.load("audio/ui/button-click-sound.mp3"),
        denied: asset_server.load("audio/ui/denied-sound.mp3"),
        menu_back: asset_server.load("audio/ui/menu-back-sound.mp3"),
    });
}

/// Which pair of hover/click sounds a button uses. `hover` plays on entering
/// [`Interaction::Hovered`] from [`Interaction::None`]; `click` plays on
/// entering [`Interaction::Pressed`] from anything else.
#[derive(Component, Clone, Copy)]
pub struct UiSound {
    hover: fn(&UiSfx) -> Handle<AudioSource>,
    click: fn(&UiSfx) -> Handle<AudioSource>,
}

impl UiSound {
    /// Start-menu / lobby-screen buttons: `menu-hover-sound` / `menu-select-sound`.
    pub const MENU: UiSound = UiSound {
        hover: |s| s.menu_hover.clone(),
        click: |s| s.menu_select.clone(),
    };
    /// Same hover as [`Self::MENU`], but the click is a "go back" navigation
    /// (e.g. leaving a lobby back to the browser) — `menu-back-sound`.
    pub const MENU_BACK: UiSound = UiSound {
        hover: |s| s.menu_hover.clone(),
        click: |s| s.menu_back.clone(),
    };
    /// In-game (Esc) pause-menu buttons: `button-hover-sound` / `button-click-sound`.
    pub const BUTTON: UiSound = UiSound {
        hover: |s| s.button_hover.clone(),
        click: |s| s.button_click.clone(),
    };
    /// Same hover as [`Self::BUTTON`], but the click is a "go back" navigation
    /// (leaving the game back to the main menu) — `menu-back-sound`.
    pub const BUTTON_BACK: UiSound = UiSound {
        hover: |s| s.button_hover.clone(),
        click: |s| s.menu_back.clone(),
    };
}

/// Bundle to attach to a manually-built `Button` entity so it gets hover/click
/// sounds via [`UiSound`] — for the handful of buttons that don't go through
/// [`spawn_button`] / [`spawn_button_hud`] (e.g. a row with custom children).
pub fn ui_sound(sfx: UiSound) -> impl Bundle {
    (sfx, SfxState::default())
}

/// Tracks each button's `Interaction` from the previous frame so
/// [`play_ui_sfx`] can tell "just entered Hovered" / "just entered Pressed"
/// apart from every other transition (e.g. Pressed -> Hovered on release,
/// which would otherwise replay the hover sound right on top of the click).
#[derive(Component, Default)]
struct SfxState(Interaction);

fn play_ui_sfx(
    mut commands: Commands,
    sfx: Res<UiSfx>,
    mut q: Query<(&Interaction, &mut SfxState, &UiSound)>,
) {
    for (interaction, mut state, sound) in &mut q {
        if *interaction == state.0 {
            continue;
        }
        let clip = match (*interaction, state.0) {
            (Interaction::Hovered, Interaction::None) => Some((sound.hover)(&sfx)),
            (Interaction::Pressed, prev) if prev != Interaction::Pressed => {
                Some((sound.click)(&sfx))
            }
            _ => None,
        };
        if let Some(clip) = clip {
            commands.spawn((AudioPlayer::new(clip), PlaybackSettings::DESPAWN));
        }
        state.0 = *interaction;
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
/// its own click-intent enum). `sfx` picks the hover/click sound pair (see
/// [`UiSound`]).
#[allow(clippy::too_many_arguments)]
pub fn spawn_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    size: f32,
    marker: M,
    normal: Color,
    hover: Color,
    text_color: Color,
    sfx: UiSound,
) {
    parent
        .spawn((
            Button,
            Interaction::default(),
            marker,
            Hoverable { normal, hover },
            ui_sound(sfx),
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
    sfx: UiSound,
) {
    parent
        .spawn((
            Button,
            Interaction::default(),
            marker,
            Hoverable { normal, hover },
            ui_sound(sfx),
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
