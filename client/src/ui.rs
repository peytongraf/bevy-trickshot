//! Shared `bevy_ui` building blocks for the menus — the settings / pause menu
//! (`menu.rs`) and the main-menu / lobby screens (`lobby_ui.rs`) — styled
//! after a modern Call of Duty front end: black (or, in game, see-through
//! black) backgrounds, flat square panels and buttons tinted faintly white,
//! condensed all-caps headings, gold for what's selected / the main action,
//! and a solid white hover with black text.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;

// ---- palette --------------------------------------------------------------
/// In-game menus: black, but see-through enough that the world shows behind.
pub const BACKDROP: Color = Color::srgba(0.0, 0.0, 0.0, 0.78);
/// A panel / card: a faint white lift off the black.
pub const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.035);
/// Solid black — the out-of-game screens' background, and the text colour on
/// gold / white fills.
pub const PANEL_SOLID: Color = Color::BLACK;
/// A button / row at rest.
pub const ROW: Color = Color::srgba(1.0, 1.0, 1.0, 0.07);
/// Hovered: solid white (its text flips to black — see [`Hoverable`]).
pub const ROW_HOVER: Color = Color::srgb(0.94, 0.94, 0.94);
/// Slider tracks, list rows, dividers.
pub const TRACK: Color = Color::srgba(1.0, 1.0, 1.0, 0.12);
/// Gold: what's selected, the screen's main action, and highlights.
pub const ACCENT: Color = Color::srgb(0.96, 0.78, 0.2);
/// A quiet gold wash behind a selected option whose text is [`ACCENT`].
pub const ACCENT_DIM: Color = Color::srgba(0.96, 0.78, 0.2, 0.16);
pub const TEXT: Color = Color::srgb(0.94, 0.94, 0.94);
pub const TEXT_DIM: Color = Color::srgb(0.58, 0.59, 0.62);
/// Hairline borders on panels and buttons.
pub const EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.1);
/// "VICTORY" headline — the match-results screen (`menu.rs`).
pub const VICTORY: Color = Color::srgb(0.16, 0.85, 0.62);
/// "DEFEAT" headline — matches the kill-cam banner's red.
pub const DEFEAT: Color = Color::srgb(0.85, 0.06, 0.06);

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
        menu_hover: asset_server.load("audio/ui/menu_hover.mp3"),
        menu_select: asset_server.load("audio/ui/menu_select.mp3"),
        button_hover: asset_server.load("audio/ui/button_hover.mp3"),
        button_click: asset_server.load("audio/ui/button_click.mp3"),
        denied: asset_server.load("audio/ui/denied.mp3"),
        menu_back: asset_server.load("audio/ui/menu_back.mp3"),
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
/// [`spawn_button_hud`] (e.g. a row with custom children).
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

/// Put on any `Button` to have [`hover_tint`] swap its background on hover —
/// and, if `text` is set, the colour of every text inside it (`(normal,
/// hovered)`), so a white hover can flip its label to black.
#[derive(Component)]
pub struct Hoverable {
    pub normal: Color,
    pub hover: Color,
    pub text: Option<(Color, Color)>,
}

impl Hoverable {
    /// `normal` → `hover` background, with the text in `text` flipped to
    /// black whenever the hover fill is light enough to need it.
    pub fn new(normal: Color, hover: Color, text: Color) -> Self {
        Self {
            normal,
            hover,
            text: Some((text, readable_on(hover, text))),
        }
    }
}

/// `text`, or black if `bg` is too light for it to read.
fn readable_on(bg: Color, text: Color) -> Color {
    let c = bg.to_srgba();
    let light = (0.2126 * c.red + 0.7152 * c.green + 0.0722 * c.blue) * c.alpha > 0.55;
    if light {
        PANEL_SOLID
    } else {
        text
    }
}

fn hover_tint(
    mut q: Query<(Entity, &Interaction, &Hoverable, &mut BackgroundColor), Changed<Interaction>>,
    children: Query<&Children>,
    mut texts: Query<&mut TextColor>,
) {
    for (entity, interaction, hover, mut bg) in &mut q {
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        bg.0 = if hovered { hover.hover } else { hover.normal };
        if let Some((normal, hovered_text)) = hover.text {
            let want = if hovered { hovered_text } else { normal };
            for child in children.iter_descendants(entity) {
                if let Ok(mut color) = texts.get_mut(child) {
                    color.0 = want;
                }
            }
        }
    }
}

// ---- page furniture ---------------------------------------------------------

/// A page's heading, CoD style: a small grey breadcrumb over a big
/// condensed title.
pub fn page_title(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    breadcrumb: &str,
    title: &str,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|h| {
            h.spawn(label_hud(asset_server, breadcrumb, 16.0, TEXT_DIM));
            h.spawn(label_hud(asset_server, title, 56.0, TEXT));
        });
}

/// A small section heading with a hairline running off to its right.
pub fn section_heading(parent: &mut ChildSpawnerCommands, asset_server: &AssetServer, text: &str) {
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px(12.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn(label_hud(asset_server, text, 17.0, TEXT_DIM));
            row.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(EDGE),
            ));
        });
}

/// A full-width hairline.
pub fn divider() -> impl Bundle {
    (
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(1.0),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(EDGE),
    )
}

/// A flat square panel with a hairline edge.
pub fn panel_node(node: Node) -> impl Bundle {
    (
        Node {
            border: UiRect::all(Val::Px(1.0)),
            ..node
        },
        BackgroundColor(PANEL),
        BorderColor(EDGE),
    )
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

/// A text node set in [`crate::BODY_FONT`] — the plain readable face for
/// blocks of copy (e.g. the main-menu "What's New" notes), as opposed to
/// [`label_hud`]'s tall condensed caps.
pub fn label_body(
    asset_server: &AssetServer,
    text: impl Into<String>,
    size: f32,
    color: Color,
) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: asset_server.load(crate::BODY_FONT),
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
            border: UiRect::bottom(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(ROW),
        BorderColor(ACCENT),
    )
}

/// Spawn a labelled button carrying `marker` (any component — each screen uses
/// its own click-intent enum), its label set in [`crate::HUD_FONT`] (see
/// [`label_hud`]). `sfx` picks the hover/click sound pair (see [`UiSound`]);
/// the label flips to black when the hover fill is light (see [`Hoverable`]).
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
            Hoverable::new(normal, hover, text_color),
            ui_sound(sfx),
            Node {
                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(normal),
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
