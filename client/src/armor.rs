//! `Zombies` armor (`shared::armor`): the armor station
//! (`models/props/armor_station.glb`), the prompt shown at it, and the
//! Cold War style menu the interact key opens there — the Pack-a-Punch's
//! look: the three levels as tiles with the armor icon on each (owned /
//! affordable / too expensive — any level above ours can be bought straight
//! away, paying for every one on the way), then a REFILL tile that fills
//! what we own back up for a flat price; the selected tile's cost and what
//! it does in the header, and our points. Mouse to pick, click to buy,
//! `Esc` / EXIT (or the mouse back button) to leave.
//!
//! The menu is [`menu::Screen::Armor`], so while it's up the cursor is free
//! and gameplay input is frozen. A purchase plays the buy sound, asks the
//! server ([`shared::BuyArmor`] / [`shared::RefillArmor`], which re-check
//! everything and take the points) and closes the menu. The armor itself
//! is read off the replicated lobby (`LobbyMember::armor`) — its bars are
//! over each health bar (`zombies_hud`) — and the last of ours breaking
//! plays the armor-destroyed sound.
//!
//! The station, prompt and menu are `StateScoped(InGame)`, the station's
//! taken away whenever we're not in a `Zombies` game, and the menu closes
//! whenever it stops making sense (walked off, died, out of the game) —
//! nothing carries over.

use bevy::audio::Volume;
use bevy::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::armor::{cost_to, numeral, Armor, HALF_EXTENTS, MAX_LEVEL, PLATE_POINTS, REFILL_COST};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::menu::{self, Screen};
use crate::net::GameClient;
use crate::ui::{ui_sound, UiSfx, UiSound};
use crate::zombies_hud::{zombies_game, MONEY_YELLOW};
use crate::{killcam, AppState, GameSounds, Player, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) const ARMOR_STATION_MODEL: &str = "models/props/armor_station.glb";
pub(crate) const ARMOR_ICON: &str = "textures/icons/armor/armor.png";

/// The armor's blue — its bars and the station's colour in the editor.
pub(crate) const ARMOR_BLUE: Color = Color::srgb(0.25, 0.6, 1.0);

/// `armor_station.glb` as made: its bounds (it's ~7.7 units tall, its
/// middle off its origin).
const MODEL_MIN: Vec3 = Vec3::new(-1.191, -0.023, -1.573);
const MODEL_MAX: Vec3 = Vec3::new(3.111, 7.649, 2.374);

/// The model's front faces +X as made: a quarter turn clockwise (seen from
/// above) faces it +Z, the way a placement's front points.
const MODEL_TURN_DEG: f32 = -90.0;

/// The station's model, from the ground under its middle: scaled to
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

pub(crate) struct ArmorPlugin;

impl Plugin for ArmorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArmorMenu>()
            .add_systems(OnEnter(AppState::InGame), spawn_armor_prompt)
            .add_systems(OnExit(AppState::InGame), close_on_exit)
            .add_systems(
                Update,
                (
                    sync_armor_station,
                    play_armor_broken,
                    update_armor_prompt,
                    open_armor_menu.run_if(menu::game_active.and(killcam::no_killcam)),
                    armor_menu_lifecycle,
                    armor_menu_input,
                    refresh_armor_menu,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

// --- the station -----------------------------------------------------------

#[derive(Component)]
struct ArmorStation;

/// Put the station on the map while we're in a `Zombies` game on a map with
/// one (and take it away otherwise), where its layout says.
fn sync_armor_station(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut stations: Query<(Entity, &mut Transform), With<ArmorStation>>,
    mut commands: Commands,
) {
    let Some(at) = zombies_game(&local, &lobbies).and_then(|l| shared::armor::placement(l.map)) else {
        for (e, _) in &stations {
            commands.entity(e).despawn();
        }
        return;
    };
    if let Some((_, mut t)) = stations.iter_mut().next() {
        t.set_if_neq(crate::util::placed(at));
        return;
    }
    let half = HALF_EXTENTS;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            ArmorStation,
            crate::util::placed(at),
            Visibility::default(),
        ))
        .with_children(|s| {
            s.spawn((
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(ARMOR_STATION_MODEL))),
                model_transform(),
            ));
            // Solid — the server's `shared::armor::solid_box`.
            s.spawn((
                bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(Vec3::Y * half.y),
            ));
        });
}

/// Our armor in our `Zombies` game.
fn my_armor(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>) -> Option<Armor> {
    let lobby = zombies_game(local, lobbies)?;
    let me = local.iter().next()?.0;
    lobby.members.iter().find(|m| m.peer == me).map(|m| m.armor)
}

/// The last of our armor just broke: the armor-destroyed sound, just for us.
fn play_armor_broken(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Res<GameSounds>,
    mut had_armor: Local<bool>,
    mut commands: Commands,
) {
    let armor = my_armor(&local, &lobbies).unwrap_or_default();
    let has = armor.points > 0.0;
    // (Losing it all at once with the plates — a new game, a bleed-out —
    // isn't it breaking.)
    if *had_armor && !has && armor.level > 0 {
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.armor_destroy.clone()),
            PlaybackSettings::DESPAWN,
        ));
    }
    *had_armor = has;
}

/// Our lobby's armor and our points, if we're at the station in a running
/// `Zombies` game on a map with one.
fn at_station(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&Lobby>,
    player: &Transform,
) -> Option<(Armor, u32)> {
    let lobby = zombies_game(local, lobbies)?;
    let me = local.iter().next()?.0;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !shared::armor::in_range(lobby.map, feet, 0.0) {
        return None;
    }
    let member = lobby.members.iter().find(|m| m.peer == me)?;
    Some((member.armor, member.score))
}

// --- the prompt at the station ---------------------------------------------

#[derive(Component)]
struct ArmorPrompt;

#[derive(Component)]
struct ArmorPromptText;

const PROMPT_TITLE_BG: Color = Color::srgba(0.04, 0.04, 0.05, 0.92);
const PROMPT_LINE_BG: Color = Color::srgba(0.05, 0.16, 0.32, 0.9);

fn spawn_armor_prompt(mut commands: Commands, asset_server: Res<AssetServer>) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            ArmorPrompt,
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
                Text::new("ARMOR"),
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
                ArmorPromptText,
                Text::new(""),
                TextFont {
                    font: body,
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.93, 0.97)),
            ));
        });
}

/// Show the prompt while we're at the station (hidden behind menus, during
/// a kill cam and once dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_armor_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut prompt: Single<&mut Visibility, With<ArmorPrompt>>,
    mut text: Single<&mut Text, With<ArmorPromptText>>,
) {
    let here = player
        .filter(|_| !menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .and_then(|p| at_station(&local, &lobbies, &p));
    let Some((armor, _)) = here else {
        prompt.set_if_neq(Visibility::Hidden);
        return;
    };
    prompt.set_if_neq(Visibility::Inherited);
    let key = binds.interact.label().to_uppercase();
    let line = if armor.level >= MAX_LEVEL && armor.is_full() {
        "Your armor is maxed out".to_string()
    } else if armor.level > 0 {
        format!("Press {key} to upgrade or refill your armor")
    } else {
        format!("Press {key} to buy armor")
    };
    if text.0 != line {
        text.0 = line;
    }
}

/// The interact key at the station: open the menu, starting on the next
/// level to buy (or REFILL, if we own every level).
#[allow(clippy::too_many_arguments)]
fn open_armor_menu(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<ArmorMenu>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let Some((armor, _)) = player.and_then(|p| at_station(&local, &lobbies, &p)) else {
        return;
    };
    *state = ArmorMenu {
        selected: if armor.level >= MAX_LEVEL {
            Choice::Refill
        } else {
            Choice::Level(armor.level + 1)
        },
        close_pending: false,
    };
    menu.screen = Screen::Armor;
    menu.dirty = true;
}

// --- the menu ----------------------------------------------------------------

/// A tile: a level, or the refill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Level(u8),
    Refill,
}

/// The open menu's state (meaningless while it's closed).
#[derive(Resource)]
struct ArmorMenu {
    /// The tile the header shows and PURCHASE buys: the one hovered last.
    selected: Choice,
    /// Bought or EXIT clicked: close once the mouse button's up again, so the
    /// click can't carry on into a shot the moment gameplay input is back.
    close_pending: bool,
}

impl Default for ArmorMenu {
    fn default() -> Self {
        Self {
            selected: Choice::Level(1),
            close_pending: false,
        }
    }
}

/// Where a tile stands for us.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Owned,
    Buyable,
    TooPoor,
    /// The refill, with nothing to refill (no armor, or it's full).
    Nothing,
}

fn status(choice: Choice, armor: Armor, points: u32) -> Status {
    match choice {
        Choice::Level(level) if level <= armor.level => Status::Owned,
        Choice::Level(level) if points >= cost_to(armor.level, level) => Status::Buyable,
        Choice::Level(_) => Status::TooPoor,
        Choice::Refill if armor.level == 0 || armor.is_full() => Status::Nothing,
        Choice::Refill if points >= REFILL_COST => Status::Buyable,
        Choice::Refill => Status::TooPoor,
    }
}

fn cost(choice: Choice, armor: Armor) -> u32 {
    match choice {
        Choice::Level(level) => cost_to(armor.level, level),
        Choice::Refill => REFILL_COST,
    }
}

fn description(choice: Choice, armor: Armor) -> String {
    match choice {
        Choice::Level(level) => format!(
            "{} armor plate{} — soaks up {:.0} damage before your health.",
            level,
            if level == 1 { "" } else { "s" },
            level as f32 * PLATE_POINTS
        ),
        Choice::Refill if armor.level == 0 => "Refills your armor. Buy some first.".to_string(),
        Choice::Refill if armor.is_full() => "Refills your armor. It's already full.".to_string(),
        Choice::Refill => format!("Refills your armor back up to level {}.", numeral(armor.level)),
    }
}

#[derive(Component)]
struct ArmorMenuRoot;

#[derive(Component, Clone, Copy)]
struct ArmorTile(Choice);

#[derive(Component, Clone, Copy)]
struct ArmorTileIcon(Choice);

#[derive(Component, Clone, Copy)]
struct ArmorTileStatus(Choice);

/// The corner brackets around the selected tile.
#[derive(Component, Clone, Copy)]
struct ArmorTileFrame(Choice);

/// The padlock on the link into level `.0` — shown while the level before
/// it isn't ours yet.
#[derive(Component, Clone, Copy)]
struct ArmorLinkLock(u8);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ArmorText {
    Level,
    Cost,
    Description,
    Points,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ArmorButton {
    Purchase,
    Exit,
}

const PANEL_W: f32 = 860.0;
const TILE: f32 = 130.0;
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
    which: ArmorButton,
    key: &str,
    label: &str,
    heading: &Handle<Font>,
) {
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
    if which == ArmorButton::Exit {
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
            width: Val::Px(60.0),
            height: Val::Px(TILE),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|link| {
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
                ArmorLinkLock(into),
                Node {
                    width: Val::Px(34.0),
                    height: Val::Px(34.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                BorderRadius::all(Val::Percent(50.0)),
            ))
            .with_children(|lock| {
                lock.spawn((
                    Node {
                        width: Val::Px(11.0),
                        height: Val::Px(7.0),
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
                        width: Val::Px(16.0),
                        height: Val::Px(11.0),
                        ..default()
                    },
                    BackgroundColor(DARK_TEXT),
                    BorderRadius::all(Val::Px(2.0)),
                ));
            });
        });
}

/// One tile: the armor icon in its middle, its numeral (or REFILL) in the
/// corner, its price / OWNED along the bottom, and corner brackets when
/// it's selected.
fn spawn_tile(
    parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    choice: Choice,
    heading: &Handle<Font>,
    icon: &Handle<Image>,
) {
    parent
        .spawn((
            ArmorTile(choice),
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
                ArmorTileIcon(choice),
                ImageNode::new(icon.clone()),
                Node {
                    width: Val::Px(TILE * 0.56),
                    height: Val::Px(TILE * 0.56),
                    margin: UiRect::bottom(Val::Px(14.0)),
                    ..default()
                },
            ));
            // The numeral (or REFILL), bottom right like Cold War's.
            let corner = match choice {
                Choice::Level(level) => numeral(level),
                Choice::Refill => "REFILL",
            };
            tile.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(8.0),
                    top: Val::Px(4.0),
                    ..default()
                },
            ))
            .with_child((
                Text::new(corner),
                TextFont {
                    font: heading.clone(),
                    font_size: if choice == Choice::Refill { 22.0 } else { 30.0 },
                    ..default()
                },
                TextColor(DARK_TEXT.with_alpha(0.7)),
            ));
            tile.spawn(Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(4.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_child((
                ArmorTileStatus(choice),
                Text::new(""),
                TextFont {
                    font: heading.clone(),
                    font_size: 22.0,
                    ..default()
                },
                TextColor(DARK_TEXT),
            ));
            tile.spawn((
                ArmorTileFrame(choice),
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

fn spawn_armor_menu(commands: &mut Commands, asset_server: &AssetServer) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    let icon = asset_server.load(ARMOR_ICON);
    let hfont = |size: f32| TextFont {
        font: heading.clone(),
        font_size: size,
        ..default()
    };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            ArmorMenuRoot,
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
                // Tab bar: just the one tab.
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
                        .with_child((Text::new("ARMOR"), hfont(38.0), TextColor(DARK_TEXT)));
                    });
                // Header: the selected tile, and its cost.
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
                        h.spawn((ArmorText::Level, Text::new(""), hfont(64.0), TextColor(LEVEL_RED)));
                        h.spawn((ArmorText::Cost, Text::new(""), hfont(44.0), TextColor(DARK_TEXT)));
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
                                ArmorText::Description,
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
                                spawn_tile(row, Choice::Level(level), &heading, &icon);
                            }
                            // (A gap, then the refill — apart from the chain.)
                            row.spawn(Node {
                                width: Val::Px(56.0),
                                ..default()
                            });
                            spawn_tile(row, Choice::Refill, &heading, &icon);
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
                                spawn_control(c, ArmorButton::Purchase, "LMB", "PURCHASE", &heading);
                                spawn_control(c, ArmorButton::Exit, "ESC", "EXIT", &heading);
                            });
                            f.spawn((ArmorText::Points, Text::new(""), hfont(48.0), TextColor(MONEY_YELLOW)));
                        });
                    });
            });
        });
}

/// Build the menu when it opens and take it down when it closes; close it
/// once a purchase / EXIT click is released, or as soon as it no longer
/// makes sense (away from the station, dead, out of the game).
#[allow(clippy::too_many_arguments)]
fn armor_menu_lifecycle(
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<ArmorMenu>,
    mouse: Res<ButtonInput<MouseButton>>,
    death: Res<crate::death_effect::DeathEffect>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    roots: Query<Entity, With<ArmorMenuRoot>>,
    mut commands: Commands,
) {
    if menu.screen == Screen::Armor {
        // The mouse back button leaves like EXIT does (once it's released).
        if mouse.just_pressed(MouseButton::Back) {
            state.close_pending = true;
        }
        let still_here = !death.is_active() && player.is_some_and(|p| at_station(&local, &lobbies, &p).is_some());
        let released = state.close_pending && mouse.get_pressed().next().is_none();
        if !still_here || released {
            menu.screen = Screen::None;
            menu.dirty = true;
            state.close_pending = false;
        }
    }
    let show = menu.screen == Screen::Armor && !state.close_pending;
    match (show, roots.is_empty()) {
        (true, true) => spawn_armor_menu(&mut commands, &asset_server),
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
fn armor_menu_input(
    menu: Res<menu::Menu>,
    mut state: ResMut<ArmorMenu>,
    sounds: Option<Res<GameSounds>>,
    sfx: Option<Res<UiSfx>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut moved: EventReader<CursorMoved>,
    tiles: Query<(&ArmorTile, &Interaction)>,
    pressed_tiles: Query<(&ArmorTile, &Interaction), Changed<Interaction>>,
    buttons: Query<(&ArmorButton, &Interaction), Changed<Interaction>>,
    mut buy_sender: Query<&mut TriggerSender<shared::BuyArmor>, With<GameClient>>,
    mut refill_sender: Query<&mut TriggerSender<shared::RefillArmor>, With<GameClient>>,
    mut commands: Commands,
) {
    let cursor_moved = moved.read().count() > 0;
    if menu.screen != Screen::Armor || state.close_pending {
        return;
    }
    // Only when the mouse moves: the freed cursor starts in the middle of
    // the screen, right over a tile, and that shouldn't pick it.
    if cursor_moved {
        let hovered = tiles.iter().find(|(_, i)| **i != Interaction::None).map(|(t, _)| t.0);
        if let Some(choice) = hovered.filter(|c| *c != state.selected) {
            state.selected = choice;
            if let Some(sfx) = &sfx {
                commands.spawn((
                    AudioPlayer::new(sfx.button_hover.clone()),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.6)),
                ));
            }
        }
    }
    let mut buy: Option<Choice> = None;
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
            ArmorButton::Purchase => buy = Some(state.selected),
            ArmorButton::Exit => state.close_pending = true,
        }
    }
    let Some(choice) = buy else {
        return;
    };
    let me = local.iter().next().map(|l| l.0);
    let (armor, points) = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or((Armor::default(), 0), |m| (m.armor, m.score));
    let sent = status(choice, armor, points) == Status::Buyable
        && match choice {
            Choice::Level(level) => buy_sender.single_mut().is_ok_and(|mut s| {
                s.trigger::<shared::LobbyChannel>(shared::BuyArmor { level });
                true
            }),
            Choice::Refill => refill_sender.single_mut().is_ok_and(|mut s| {
                s.trigger::<shared::LobbyChannel>(shared::RefillArmor);
                true
            }),
        };
    if sent {
        info!("asked the server for armor: {choice:?}");
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.buy_ammo.clone()),
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
fn refresh_armor_menu(
    menu: Res<menu::Menu>,
    state: Res<ArmorMenu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut tiles: Query<(&ArmorTile, &Interaction, &mut BackgroundColor, &mut BorderColor), Without<ArmorButton>>,
    mut icons: Query<(&ArmorTileIcon, &mut ImageNode)>,
    mut statuses: Query<(&ArmorTileStatus, &mut Text, &mut TextColor), Without<ArmorText>>,
    mut frames: Query<(&ArmorTileFrame, &mut Visibility), Without<ArmorLinkLock>>,
    mut locks: Query<(&ArmorLinkLock, &mut Visibility), Without<ArmorTileFrame>>,
    mut texts: Query<(&ArmorText, &mut Text, &mut TextColor), Without<ArmorTileStatus>>,
    mut buttons: Query<(&ArmorButton, &Interaction, &mut BackgroundColor), Without<ArmorTile>>,
) {
    if menu.screen != Screen::Armor {
        return;
    }
    let me = local.iter().next().map(|l| l.0);
    let (armor, points) = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or((Armor::default(), 0), |m| (m.armor, m.score));
    let status_of = |c: Choice| status(c, armor, points);

    for (tile, interaction, mut bg, mut border) in &mut tiles {
        let (fill, edge) = match status_of(tile.0) {
            Status::Owned => (Color::srgb(0.86, 0.88, 0.9), Color::srgb(0.86, 0.88, 0.9)),
            Status::Buyable => (Color::srgb(0.58, 0.56, 0.52), LIGHT_TEXT),
            Status::TooPoor | Status::Nothing => (Color::srgb(0.42, 0.4, 0.37), Color::NONE),
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
            Status::Owned | Status::Buyable => Color::WHITE,
            Status::TooPoor | Status::Nothing => Color::srgba(1.0, 1.0, 1.0, 0.45),
        };
        if image.color != tint {
            image.color = tint;
        }
    }
    for (s, mut text, mut color) in &mut statuses {
        let price = format!("${}", cost(s.0, armor));
        let (line, c) = match status_of(s.0) {
            Status::Owned => ("OWNED".to_string(), OWNED_GREEN),
            Status::Buyable => (price, MONEY_YELLOW),
            Status::TooPoor => (price, POOR_RED),
            Status::Nothing => (price, DARK_TEXT.with_alpha(0.5)),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }
    for (f, mut vis) in &mut frames {
        vis.set_if_neq(if f.0 == state.selected { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (l, mut vis) in &mut locks {
        vis.set_if_neq(if l.0 > armor.level + 1 { Visibility::Inherited } else { Visibility::Hidden });
    }

    let selected = status_of(state.selected);
    for (which, mut text, mut color) in &mut texts {
        let (line, c) = match which {
            ArmorText::Level => (
                match state.selected {
                    Choice::Level(level) => format!("LEVEL {}", numeral(level)),
                    Choice::Refill => "REFILL".to_string(),
                },
                LEVEL_RED,
            ),
            ArmorText::Cost => {
                let price = format!("Cost: ${}", cost(state.selected, armor));
                match selected {
                    Status::Owned => ("OWNED".to_string(), OWNED_GREEN),
                    Status::TooPoor => (price, POOR_RED),
                    Status::Buyable => (price, DARK_TEXT),
                    Status::Nothing => (price, DARK_TEXT.with_alpha(0.5)),
                }
            }
            ArmorText::Description => (description(state.selected, armor), LIGHT_TEXT),
            ArmorText::Points => (format!("${points}"), MONEY_YELLOW),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }
    for (button, interaction, mut bg) in &mut buttons {
        // PURCHASE only lights up when there's something to buy.
        let live = *button == ArmorButton::Exit || selected == Status::Buyable;
        let c = if live && *interaction != Interaction::None {
            Color::srgba(1.0, 1.0, 1.0, 0.12)
        } else {
            Color::NONE
        };
        bg.set_if_neq(BackgroundColor(c));
    }
}

/// Leaving the game with the menu somehow still up: close it.
fn close_on_exit(mut menu: ResMut<menu::Menu>, mut state: ResMut<ArmorMenu>) {
    if menu.screen == Screen::Armor {
        menu.screen = Screen::None;
        menu.dirty = true;
    }
    *state = ArmorMenu::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_station_model_stands_on_the_ground_at_its_height() {
        let t = model_transform();
        let lo = t.transform_point(MODEL_MIN);
        let hi = t.transform_point(MODEL_MAX);
        assert!(lo.y.abs() < 1e-4);
        assert!((hi.y - HALF_EXTENTS.y * 2.0).abs() < 1e-4);
        assert!((lo.x + hi.x).abs() < 1e-4 && (lo.z + hi.z).abs() < 1e-4);
    }
}
