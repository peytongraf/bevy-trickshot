//! The pause / settings menu and the first-run username screen, built with
//! `bevy_ui`.
//!
//! * `Esc` opens/closes the settings menu (or the username screen if no name is
//!   set yet). While either is open the game is frozen and the cursor is free.
//! * Categories: **Profile** (username), **Controls** (sensitivity, FOV, debug
//!   toggle), **Keybinds** (rebind any action).
//! * `debug_mode` gates the existing egui tuning panels (`ads_tuning_ui`).
//!
//! The UI is rebuilt from scratch whenever `menu.dirty` is set (tab / screen /
//! rebind changes). Live values (slider fills, the FOV number, the username
//! field) are patched in place by [`refresh_dynamic`] so a slider drag doesn't
//! churn the whole tree.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;

use crate::keybinds::{Binding, KeyBindings, SLOTS};
use crate::settings::{Settings, FOV_MAX, FOV_MIN, SENS_MAX, SENS_MIN};

// ---- palette ---------------------------------------------------------------
const BACKDROP: Color = Color::srgba(0.03, 0.04, 0.055, 0.82);
const PANEL: Color = Color::srgb(0.072, 0.083, 0.10);
const PANEL_SOLID: Color = Color::srgb(0.03, 0.035, 0.05);
const ROW: Color = Color::srgb(0.11, 0.125, 0.150);
const ROW_HOVER: Color = Color::srgb(0.17, 0.19, 0.23);
const TRACK: Color = Color::srgb(0.16, 0.175, 0.205);
const ACCENT: Color = Color::srgb(0.96, 0.62, 0.12);
const ACCENT_DIM: Color = Color::srgb(0.42, 0.30, 0.10);
const TEXT: Color = Color::srgb(0.90, 0.92, 0.94);
const TEXT_DIM: Color = Color::srgb(0.55, 0.58, 0.63);

// A real font can be dropped in later: load `assets/fonts/<x>.ttf` and thread a
// `Handle<Font>` into `TextFont`. Until then this uses Bevy's embedded default.

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Screen {
    None,
    Username,
    Settings,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tab {
    Profile,
    Controls,
    Keybinds,
}

#[derive(Resource)]
pub struct Menu {
    pub screen: Screen,
    pub tab: Tab,
    /// `Some(i)` while waiting to capture a new binding for `SLOTS[i]`.
    pub rebinding: Option<usize>,
    /// Set true one frame after a rebind starts, so the click that started it
    /// isn't captured as the new binding.
    rebind_armed: bool,
    pub username_draft: String,
    /// Request a full UI rebuild next frame.
    dirty: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            screen: Screen::None,
            tab: Tab::Profile,
            rebinding: None,
            rebind_armed: false,
            username_draft: String::new(),
            dirty: true,
        }
    }
}

impl Menu {
    pub fn is_open(&self) -> bool {
        self.screen != Screen::None
    }
}

/// Run condition: gameplay systems only tick while no menu is up.
pub fn game_active(menu: Res<Menu>) -> bool {
    !menu.is_open()
}

/// Run condition for the egui dev panels.
pub fn debug_enabled(settings: Res<Settings>) -> bool {
    settings.debug_mode
}

/// Marker for HUD elements (crosshair / ammo / fps) so they hide behind a menu.
#[derive(Component)]
pub struct HudElement;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Menu>()
            .add_systems(Startup, startup_menu.after(crate::grab_cursor))
            .add_systems(
                Update,
                (
                    (menu_toggle, rebind_capture).chain(),
                    username_input,
                    menu_click,
                    menu_hover,
                    slider_drag,
                    refresh_dynamic,
                    rebuild_menu,
                    cursor_and_hud,
                )
                    .chain(),
            );
    }
}

fn startup_menu(mut menu: ResMut<Menu>, settings: Res<Settings>) {
    if !settings.has_username() {
        menu.screen = Screen::Username;
        menu.username_draft.clear();
        menu.dirty = true;
    }
}

// ---- input ---------------------------------------------------------------

fn menu_toggle(keys: Res<ButtonInput<KeyCode>>, mut menu: ResMut<Menu>, settings: Res<Settings>) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if menu.rebinding.is_some() {
        menu.rebinding = None;
        menu.dirty = true;
        return;
    }
    let has_name = settings.has_username();
    match menu.screen {
        Screen::None => {
            menu.screen = if has_name { Screen::Settings } else { Screen::Username };
            menu.tab = Tab::Profile;
            menu.username_draft = settings.username.clone().unwrap_or_default();
            menu.dirty = true;
        }
        Screen::Settings => {
            menu.screen = if has_name { Screen::None } else { Screen::Username };
            menu.dirty = true;
        }
        Screen::Username => {
            if has_name {
                menu.screen = Screen::None;
                menu.dirty = true;
            }
        }
    }
}

fn rebind_capture(
    mut menu: ResMut<Menu>,
    mut binds: ResMut<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    let Some(slot) = menu.rebinding else {
        return;
    };
    if !menu.rebind_armed {
        menu.rebind_armed = true;
        return;
    }
    if let Some(&code) = keys.get_just_pressed().next() {
        if code != KeyCode::Escape {
            binds.set_slot(slot, Binding::Key(code));
        }
        menu.rebinding = None;
        menu.dirty = true;
        return;
    }
    if let Some(&btn) = mouse.get_just_pressed().next() {
        binds.set_slot(slot, Binding::Mouse(btn));
        menu.rebinding = None;
        menu.dirty = true;
    }
}

fn username_input(
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut events: EventReader<KeyboardInput>,
) {
    let editing = matches!(menu.screen, Screen::Username)
        || (menu.screen == Screen::Settings && menu.tab == Tab::Profile);
    if !editing {
        events.clear();
        return;
    }
    for ev in events.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        match &ev.logical_key {
            Key::Character(s) => {
                for ch in s.chars() {
                    if !ch.is_control() && menu.username_draft.chars().count() < 20 {
                        menu.username_draft.push(ch);
                    }
                }
            }
            Key::Space => {
                if menu.username_draft.chars().count() < 20 {
                    menu.username_draft.push(' ');
                }
            }
            Key::Backspace => {
                menu.username_draft.pop();
            }
            Key::Enter => confirm_username(&mut menu, &mut settings),
            _ => {}
        }
    }
}

fn confirm_username(menu: &mut Menu, settings: &mut Settings) {
    let name = menu.username_draft.trim().to_string();
    if name.is_empty() {
        return;
    }
    settings.username = Some(name);
    if menu.screen == Screen::Username {
        menu.screen = Screen::None;
    }
    menu.dirty = true;
}

// ---- button / slider interaction ---------------------------------------

#[derive(Component, Clone)]
enum Btn {
    SelectTab(Tab),
    ToggleDebug,
    Rebind(usize),
    ResetKeybinds,
    Step(SliderField, f32),
    ConfirmUsername,
}

#[derive(Component, Clone, Copy, PartialEq)]
enum SliderField {
    Sensitivity,
    Fov,
}

#[derive(Component)]
struct Hoverable {
    normal: Color,
    hover: Color,
}

#[derive(Component)]
struct SliderTrack(SliderField);
#[derive(Component)]
struct SliderFill(SliderField);

/// Text nodes patched in place by [`refresh_dynamic`].
#[derive(Component)]
enum DynText {
    SliderValue(SliderField),
    Debug,
    Username,
}

fn menu_click(
    q: Query<(&Interaction, &Btn), Changed<Interaction>>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut binds: ResMut<KeyBindings>,
) {
    for (interaction, btn) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn {
            Btn::SelectTab(tab) => {
                menu.tab = *tab;
                menu.rebinding = None;
                if *tab == Tab::Profile {
                    menu.username_draft = settings.username.clone().unwrap_or_default();
                }
                menu.dirty = true;
            }
            Btn::ToggleDebug => settings.debug_mode = !settings.debug_mode,
            Btn::Rebind(i) => {
                menu.rebinding = Some(*i);
                menu.rebind_armed = false;
                menu.dirty = true;
            }
            Btn::ResetKeybinds => *binds = KeyBindings::default(),
            Btn::Step(field, delta) => step_field(&mut settings, *field, *delta),
            Btn::ConfirmUsername => confirm_username(&mut menu, &mut settings),
        }
    }
}

fn step_field(settings: &mut Settings, field: SliderField, delta: f32) {
    match field {
        SliderField::Sensitivity => {
            settings.sensitivity = (settings.sensitivity + delta).clamp(SENS_MIN, SENS_MAX);
        }
        SliderField::Fov => {
            settings.fov = (settings.fov + delta).clamp(FOV_MIN, FOV_MAX).round();
        }
    }
}

fn menu_hover(
    mut q: Query<(&Interaction, &Hoverable, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, hover, mut bg) in &mut q {
        bg.0 = match interaction {
            Interaction::Hovered | Interaction::Pressed => hover.hover,
            Interaction::None => hover.normal,
        };
    }
}

fn slider_drag(
    tracks: Query<(&Interaction, &SliderTrack, &RelativeCursorPosition)>,
    mut settings: ResMut<Settings>,
) {
    for (interaction, track, rel) in &tracks {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(pos) = rel.normalized else { continue };
        let t = pos.x.clamp(0.0, 1.0);
        match track.0 {
            SliderField::Sensitivity => {
                settings.sensitivity = (SENS_MIN + t * (SENS_MAX - SENS_MIN))
                    .clamp(SENS_MIN, SENS_MAX);
            }
            SliderField::Fov => {
                settings.fov = (FOV_MIN + t * (FOV_MAX - FOV_MIN)).round().clamp(FOV_MIN, FOV_MAX);
            }
        }
    }
}

fn field_fraction(settings: &Settings, field: SliderField) -> f32 {
    match field {
        SliderField::Sensitivity => {
            (settings.sensitivity - SENS_MIN) / (SENS_MAX - SENS_MIN)
        }
        SliderField::Fov => (settings.fov - FOV_MIN) / (FOV_MAX - FOV_MIN),
    }
    .clamp(0.0, 1.0)
}

fn field_value_text(settings: &Settings, field: SliderField) -> String {
    match field {
        SliderField::Sensitivity => format!("{:.2}", settings.sensitivity),
        SliderField::Fov => format!("{:.0}", settings.fov),
    }
}

fn refresh_dynamic(
    settings: Res<Settings>,
    menu: Res<Menu>,
    mut fills: Query<(&SliderFill, &mut Node)>,
    mut texts: Query<(&DynText, &mut Text)>,
) {
    if !(settings.is_changed() || menu.is_changed()) {
        return;
    }
    for (fill, mut node) in &mut fills {
        node.width = Val::Percent(field_fraction(&settings, fill.0) * 100.0);
    }
    for (dynt, mut text) in &mut texts {
        let s = match dynt {
            DynText::SliderValue(f) => field_value_text(&settings, *f),
            DynText::Debug => if settings.debug_mode { "ON" } else { "OFF" }.to_string(),
            DynText::Username => {
                if menu.username_draft.is_empty() {
                    "_".to_string()
                } else {
                    format!("{}|", menu.username_draft)
                }
            }
        };
        *text = Text::new(s);
    }
}

fn cursor_and_hud(
    menu: Res<Menu>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut hud: Query<&mut Visibility, With<HudElement>>,
) {
    if !menu.is_changed() {
        return;
    }
    let open = menu.is_open();
    for mut window in &mut windows {
        crate::set_cursor_grabbed(&mut window, !open);
    }
    let vis = if open {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut v in &mut hud {
        *v = vis;
    }
}

// ---- rebuild -----------------------------------------------------------

#[derive(Component)]
struct MenuRoot;

fn rebuild_menu(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    binds: Res<KeyBindings>,
    existing: Query<Entity, With<MenuRoot>>,
) {
    if !menu.dirty {
        return;
    }
    menu.dirty = false;
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    match menu.screen {
        Screen::None => {}
        Screen::Username => build_username(&mut commands),
        Screen::Settings => build_settings(&mut commands, &menu, &settings, &binds),
    }
}

fn overlay_root(solid: bool) -> impl Bundle {
    (
        MenuRoot,
        GlobalZIndex(50),
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

fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

fn field_box(width: f32) -> impl Bundle {
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

fn spawn_button(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    size: f32,
    marker: Btn,
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

fn build_username(commands: &mut Commands) {
    commands.spawn(overlay_root(true)).with_children(|root| {
        root.spawn((
            Node {
                width: Val::Px(520.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(40.0)),
                row_gap: Val::Px(18.0),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderRadius::all(Val::Px(10.0)),
        ))
        .with_children(|card| {
            card.spawn(label("CHOOSE A USERNAME", 30.0, TEXT));
            card.spawn(label(
                "This is how other players will see you. You can change it later in Settings.",
                15.0,
                TEXT_DIM,
            ));
            card.spawn(field_box(360.0)).with_children(|f| {
                f.spawn((label("", 22.0, TEXT), DynText::Username));
            });
            spawn_button(
                card,
                "CONFIRM",
                20.0,
                Btn::ConfirmUsername,
                ACCENT,
                ACCENT,
                PANEL_SOLID,
            );
        });
    });
}

fn build_settings(commands: &mut Commands, menu: &Menu, settings: &Settings, binds: &KeyBindings) {
    commands.spawn(overlay_root(false)).with_children(|root| {
        root.spawn((
            Node {
                width: Val::Px(900.0),
                height: Val::Px(580.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(PANEL),
            BorderRadius::all(Val::Px(8.0)),
        ))
        .with_children(|panel| {
            // header
            panel
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(28.0), Val::Px(20.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                    BorderColor(TRACK),
                ))
                .with_children(|h| {
                    h.spawn(label("SETTINGS", 28.0, TEXT));
                    h.spawn((
                        Node {
                            width: Val::Px(46.0),
                            height: Val::Px(3.0),
                            ..default()
                        },
                        BackgroundColor(ACCENT),
                    ));
                });

            // body: left categories + right content
            panel
                .spawn(Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    ..default()
                })
                .with_children(|body| {
                    body.spawn((
                        Node {
                            width: Val::Px(230.0),
                            height: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::all(Val::Px(14.0)),
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.055, 0.064, 0.08)),
                    ))
                    .with_children(|cats| {
                        for (tab, name) in [
                            (Tab::Profile, "PROFILE"),
                            (Tab::Controls, "CONTROLS"),
                            (Tab::Keybinds, "KEYBINDS"),
                        ] {
                            let selected = menu.tab == tab;
                            spawn_button(
                                cats,
                                name,
                                17.0,
                                Btn::SelectTab(tab),
                                if selected { ROW_HOVER } else { PANEL },
                                ROW_HOVER,
                                if selected { ACCENT } else { TEXT_DIM },
                            );
                        }
                    });

                    body.spawn(Node {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(30.0)),
                        row_gap: Val::Px(16.0),
                        overflow: Overflow::clip(),
                        ..default()
                    })
                    .with_children(|content| match menu.tab {
                        Tab::Profile => build_profile(content),
                        Tab::Controls => build_controls(content, settings),
                        Tab::Keybinds => build_keybinds(content, menu, binds),
                    });
                });

            // footer
            panel
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(28.0), Val::Px(14.0)),
                        column_gap: Val::Px(24.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.055, 0.064, 0.08)),
                ))
                .with_children(|f| {
                    f.spawn(label("[Esc] Close", 14.0, TEXT_DIM));
                    f.spawn(label("Changes save automatically", 14.0, TEXT_DIM));
                });
        });
    });
}

fn build_profile(content: &mut ChildSpawnerCommands) {
    content.spawn(label("USERNAME", 15.0, TEXT_DIM));
    content
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(14.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn(field_box(320.0)).with_children(|f| {
                f.spawn((label("", 22.0, TEXT), DynText::Username));
            });
            spawn_button(
                row,
                "APPLY",
                17.0,
                Btn::ConfirmUsername,
                ACCENT,
                ACCENT,
                PANEL_SOLID,
            );
        });
    content.spawn(label(
        "Start typing to edit. Enter or Apply to save.",
        14.0,
        TEXT_DIM,
    ));
}

fn build_controls(content: &mut ChildSpawnerCommands, settings: &Settings) {
    spawn_slider_row(content, "MOUSE SENSITIVITY", SliderField::Sensitivity, settings, 0.05);
    spawn_slider_row(content, "FIELD OF VIEW", SliderField::Fov, settings, 1.0);

    content
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(16.0),
            margin: UiRect::top(Val::Px(8.0)),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                label("DEBUG MODE", 15.0, TEXT_DIM),
                Node {
                    width: Val::Px(220.0),
                    ..default()
                },
            ));
            spawn_button(
                row,
                if settings.debug_mode { "ON" } else { "OFF" },
                17.0,
                Btn::ToggleDebug,
                ROW,
                ROW_HOVER,
                if settings.debug_mode { ACCENT } else { TEXT },
            );
            // keep the toggle label live without a rebuild
            row.spawn((
                label("", 0.001, Color::NONE),
                DynText::Debug,
                Node {
                    width: Val::Px(0.0),
                    ..default()
                },
            ));
        });
    content.spawn(label(
        "Debug mode shows the muzzle-flash / smoke / gravity tuning panels (top-right).",
        14.0,
        TEXT_DIM,
    ));
}

fn spawn_slider_row(
    content: &mut ChildSpawnerCommands,
    name: &str,
    field: SliderField,
    settings: &Settings,
    step: f32,
) {
    content
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(12.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                label(name, 15.0, TEXT_DIM),
                Node {
                    width: Val::Px(220.0),
                    ..default()
                },
            ));
            spawn_button(row, "-", 18.0, Btn::Step(field, -step), ROW, ROW_HOVER, TEXT);
            // track
            row.spawn((
                Button,
                Interaction::default(),
                SliderTrack(field),
                RelativeCursorPosition::default(),
                Node {
                    width: Val::Px(300.0),
                    height: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(TRACK),
                BorderRadius::all(Val::Px(5.0)),
            ))
            .with_children(|track| {
                track.spawn((
                    SliderFill(field),
                    Node {
                        width: Val::Percent(field_fraction(settings, field) * 100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                    BorderRadius::all(Val::Px(5.0)),
                ));
            });
            spawn_button(row, "+", 18.0, Btn::Step(field, step), ROW, ROW_HOVER, TEXT);
            row.spawn((
                label(field_value_text(settings, field), 18.0, TEXT),
                DynText::SliderValue(field),
                Node {
                    width: Val::Px(54.0),
                    ..default()
                },
            ));
        });
}

fn build_keybinds(content: &mut ChildSpawnerCommands, menu: &Menu, binds: &KeyBindings) {
    content.spawn(label(
        "Click a binding, then press a key or mouse button. Esc cancels.",
        14.0,
        TEXT_DIM,
    ));
    content
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|list| {
            for (i, (name, _)) in SLOTS.iter().enumerate() {
                list.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(if i % 2 == 0 { PANEL } else { ROW }),
                    BorderRadius::all(Val::Px(3.0)),
                ))
                .with_children(|row| {
                    row.spawn(label(*name, 15.0, TEXT));
                    let capturing = menu.rebinding == Some(i);
                    spawn_button(
                        row,
                        &if capturing {
                            "PRESS ANY KEY".to_string()
                        } else {
                            binds.slot(i).label()
                        },
                        15.0,
                        Btn::Rebind(i),
                        if capturing { ACCENT_DIM } else { TRACK },
                        ROW_HOVER,
                        if capturing { ACCENT } else { TEXT },
                    );
                });
            }
        });
    spawn_button(
        content,
        "RESET TO DEFAULTS",
        15.0,
        Btn::ResetKeybinds,
        ROW,
        ROW_HOVER,
        TEXT_DIM,
    );
}
