//! Buying Pack-a-Punch (`shared::pap`): the prompt shown while standing at
//! the machine, and the Cold War style menu the interact key opens there —
//! the weapon in our hands, its three levels as tiles (owned / affordable /
//! too expensive — any level above ours can be bought straight away, paying
//! for every one on the way), the selected level's cost, and our points. Mouse to pick, click to buy, `Esc` (or EXIT) to back out.
//!
//! The menu is [`menu::Screen::PackAPunch`], so while it's up the cursor is
//! free and gameplay input is frozen like any other menu. A purchase plays
//! the buy sound, asks the server ([`shared::BuyPap`], which re-checks
//! everything and takes the points) and closes the menu, locking the cursor
//! again. Everything shown is read off the replicated lobby; the UI is
//! `StateScoped(InGame)` and the menu is closed whenever it no longer makes
//! sense (left the machine, died, game over), so nothing carries over.

use bevy::audio::Volume;
use bevy::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::pap::{PapWeapon, MAX_LEVEL};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::menu::{self, Screen};
use crate::net::GameClient;
use crate::pap::{my_pap_levels, PapSettings};
use crate::ui::{ui_sound, UiSfx, UiSound};
use crate::weapons::{Weapon, WeaponSlot, MAG_SIZE, PAP_EXTRA_MAGS_PER_LEVEL};
use crate::zombies_hud::{zombies_game, MONEY_YELLOW};
use crate::{killcam, AppState, GameSounds, Player, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) struct PapMenuPlugin;

impl Plugin for PapMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PapMenu>()
            .add_systems(OnEnter(AppState::InGame), spawn_pap_prompt)
            .add_systems(OnExit(AppState::InGame), close_on_exit)
            .add_systems(
                Update,
                (
                    update_pap_prompt,
                    open_pap_menu.run_if(menu::game_active.and(killcam::no_killcam)),
                    pap_menu_lifecycle,
                    pap_menu_input,
                    refresh_pap_menu,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The open menu's state (meaningless while it's closed).
#[derive(Resource)]
struct PapMenu {
    /// The weapon being packed — whichever was in our hands when it opened.
    weapon: PapWeapon,
    /// The level the header shows and PURCHASE buys: the one hovered last,
    /// starting on the next one to buy.
    selected: u8,
    /// Bought or EXIT clicked: close once the mouse button's up again, so the
    /// click can't carry on into a shot the moment gameplay input is back.
    close_pending: bool,
}

impl Default for PapMenu {
    fn default() -> Self {
        Self {
            weapon: PapWeapon::Sniper,
            selected: 1,
            close_pending: false,
        }
    }
}

/// Where a level stands for us.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LevelStatus {
    Owned,
    Buyable,
    TooPoor,
}

fn level_status(level: u8, current: u8, points: u32) -> LevelStatus {
    if level <= current {
        LevelStatus::Owned
    } else if points >= shared::pap::cost_to(current, level) {
        LevelStatus::Buyable
    } else {
        LevelStatus::TooPoor
    }
}

/// The weapon in our hands.
fn held_weapon(weapon: &Weapon) -> PapWeapon {
    match weapon.slot {
        WeaponSlot::Primary => PapWeapon::Sniper,
        WeaponSlot::Secondary => PapWeapon::Knife,
    }
}

/// What packing `weapon` to `level` does, for the menu's description line.
fn level_description(weapon: PapWeapon, level: u8) -> String {
    let damage = format!("{:.0}x Damage", shared::pap::damage_mult(level));
    match weapon {
        PapWeapon::Sniper => format!(
            "{damage}  |  +{} Max Ammo",
            MAG_SIZE * PAP_EXTRA_MAGS_PER_LEVEL * level as u32
        ),
        PapWeapon::Knife => damage,
    }
}

/// Our lobby, id, feet and points, if we're at the machine in a running
/// `Zombies` game on a map with one.
fn at_machine<'a>(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &'a Query<&Lobby>,
    settings: &PapSettings,
    player: &Transform,
) -> Option<(&'a Lobby, u32)> {
    let lobby = zombies_game(local, lobbies).filter(|l| shared::pap::machine_pos(l.map).is_some())?;
    let me = local.iter().next()?.0;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !settings.in_range(feet) {
        return None;
    }
    let points = lobby.members.iter().find(|m| m.peer == me)?.score;
    Some((lobby, points))
}

// --- the prompt at the machine ---------------------------------------------

#[derive(Component)]
struct PapPrompt;

#[derive(Component)]
struct PapPromptText;

const PROMPT_TITLE_BG: Color = Color::srgba(0.04, 0.04, 0.05, 0.92);
const PROMPT_LINE_BG: Color = Color::srgba(0.28, 0.05, 0.08, 0.9);

fn spawn_pap_prompt(mut commands: Commands, asset_server: Res<AssetServer>) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PapPrompt,
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(PROMPT_TITLE_BG),
            ))
            .with_child((
                Text::new("PACK-A-PUNCH"),
                TextFont {
                    font: heading,
                    font_size: 40.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(40.0), Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(PROMPT_LINE_BG),
            ))
            .with_child((
                PapPromptText,
                Text::new(""),
                TextFont {
                    font: body,
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.9, 0.9)),
            ));
        });
}

/// Show the prompt while we're at the machine (hidden behind menus, during a
/// kill cam and once dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_pap_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    settings: Res<PapSettings>,
    weapon: Res<Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut prompt: Single<&mut Visibility, With<PapPrompt>>,
    mut text: Single<&mut Text, With<PapPromptText>>,
) {
    let here = player
        .filter(|_| !menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .and_then(|p| at_machine(&local, &lobbies, &settings, &p));
    let Some((lobby, _)) = here else {
        prompt.set_if_neq(Visibility::Hidden);
        return;
    };
    prompt.set_if_neq(Visibility::Inherited);
    let held = held_weapon(&weapon);
    let line = if !shared::power::has_power(lobby.map, lobby.power_on) {
        "The power must be activated first".to_string()
    } else if my_pap_levels(&local, &lobbies).get(held) >= MAX_LEVEL {
        format!("Your {} is fully packed", held.label())
    } else {
        format!(
            "Press {} to pack your {}",
            binds.interact.label().to_uppercase(),
            held.label()
        )
    };
    if text.0 != line {
        text.0 = line;
    }
}

/// The interact key at the machine with the power on: open the menu on the
/// weapon in our hands.
#[allow(clippy::too_many_arguments)]
fn open_pap_menu(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    settings: Res<PapSettings>,
    weapon: Res<Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<PapMenu>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let Some(player) = player else {
        return;
    };
    let Some((lobby, _)) = at_machine(&local, &lobbies, &settings, &player) else {
        return;
    };
    if !shared::power::has_power(lobby.map, lobby.power_on) {
        return;
    }
    let held = held_weapon(&weapon);
    let current = my_pap_levels(&local, &lobbies).get(held);
    *state = PapMenu {
        weapon: held,
        selected: (current + 1).min(MAX_LEVEL),
        close_pending: false,
    };
    menu.screen = Screen::PackAPunch;
    menu.dirty = true;
}

// --- the menu ----------------------------------------------------------------

#[derive(Component)]
struct PapMenuRoot;

#[derive(Component, Clone, Copy)]
struct PapTile(u8);

#[derive(Component, Clone, Copy)]
struct PapTileNumeral(u8);

#[derive(Component, Clone, Copy)]
struct PapTileStatus(u8);

/// The corner brackets around the selected tile.
#[derive(Component, Clone, Copy)]
struct PapTileFrame(u8);

/// The padlock on the link into level `.0` — shown while the level before
/// it isn't ours yet.
#[derive(Component, Clone, Copy)]
struct PapLinkLock(u8);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PapText {
    Level,
    Cost,
    Description,
    Points,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PapButton {
    Purchase,
    Exit,
}

const PANEL_W: f32 = 860.0;
const TILE: f32 = 150.0;
const HEADER_BG: Color = Color::srgb(0.87, 0.85, 0.81);
const BODY_BG: Color = Color::srgba(0.36, 0.33, 0.29, 0.96);
const TAB_BAR_BG: Color = Color::srgba(0.05, 0.07, 0.1, 0.93);
const LEVEL_RED: Color = Color::srgb(0.72, 0.1, 0.12);
const DARK_TEXT: Color = Color::srgb(0.1, 0.1, 0.1);
const LIGHT_TEXT: Color = Color::srgb(0.92, 0.9, 0.86);
const OWNED_GREEN: Color = Color::srgb(0.15, 0.5, 0.22);
const POOR_RED: Color = Color::srgb(0.9, 0.22, 0.2);

/// A key chip ("LMB", "ESC") and its label — the footer's controls.
fn spawn_control(
    parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    which: PapButton,
    key: &str,
    label: &str,
    heading: &Handle<Font>,
) {
    let sound = match which {
        PapButton::Purchase => None,
        PapButton::Exit => Some(ui_sound(UiSound::BUTTON_BACK)),
    };
    let mut button = parent.spawn((
        which,
        Button,
        Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(12.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        BorderRadius::all(Val::Px(6.0)),
    ));
    if let Some(sound) = sound {
        button.insert(sound);
    }
    button.with_children(|b| {
        b.spawn((
            Node {
                min_width: Val::Px(44.0),
                height: Val::Px(36.0),
                padding: UiRect::horizontal(Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BorderColor(LIGHT_TEXT),
            BorderRadius::all(Val::Px(18.0)),
        ))
        .with_child((
            Text::new(key),
            TextFont {
                font: heading.clone(),
                font_size: 22.0,
                ..default()
            },
            TextColor(LIGHT_TEXT),
        ));
        b.spawn((
            Text::new(label),
            TextFont {
                font: heading.clone(),
                font_size: 34.0,
                ..default()
            },
            TextColor(LIGHT_TEXT),
        ));
    });
}

/// A padlock in a white circle, on the link between two tiles.
fn spawn_link(parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands, into: u8) {
    parent
        .spawn(Node {
            width: Val::Px(90.0),
            height: Val::Px(TILE),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|link| {
            // The line across.
            link.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    height: Val::Px(4.0),
                    ..default()
                },
                BackgroundColor(Color::WHITE),
            ));
            link.spawn((
                PapLinkLock(into),
                Node {
                    width: Val::Px(38.0),
                    height: Val::Px(38.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                BorderRadius::all(Val::Percent(50.0)),
            ))
            .with_children(|lock| {
                // Shackle, then body.
                lock.spawn((
                    Node {
                        width: Val::Px(12.0),
                        height: Val::Px(8.0),
                        border: UiRect {
                            left: Val::Px(3.0),
                            right: Val::Px(3.0),
                            top: Val::Px(3.0),
                            bottom: Val::Px(0.0),
                        },
                        ..default()
                    },
                    BorderColor(DARK_TEXT),
                    BorderRadius::top(Val::Px(6.0)),
                ));
                lock.spawn((
                    Node {
                        width: Val::Px(18.0),
                        height: Val::Px(12.0),
                        ..default()
                    },
                    BackgroundColor(DARK_TEXT),
                    BorderRadius::all(Val::Px(2.0)),
                ));
            });
        });
}

/// One level's tile, with its corner brackets.
fn spawn_tile(parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands, level: u8, heading: &Handle<Font>) {
    parent
        .spawn((
            PapTile(level),
            Button,
            Node {
                width: Val::Px(TILE),
                height: Val::Px(TILE),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor(Color::NONE),
        ))
        .with_children(|tile| {
            tile.spawn((
                PapTileNumeral(level),
                Text::new(shared::pap::numeral(level)),
                TextFont {
                    font: heading.clone(),
                    font_size: 84.0,
                    ..default()
                },
                TextColor(DARK_TEXT),
            ));
            tile.spawn(Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(6.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_child((
                PapTileStatus(level),
                Text::new(""),
                TextFont {
                    font: heading.clone(),
                    font_size: 22.0,
                    ..default()
                },
                TextColor(DARK_TEXT),
            ));
            // Corner brackets, just outside the tile.
            tile.spawn((
                PapTileFrame(level),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(-12.0),
                    right: Val::Px(-12.0),
                    top: Val::Px(-12.0),
                    bottom: Val::Px(-12.0),
                    ..default()
                },
                Visibility::Hidden,
            ))
            .with_children(|frame| {
                let arm = Val::Px(3.0);
                let none = Val::Px(0.0);
                for (left, top) in [(true, true), (false, true), (true, false), (false, false)] {
                    frame.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(24.0),
                            height: Val::Px(24.0),
                            left: if left { Val::Px(0.0) } else { Val::Auto },
                            right: if left { Val::Auto } else { Val::Px(0.0) },
                            top: if top { Val::Px(0.0) } else { Val::Auto },
                            bottom: if top { Val::Auto } else { Val::Px(0.0) },
                            border: UiRect {
                                left: if left { arm } else { none },
                                right: if left { none } else { arm },
                                top: if top { arm } else { none },
                                bottom: if top { none } else { arm },
                            },
                            ..default()
                        },
                        BorderColor(LIGHT_TEXT),
                    ));
                }
            });
        });
}

fn spawn_pap_menu(commands: &mut Commands, asset_server: &AssetServer, weapon: PapWeapon) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    let hfont = |size: f32| TextFont {
        font: heading.clone(),
        font_size: size,
        ..default()
    };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PapMenuRoot,
            // Over the HUD, like the pause menu.
            GlobalZIndex(50),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.02, 0.06, 0.35)),
        ))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Px(PANEL_W),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                ..default()
            })
            .with_children(|panel| {
                // Tab bar: just the one tab, named for the weapon.
                panel
                    .spawn((
                        Node {
                            margin: UiRect::horizontal(Val::Px(40.0)),
                            padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(TAB_BAR_BG),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            Node {
                                padding: UiRect::axes(Val::Px(56.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.85, 0.85, 0.85)),
                            BorderRadius::all(Val::Px(3.0)),
                        ))
                        .with_child((
                            Text::new(format!("PACK {}", weapon.label().to_uppercase())),
                            hfont(38.0),
                            TextColor(DARK_TEXT),
                        ));
                    });
                // Header: the selected level, and its cost.
                panel
                    .spawn((
                        Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(24.0), Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(HEADER_BG),
                    ))
                    .with_children(|h| {
                        h.spawn((PapText::Level, Text::new(""), hfont(64.0), TextColor(LEVEL_RED)));
                        h.spawn((PapText::Cost, Text::new(""), hfont(44.0), TextColor(DARK_TEXT)));
                    });
                // Body: what it does, the tiles, the controls and our points.
                panel
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::new(Val::Px(24.0), Val::Px(24.0), Val::Px(14.0), Val::Px(16.0)),
                            row_gap: Val::Px(26.0),
                            ..default()
                        },
                        BackgroundColor(BODY_BG),
                    ))
                    .with_children(|b| {
                        b.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|d| {
                            d.spawn((
                                PapText::Description,
                                Text::new(""),
                                TextFont {
                                    font: body.clone(),
                                    font_size: 30.0,
                                    ..default()
                                },
                                TextColor(LIGHT_TEXT),
                            ));
                            d.spawn((
                                Node {
                                    height: Val::Px(2.0),
                                    ..default()
                                },
                                BackgroundColor(LIGHT_TEXT.with_alpha(0.35)),
                            ));
                        });
                        b.spawn(Node {
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            padding: UiRect::vertical(Val::Px(12.0)),
                            ..default()
                        })
                        .with_children(|row| {
                            for level in 1..=MAX_LEVEL {
                                if level > 1 {
                                    spawn_link(row, level);
                                }
                                spawn_tile(row, level, &heading);
                            }
                        });
                        b.spawn(Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|f| {
                            f.spawn(Node {
                                column_gap: Val::Px(36.0),
                                align_items: AlignItems::Center,
                                ..default()
                            })
                            .with_children(|c| {
                                spawn_control(c, PapButton::Purchase, "LMB", "PURCHASE", &heading);
                                spawn_control(c, PapButton::Exit, "ESC", "EXIT", &heading);
                            });
                            f.spawn((PapText::Points, Text::new(""), hfont(48.0), TextColor(MONEY_YELLOW)));
                        });
                    });
            });
        });
}

/// Build the menu when it opens and take it down when it closes; close it
/// once a purchase / EXIT click is released, or as soon as it no longer
/// makes sense (away from the machine, dead, out of the game).
#[allow(clippy::too_many_arguments)]
fn pap_menu_lifecycle(
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<PapMenu>,
    mouse: Res<ButtonInput<MouseButton>>,
    death: Res<crate::death_effect::DeathEffect>,
    settings: Res<PapSettings>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    roots: Query<Entity, With<PapMenuRoot>>,
    mut commands: Commands,
) {
    let open = menu.screen == Screen::PackAPunch;
    if open {
        let still_here = !death.is_active()
            && player.is_some_and(|p| at_machine(&local, &lobbies, &settings, &p).is_some());
        let released = state.close_pending && mouse.get_pressed().next().is_none();
        if !still_here || released {
            menu.screen = Screen::None;
            menu.dirty = true;
            state.close_pending = false;
        }
    }
    let show = menu.screen == Screen::PackAPunch && !state.close_pending;
    match (show, roots.is_empty()) {
        (true, true) => spawn_pap_menu(&mut commands, &asset_server, state.weapon),
        (false, false) => {
            for e in &roots {
                commands.entity(e).despawn();
            }
        }
        _ => {}
    }
}

/// Hovering a tile selects it; clicking one (or PURCHASE, for the selected
/// one) buys it if we can — the buy sound, the request, and the menu closes —
/// or plays the denied sound if we can't. EXIT closes it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pap_menu_input(
    menu: Res<menu::Menu>,
    mut state: ResMut<PapMenu>,
    sounds: Option<Res<GameSounds>>,
    sfx: Option<Res<UiSfx>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut moved: EventReader<CursorMoved>,
    tiles: Query<(&PapTile, &Interaction)>,
    pressed_tiles: Query<(&PapTile, &Interaction), Changed<Interaction>>,
    buttons: Query<(&PapButton, &Interaction), Changed<Interaction>>,
    mut sender: Query<&mut TriggerSender<shared::BuyPap>, With<GameClient>>,
    mut commands: Commands,
) {
    let cursor_moved = moved.read().count() > 0;
    if menu.screen != Screen::PackAPunch || state.close_pending {
        return;
    }
    // Only when the mouse moves: the freed cursor starts in the middle of
    // the screen, right over a tile, and that shouldn't pick it.
    if cursor_moved {
        let hovered = tiles
            .iter()
            .find(|(_, i)| **i != Interaction::None)
            .map(|(t, _)| t.0);
        if let Some(level) = hovered.filter(|l| *l != state.selected) {
            state.selected = level;
            if let Some(sfx) = &sfx {
                commands.spawn((
                    AudioPlayer::new(sfx.button_hover.clone()),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.6)),
                ));
            }
        }
    }
    let mut buy: Option<u8> = None;
    for (tile, interaction) in &pressed_tiles {
        if *interaction == Interaction::Pressed {
            buy = Some(tile.0);
        }
    }
    for (button, interaction) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            PapButton::Purchase => buy = Some(state.selected),
            PapButton::Exit => state.close_pending = true,
        }
    }
    let Some(level) = buy else {
        return;
    };

    let me = local.iter().next().map(|l| l.0);
    let points = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or(0, |m| m.score);
    let current = my_pap_levels(&local, &lobbies).get(state.weapon);
    let buyable = level_status(level, current, points) == LevelStatus::Buyable;
    let sent = buyable
        && sender.single_mut().is_ok_and(|mut s| {
            s.trigger::<shared::LobbyChannel>(shared::BuyPap {
                weapon: state.weapon,
                level,
            });
            true
        });
    if sent {
        info!("asked the server to pack our {} to level {level}", state.weapon.label());
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.pap_buy.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
        state.close_pending = true;
    } else if let Some(sfx) = &sfx {
        commands.spawn((AudioPlayer::new(sfx.denied.clone()), PlaybackSettings::DESPAWN));
    }
}

/// Keep the open menu's tiles, header and points matching where we stand.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn refresh_pap_menu(
    menu: Res<menu::Menu>,
    state: Res<PapMenu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut tiles: Query<(&PapTile, &Interaction, &mut BackgroundColor, &mut BorderColor), Without<PapButton>>,
    mut numerals: Query<(&PapTileNumeral, &mut TextColor), (Without<PapTileStatus>, Without<PapText>)>,
    mut statuses: Query<(&PapTileStatus, &mut Text, &mut TextColor), (Without<PapTileNumeral>, Without<PapText>)>,
    mut frames: Query<(&PapTileFrame, &mut Visibility), Without<PapLinkLock>>,
    mut locks: Query<(&PapLinkLock, &mut Visibility), Without<PapTileFrame>>,
    mut texts: Query<(&PapText, &mut Text, &mut TextColor), (Without<PapTileNumeral>, Without<PapTileStatus>)>,
    mut buttons: Query<(&PapButton, &Interaction, &mut BackgroundColor), Without<PapTile>>,
) {
    if menu.screen != Screen::PackAPunch {
        return;
    }
    let me = local.iter().next().map(|l| l.0);
    let points = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or(0, |m| m.score);
    let current = my_pap_levels(&local, &lobbies).get(state.weapon);
    let status = |level: u8| level_status(level, current, points);

    for (tile, interaction, mut bg, mut border) in &mut tiles {
        let (fill, edge) = match status(tile.0) {
            LevelStatus::Owned => (Color::srgb(0.86, 0.88, 0.9), Color::srgb(0.86, 0.88, 0.9)),
            LevelStatus::Buyable => (Color::srgb(0.58, 0.56, 0.52), LIGHT_TEXT),
            LevelStatus::TooPoor => (Color::srgb(0.42, 0.4, 0.37), Color::NONE),
        };
        // A little lighter under the mouse.
        let fill = if *interaction == Interaction::None {
            fill
        } else {
            fill.mix(&Color::WHITE, 0.12)
        };
        bg.set_if_neq(BackgroundColor(fill));
        border.set_if_neq(BorderColor(edge));
    }
    for (n, mut color) in &mut numerals {
        let c = match status(n.0) {
            LevelStatus::Owned => Color::srgb(0.12, 0.25, 0.58),
            LevelStatus::Buyable => Color::srgb(0.06, 0.06, 0.06),
            LevelStatus::TooPoor => Color::srgba(0.0, 0.0, 0.0, 0.45),
        };
        color.set_if_neq(TextColor(c));
    }
    for (s, mut text, mut color) in &mut statuses {
        let cost = format!("${}", shared::pap::cost_to(current, s.0));
        let (line, c) = match status(s.0) {
            LevelStatus::Owned => ("OWNED".to_string(), OWNED_GREEN),
            LevelStatus::Buyable => (cost, MONEY_YELLOW),
            LevelStatus::TooPoor => (cost, POOR_RED),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }
    for (f, mut vis) in &mut frames {
        vis.set_if_neq(if f.0 == state.selected {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for (l, mut vis) in &mut locks {
        vis.set_if_neq(if l.0 > current + 1 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }

    let selected = status(state.selected);
    for (which, mut text, mut color) in &mut texts {
        let (line, c) = match which {
            PapText::Level => (format!("LEVEL {}", shared::pap::numeral(state.selected)), LEVEL_RED),
            PapText::Cost => {
                let cost = format!("Cost: ${}", shared::pap::cost_to(current, state.selected));
                match selected {
                    LevelStatus::Owned => ("OWNED".to_string(), OWNED_GREEN),
                    LevelStatus::TooPoor => (cost, POOR_RED),
                    LevelStatus::Buyable => (cost, DARK_TEXT),
                }
            }
            PapText::Description => (level_description(state.weapon, state.selected), LIGHT_TEXT),
            PapText::Points => (format!("${points}"), MONEY_YELLOW),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }
    for (button, interaction, mut bg) in &mut buttons {
        // PURCHASE only lights up when there's something to buy.
        let live = *button == PapButton::Exit || selected == LevelStatus::Buyable;
        let c = if live && *interaction != Interaction::None {
            Color::srgba(1.0, 1.0, 1.0, 0.12)
        } else {
            Color::NONE
        };
        bg.set_if_neq(BackgroundColor(c));
    }
}

/// Leaving the game with the menu somehow still up: close it.
fn close_on_exit(mut menu: ResMut<menu::Menu>, mut state: ResMut<PapMenu>) {
    if menu.screen == Screen::PackAPunch {
        menu.screen = Screen::None;
        menu.dirty = true;
    }
    *state = PapMenu::default();
}
