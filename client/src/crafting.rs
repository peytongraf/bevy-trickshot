//! The `Zombies` crafting table (`shared::crafting`), Cold War's: the table
//! (`models/props/crafting_table.glb`, where the map's layout puts it), the
//! prompt shown at it, and the menu the interact key opens there — a
//! TACTICAL and a LETHAL tab, the selected item's name and cost in the
//! header, what it does under that, a tile for each item the tab sells
//! (its icon, and how many we carry of the most we can), then EXIT and our
//! points. Hovering a tile shows it in the header; clicking it crafts
//! one — the pickup sound, and the server takes the points
//! ([`shared::BuyEquipment`]) — and the menu stays open for more. It shows
//! whether the picked item can be bought, is too expensive, or is already
//! carried in full. `Esc` / EXIT (or the mouse back button) leaves.
//!
//! The menu is [`menu::Screen::Crafting`], so while it's up the cursor is
//! free and gameplay input is frozen. Crafting a kind other than the one of
//! its sort we carry swaps to it (the old ones are dropped around us). The
//! count itself is ours (`Weapon`), added when the server answers
//! ([`shared::EquipmentBought`]).
//!
//! The table, prompt and menu are `StateScoped(InGame)`, the table's taken
//! away whenever we're not in a `Zombies` game, the menu closes whenever it
//! stops making sense (walked off, died, out of the game), and its state is
//! reset on opening and on leaving the game — nothing carries over.

use bevy::audio::Volume;
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use lightyear::prelude::{LocalId, MessageReceiver, TriggerSender};
use shared::crafting::{cost, items, Tab, HALF_EXTENTS};
use shared::lethal::LethalKind;
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::menu::{self, Screen};
use crate::net::GameClient;
use crate::ui::{ui_sound, UiSfx, UiSound};
use crate::zombies_hud::{zombies_game, MONEY_YELLOW};
use crate::{killcam, lethal_icon_path, AppState, GameSounds, Lethal, Player, Weapon, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) const CRAFTING_TABLE_MODEL: &str = "models/props/crafting_table.glb";

/// `crafting_table.glb` as made: its bounds (~2 m tall, its long side along
/// z, its middle a little off its origin).
const MODEL_MIN: Vec3 = Vec3::new(-0.398, 0.0, -1.333);
const MODEL_MAX: Vec3 = Vec3::new(0.301, 2.067, 1.16);

/// The model's front faces +X as made (its long side runs along z): a
/// quarter turn clockwise (seen from above) faces it +Z, the way a
/// placement's front points.
const MODEL_TURN_DEG: f32 = -90.0;

/// How long (s) a crafted item counts as on its way before we stop waiting
/// for the server's answer (a request it turned down never gets one).
const IN_FLIGHT_SECS: f32 = 2.0;

/// The table's model, from the ground under its middle: scaled to
/// [`HALF_EXTENTS`]' height, its middle over the origin, standing on the
/// ground, facing +Z.
pub(crate) fn model_transform() -> Transform {
    let size = MODEL_MAX - MODEL_MIN;
    let scale = HALF_EXTENTS.y * 2.0 / size.y;
    let middle = (MODEL_MIN + MODEL_MAX) * 0.5;
    let turn = Quat::from_rotation_y(MODEL_TURN_DEG.to_radians());
    Transform {
        translation: turn * (Vec3::new(-middle.x, -MODEL_MIN.y, -middle.z) * scale),
        rotation: turn,
        scale: Vec3::splat(scale),
    }
}

pub(crate) struct CraftingPlugin;

impl Plugin for CraftingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CraftingMenu>()
            .add_systems(OnEnter(AppState::InGame), spawn_crafting_prompt)
            .add_systems(OnExit(AppState::InGame), close_on_exit)
            .add_systems(
                Update,
                (
                    sync_crafting_table,
                    receive_crafted,
                    update_crafting_prompt,
                    open_crafting_menu.run_if(menu::game_active.and(killcam::no_killcam)),
                    crafting_menu_lifecycle,
                    crafting_menu_input,
                    refresh_crafting_menu,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

// --- the table -----------------------------------------------------------------

#[derive(Component)]
struct CraftingTable;

/// Put the table on the map while we're in a `Zombies` game on a map with
/// one (and take it away otherwise), where its layout says — solid, like the
/// server's `shared::crafting::solid_box`.
fn sync_crafting_table(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut tables: Query<(Entity, &mut Transform), With<CraftingTable>>,
    mut commands: Commands,
) {
    let Some(at) = zombies_game(&local, &lobbies).and_then(|l| shared::crafting::placement(l.map)) else {
        for (e, _) in &tables {
            commands.entity(e).despawn();
        }
        return;
    };
    if let Some((_, mut t)) = tables.iter_mut().next() {
        t.set_if_neq(crate::util::placed(at));
        return;
    }
    let half = HALF_EXTENTS;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            CraftingTable,
            crate::util::placed(at),
            Visibility::default(),
        ))
        .with_children(|t| {
            t.spawn((
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(CRAFTING_TABLE_MODEL))),
                model_transform(),
            ));
            t.spawn((
                bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(Vec3::Y * half.y),
            ));
        });
}

/// Our points, if we're at the table in a running `Zombies` game on a map
/// with one.
fn at_table(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>, player: &Transform) -> Option<u32> {
    let lobby = zombies_game(local, lobbies)?;
    let me = local.iter().next()?.0;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !shared::crafting::in_range(lobby.map, feet, 0.0) {
        return None;
    }
    lobby.members.iter().find(|m| m.peer == me).map(|m| m.score)
}

/// The server answered a craft: one more of it.
fn receive_crafted(
    mut receivers: Query<&mut MessageReceiver<shared::EquipmentBought>>,
    mut weapon: ResMut<Weapon>,
    mut state: ResMut<CraftingMenu>,
) {
    for mut rx in &mut receivers {
        for got in rx.receive() {
            let kind = Lethal::of(got.kind);
            weapon.add_one(kind);
            if let Some(i) = state.in_flight.iter().position(|(k, _)| *k == kind) {
                state.in_flight.remove(i);
            }
        }
    }
}

// --- the prompt at the table ---------------------------------------------------

#[derive(Component)]
struct CraftingPrompt;

#[derive(Component)]
struct CraftingPromptText;

const PROMPT_TITLE_BG: Color = Color::srgba(0.04, 0.04, 0.05, 0.92);
const PROMPT_LINE_BG: Color = Color::srgba(0.24, 0.17, 0.08, 0.9);

fn spawn_crafting_prompt(mut commands: Commands, asset_server: Res<AssetServer>) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            CraftingPrompt,
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
                Text::new("CRAFTING TABLE"),
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
                CraftingPromptText,
                Text::new(""),
                TextFont {
                    font: body,
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.92, 0.86)),
            ));
        });
}

/// Show the prompt while we're at the table (hidden behind menus, during a
/// kill cam and once dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_crafting_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut prompt: Single<&mut Visibility, With<CraftingPrompt>>,
    mut text: Single<&mut Text, With<CraftingPromptText>>,
) {
    let here = player
        .filter(|_| !menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .and_then(|p| at_table(&local, &lobbies, &p));
    if here.is_none() {
        prompt.set_if_neq(Visibility::Hidden);
        return;
    }
    prompt.set_if_neq(Visibility::Inherited);
    let line = format!("Press {} to craft equipment", binds.interact.label().to_uppercase());
    if text.0 != line {
        text.0 = line;
    }
}

/// The interact key at the table: open the menu on the lethal tab, its
/// first item picked.
#[allow(clippy::too_many_arguments)]
fn open_crafting_menu(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<CraftingMenu>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    if player.and_then(|p| at_table(&local, &lobbies, &p)).is_none() {
        return;
    }
    *state = CraftingMenu::default();
    menu.screen = Screen::Crafting;
    menu.dirty = true;
}

// --- the menu ------------------------------------------------------------------------

/// The open menu's state (meaningless while it's closed).
#[derive(Resource)]
struct CraftingMenu {
    tab: Tab,
    /// The item the header shows (the one hovered last).
    selected: LethalKind,
    /// EXIT clicked: close once the mouse button's up again, so the click
    /// can't carry on into a shot the moment gameplay input is back.
    close_pending: bool,
    /// Crafted but not answered yet, and when (`Time::elapsed_secs`) —
    /// counted as carried meanwhile, so quick clicks can't overbuy.
    in_flight: Vec<(Lethal, f32)>,
}

impl Default for CraftingMenu {
    fn default() -> Self {
        Self {
            tab: Tab::Lethal,
            selected: items(Tab::Lethal)[0],
            close_pending: false,
            in_flight: Vec::new(),
        }
    }
}

/// Where an item stands for us.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Buyable,
    TooPoor,
    /// Carrying as many as can be.
    Full,
}

/// How many of `kind` we carry, counting crafts still on their way.
fn carried(weapon: &Weapon, state: &CraftingMenu, kind: Lethal) -> u32 {
    let flying = state.in_flight.iter().filter(|(k, _)| *k == kind).count() as u32;
    (weapon.count_of(kind) + flying).min(kind.max_carried())
}

fn status(weapon: &Weapon, state: &CraftingMenu, kind: LethalKind, points: u32) -> Status {
    let k = Lethal::of(kind);
    if carried(weapon, state, k) >= k.max_carried() {
        Status::Full
    } else if points >= cost(kind).unwrap_or(u32::MAX) {
        Status::Buyable
    } else {
        Status::TooPoor
    }
}

fn label(kind: LethalKind) -> &'static str {
    match kind {
        LethalKind::Frag => "FRAG",
        LethalKind::Molotov => "MOLOTOV",
        LethalKind::FlashBang => "FLASH BANG",
        LethalKind::MonkeyBomb => "MONKEY BOMB",
        LethalKind::ThrowingKnife => "THROWING KNIFE",
    }
}

/// What it does (Cold War's lines), and what crafting it swaps out.
fn description(weapon: &Weapon, kind: LethalKind) -> String {
    let what = match kind {
        LethalKind::Frag => "Explodes after a short fuse. Can be cooked by holding.",
        LethalKind::Molotov => "Explodes on impact, spreading flames over a small area.",
        LethalKind::FlashBang => "Blinds and stuns nearby zombies for a short time.",
        LethalKind::MonkeyBomb => "Attracts normal zombies for a short duration before detonating.",
        LethalKind::ThrowingKnife => "Retrievable knife that kills normal zombies instantly on impact.",
    };
    let k = Lethal::of(kind);
    let other = if k.is_tactical() { weapon.carried_tactical() } else { weapon.carried_lethal() };
    match other.filter(|o| *o != k) {
        Some(o) => format!("{what} Replaces your {}.", label_of(o).to_lowercase()),
        None => what.to_string(),
    }
}

fn label_of(kind: Lethal) -> &'static str {
    kind.kind().map_or("", label)
}

#[derive(Component)]
struct CraftingMenuRoot;

#[derive(Component, Clone, Copy)]
struct CraftTab(Tab);

/// A tile, and its parts.
#[derive(Component, Clone, Copy)]
struct CraftTile(LethalKind);

#[derive(Component, Clone, Copy)]
struct CraftTileIcon(LethalKind);

#[derive(Component, Clone, Copy)]
struct CraftTileCount(LethalKind);

/// The corner brackets around the selected tile.
#[derive(Component, Clone, Copy)]
struct CraftTileFrame(LethalKind);

/// The tiles' row (rebuilt when the tab changes).
#[derive(Component)]
struct CraftTileRow;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum CraftText {
    Name,
    Cost,
    Description,
    Points,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum CraftButton {
    Exit,
}

const PANEL_W: f32 = 860.0;
const TILE: f32 = 130.0;
const HEADER_BG: Color = Color::srgb(0.87, 0.85, 0.81);
const BODY_BG: Color = Color::srgba(0.36, 0.33, 0.29, 0.96);
const TAB_BAR_BG: Color = Color::srgba(0.05, 0.07, 0.1, 0.93);
const NAME_RED: Color = Color::srgb(0.6, 0.08, 0.12);
const DARK_TEXT: Color = Color::srgb(0.1, 0.1, 0.1);
const LIGHT_TEXT: Color = Color::srgb(0.92, 0.9, 0.86);
const POOR_RED: Color = Color::srgb(0.85, 0.15, 0.2);
const FULL_GREY: Color = Color::srgb(0.38, 0.38, 0.38);

/// A key chip ("LMB", "ESC") and its label — the footer's controls.
fn spawn_control(parent: &mut ChildSpawnerCommands, which: CraftButton, key: &str, text: &str, heading: &Handle<Font>) {
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
    if which == CraftButton::Exit {
        button.insert(ui_sound(UiSound::BUTTON_BACK));
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
            Text::new(text),
            TextFont {
                font: heading.clone(),
                font_size: 34.0,
                ..default()
            },
            TextColor(LIGHT_TEXT),
        ));
    });
}

/// One tile: the item's icon, how many we carry of the most (bottom right),
/// and corner brackets when it's selected.
fn spawn_tile(parent: &mut ChildSpawnerCommands, kind: LethalKind, heading: &Handle<Font>, asset_server: &AssetServer) {
    parent
        .spawn((
            CraftTile(kind),
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
            // (The HUD icons, drawn to a 3:2 canvas.)
            tile.spawn((
                CraftTileIcon(kind),
                ImageNode::new(asset_server.load(lethal_icon_path(Lethal::of(kind)))),
                Node {
                    width: Val::Px(TILE * 0.9),
                    height: Val::Px(TILE * 0.6),
                    ..default()
                },
            ));
            tile.spawn(Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                bottom: Val::Px(2.0),
                ..default()
            })
            .with_child((
                CraftTileCount(kind),
                Text::new(""),
                TextFont {
                    font: heading.clone(),
                    font_size: 28.0,
                    ..default()
                },
                TextColor(DARK_TEXT),
                TextShadow {
                    offset: Vec2::splat(1.0),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.6),
                },
            ));
            tile.spawn((
                CraftTileFrame(kind),
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
                            width: Val::Px(22.0),
                            height: Val::Px(22.0),
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

fn spawn_tiles(row: &mut ChildSpawnerCommands, tab: Tab, heading: &Handle<Font>, asset_server: &AssetServer) {
    for &kind in items(tab) {
        spawn_tile(row, kind, heading, asset_server);
    }
}

fn spawn_crafting_menu(commands: &mut Commands, asset_server: &AssetServer, tab: Tab) {
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
            CraftingMenuRoot,
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
            BackgroundColor(Color::srgba(0.02, 0.01, 0.0, 0.35)),
        ))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Px(PANEL_W),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                ..default()
            })
            .with_children(|panel| {
                // The page's title over the tabs, Cold War style.
                panel
                    .spawn(Node {
                        justify_content: JustifyContent::Center,
                        margin: UiRect::bottom(Val::Px(14.0)),
                        ..default()
                    })
                    .with_child((
                        Text::new("CRAFTING TABLE"),
                        hfont(56.0),
                        TextColor(Color::WHITE),
                        TextShadow {
                            offset: Vec2::splat(2.0),
                            color: Color::srgba(0.0, 0.0, 0.0, 0.8),
                        },
                    ));
                // Tab bar: TACTICAL, LETHAL.
                panel
                    .spawn((
                        Node {
                            margin: UiRect::horizontal(Val::Px(40.0)),
                            padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                            justify_content: JustifyContent::Center,
                            column_gap: Val::Px(24.0),
                            ..default()
                        },
                        BackgroundColor(TAB_BAR_BG),
                    ))
                    .with_children(|bar| {
                        for (t, name) in [(Tab::Tactical, "TACTICAL"), (Tab::Lethal, "LETHAL")] {
                            bar.spawn((
                                CraftTab(t),
                                Button,
                                ui_sound(UiSound::BUTTON),
                                Node {
                                    padding: UiRect::axes(Val::Px(56.0), Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                                BorderRadius::all(Val::Px(3.0)),
                            ))
                            .with_child((Text::new(name), hfont(34.0), TextColor(LIGHT_TEXT)));
                        }
                    });
                // Header: the selected item, and its cost.
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
                        h.spawn((CraftText::Name, Text::new(""), hfont(64.0), TextColor(NAME_RED)));
                        h.spawn((CraftText::Cost, Text::new(""), hfont(44.0), TextColor(DARK_TEXT)));
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
                                CraftText::Description,
                                Text::new(""),
                                TextFont {
                                    font: body.clone(),
                                    font_size: 28.0,
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
                        b.spawn((
                            CraftTileRow,
                            Node {
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(28.0),
                                padding: UiRect::vertical(Val::Px(12.0)),
                                ..default()
                            },
                        ))
                        .with_children(|row| spawn_tiles(row, tab, &heading, asset_server));
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
                                spawn_control(c, CraftButton::Exit, "ESC", "EXIT", &heading);
                            });
                            f.spawn((CraftText::Points, Text::new(""), hfont(48.0), TextColor(MONEY_YELLOW)));
                        });
                    });
            });
        });
}

/// Build the menu when it opens and take it down when it closes; close it
/// once an EXIT click is released, or as soon as it no longer makes sense
/// (away from the table, dead, out of the game).
#[allow(clippy::too_many_arguments)]
fn crafting_menu_lifecycle(
    time: Res<Time>,
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<CraftingMenu>,
    mouse: Res<ButtonInput<MouseButton>>,
    death: Res<crate::death_effect::DeathEffect>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    roots: Query<Entity, With<CraftingMenuRoot>>,
    mut commands: Commands,
) {
    // (Crafts the server never answered stop counting.)
    let now = time.elapsed_secs();
    if state.in_flight.iter().any(|(_, at)| now - at > IN_FLIGHT_SECS) {
        state.in_flight.retain(|(_, at)| now - at <= IN_FLIGHT_SECS);
    }
    if menu.screen == Screen::Crafting {
        // The mouse back button leaves like EXIT does (once it's released).
        if mouse.just_pressed(MouseButton::Back) {
            state.close_pending = true;
        }
        let still_here = !death.is_active() && player.is_some_and(|p| at_table(&local, &lobbies, &p).is_some());
        let released = state.close_pending && mouse.get_pressed().next().is_none();
        if !still_here || released {
            menu.screen = Screen::None;
            menu.dirty = true;
            state.close_pending = false;
        }
    }
    let show = menu.screen == Screen::Crafting && !state.close_pending;
    match (show, roots.is_empty()) {
        (true, true) => spawn_crafting_menu(&mut commands, &asset_server, state.tab),
        (false, false) => {
            for e in &roots {
                commands.entity(e).despawn();
            }
        }
        _ => {}
    }
}

/// Clicking a tab switches to it; hovering a tile picks it (the header
/// shows it); clicking one crafts it if we can — the pickup sound and the
/// request, the menu staying open — or plays the denied sound if we can't.
/// EXIT closes it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn crafting_menu_input(
    time: Res<Time>,
    menu: Res<menu::Menu>,
    mut state: ResMut<CraftingMenu>,
    weapon: Res<Weapon>,
    (sounds, sfx): (Option<Res<GameSounds>>, Option<Res<UiSfx>>),
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut moved: EventReader<CursorMoved>,
    tiles: Query<(&CraftTile, &Interaction)>,
    pressed_tiles: Query<(&CraftTile, &Interaction), Changed<Interaction>>,
    tabs: Query<(&CraftTab, &Interaction), Changed<Interaction>>,
    buttons: Query<(&CraftButton, &Interaction), Changed<Interaction>>,
    row: Query<Entity, With<CraftTileRow>>,
    mut sender: Query<&mut TriggerSender<shared::BuyEquipment>, With<GameClient>>,
    mut commands: Commands,
) {
    let cursor_moved = moved.read().count() > 0;
    if menu.screen != Screen::Crafting || state.close_pending {
        return;
    }
    let hover_sound = |commands: &mut Commands| {
        if let Some(sfx) = &sfx {
            commands.spawn((
                AudioPlayer::new(sfx.button_hover.clone()),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.6)),
            ));
        }
    };
    // A tab: its tiles in place of the last one's, its first item picked.
    for (tab, interaction) in &tabs {
        if *interaction == Interaction::Pressed && tab.0 != state.tab {
            state.tab = tab.0;
            state.selected = items(tab.0)[0];
            if let Ok(row) = row.single() {
                commands.entity(row).despawn_related::<Children>();
                let heading = asset_server.load(HUD_FONT);
                commands
                    .entity(row)
                    .with_children(|r| spawn_tiles(r, tab.0, &heading, &asset_server));
            }
            return;
        }
    }
    // Only when the mouse moves: the freed cursor starts in the middle of
    // the screen, right over a tile, and that shouldn't pick it.
    if cursor_moved {
        let hovered = tiles.iter().find(|(_, i)| **i != Interaction::None).map(|(t, _)| t.0);
        if let Some(kind) = hovered.filter(|k| *k != state.selected) {
            state.selected = kind;
            hover_sound(&mut commands);
        }
    }
    // Clicking a tile crafts it.
    let mut buy = None;
    for (tile, interaction) in &pressed_tiles {
        if *interaction == Interaction::Pressed {
            state.selected = tile.0;
            buy = Some(tile.0);
        }
    }
    for (button, interaction) in &buttons {
        if *interaction == Interaction::Pressed && *button == CraftButton::Exit {
            state.close_pending = true;
        }
    }
    let Some(kind) = buy else {
        return;
    };
    let me = local.iter().next().map(|l| l.0);
    let points = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or(0, |m| m.score);
    let k = Lethal::of(kind);
    let sent = status(&weapon, &state, kind, points) == Status::Buyable
        && sender.single_mut().is_ok_and(|mut s| {
            s.trigger::<shared::LobbyChannel>(shared::BuyEquipment {
                kind,
                dropping: weapon.carried_other_than(k),
            });
            true
        });
    if sent {
        info!("asked the server to craft a {kind:?}");
        state.in_flight.push((k, time.elapsed_secs()));
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.pick_up_equipment.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    } else if let Some(sfx) = &sfx {
        commands.spawn((AudioPlayer::new(sfx.denied.clone()), PlaybackSettings::DESPAWN));
    }
}

/// Keep the open menu's tabs, tiles, header and points matching
/// where we stand.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn refresh_crafting_menu(
    menu: Res<menu::Menu>,
    state: Res<CraftingMenu>,
    weapon: Res<Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut tabs: Query<(&CraftTab, &Interaction, &mut BackgroundColor, &Children), (Without<CraftTile>, Without<CraftButton>)>,
    mut tiles: Query<(&CraftTile, &Interaction, &mut BackgroundColor, &mut BorderColor), (Without<CraftButton>, Without<CraftTab>)>,
    mut icons: Query<(&CraftTileIcon, &mut ImageNode)>,
    mut counts: Query<(&CraftTileCount, &mut Text, &mut TextColor), Without<CraftText>>,
    mut frames: Query<(&CraftTileFrame, &mut Visibility)>,
    mut texts: Query<(&CraftText, &mut Text, &mut TextColor), Without<CraftTileCount>>,
    mut tab_texts: Query<&mut TextColor, (Without<CraftText>, Without<CraftTileCount>)>,
    mut buttons: Query<(&CraftButton, &Interaction, &mut BackgroundColor), (Without<CraftTile>, Without<CraftTab>)>,
) {
    if menu.screen != Screen::Crafting {
        return;
    }
    let me = local.iter().next().map(|l| l.0);
    let points = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or(0, |m| m.score);
    let status_of = |k: LethalKind| status(&weapon, &state, k, points);

    for (tab, interaction, mut bg, children) in &mut tabs {
        let on = tab.0 == state.tab;
        let fill = if on {
            Color::srgb(0.85, 0.85, 0.85)
        } else if *interaction != Interaction::None {
            Color::srgba(1.0, 1.0, 1.0, 0.12)
        } else {
            Color::NONE
        };
        bg.set_if_neq(BackgroundColor(fill));
        for child in children.iter() {
            if let Ok(mut c) = tab_texts.get_mut(child) {
                c.set_if_neq(TextColor(if on { DARK_TEXT } else { LIGHT_TEXT }));
            }
        }
    }
    for (tile, interaction, mut bg, mut border) in &mut tiles {
        let (fill, edge) = match status_of(tile.0) {
            Status::Buyable => (Color::srgb(0.82, 0.81, 0.78), LIGHT_TEXT),
            Status::Full => (Color::srgb(0.6, 0.6, 0.58), Color::NONE),
            Status::TooPoor => (Color::srgb(0.48, 0.46, 0.43), Color::NONE),
        };
        let fill = if *interaction == Interaction::None {
            fill
        } else {
            fill.mix(&Color::WHITE, 0.12)
        };
        bg.set_if_neq(BackgroundColor(fill));
        border.set_if_neq(BorderColor(edge));
    }
    for (icon, mut image) in &mut icons {
        let tint = match status_of(icon.0) {
            Status::Buyable => Color::WHITE,
            Status::Full | Status::TooPoor => Color::srgba(1.0, 1.0, 1.0, 0.5),
        };
        if image.color != tint {
            image.color = tint;
        }
    }
    for (count, mut text, mut color) in &mut counts {
        let k = Lethal::of(count.0);
        let line = format!("{}/{}", carried(&weapon, &state, k), k.max_carried());
        if text.0 != line {
            text.0 = line;
        }
        let c = if status_of(count.0) == Status::Full { FULL_GREY.mix(&DARK_TEXT, 0.4) } else { DARK_TEXT };
        color.set_if_neq(TextColor(c));
    }
    for (f, mut vis) in &mut frames {
        vis.set_if_neq(if f.0 == state.selected { Visibility::Inherited } else { Visibility::Hidden });
    }

    let selected = status_of(state.selected);
    let price = cost(state.selected).unwrap_or(0);
    for (which, mut text, mut color) in &mut texts {
        let (line, c) = match which {
            CraftText::Name => (label(state.selected).to_string(), NAME_RED),
            CraftText::Cost => match selected {
                Status::Full => ("CARRYING THE MOST".to_string(), FULL_GREY),
                Status::TooPoor => (format!("Cost: ${}", crate::util::grouped(price)), POOR_RED),
                Status::Buyable => (format!("Cost: ${}", crate::util::grouped(price)), DARK_TEXT),
            },
            CraftText::Description => (description(&weapon, state.selected), LIGHT_TEXT),
            CraftText::Points => (format!("${}", crate::util::grouped(points)), if selected == Status::TooPoor { POOR_RED } else { MONEY_YELLOW }),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }
    for (_, interaction, mut bg) in &mut buttons {
        let c = if *interaction != Interaction::None {
            Color::srgba(1.0, 1.0, 1.0, 0.12)
        } else {
            Color::NONE
        };
        bg.set_if_neq(BackgroundColor(c));
    }
}

/// Leaving the game with the menu somehow still up: close it.
fn close_on_exit(mut menu: ResMut<menu::Menu>, mut state: ResMut<CraftingMenu>) {
    if menu.screen == Screen::Crafting {
        menu.screen = Screen::None;
        menu.dirty = true;
    }
    *state = CraftingMenu::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_model_stands_on_the_ground_at_its_height() {
        let t = model_transform();
        let lo = t.transform_point(MODEL_MIN);
        let hi = t.transform_point(MODEL_MAX);
        assert!(lo.y.abs() < 1e-4);
        assert!((hi.y - HALF_EXTENTS.y * 2.0).abs() < 1e-4);
        assert!((lo.x + hi.x).abs() < 1e-4 && (lo.z + hi.z).abs() < 1e-4);
    }
}
