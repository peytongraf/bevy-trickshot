//! The leaderboard: held up with the leaderboard key (Tab by default, like
//! Call of Duty), a panel across the middle of the screen ranking everyone in
//! the game, best at the top — in the look of the end-of-game results screen
//! (`menu::results_screen`): a red header bar, dark glass behind, the numbers
//! in gold and our own row edged in gold.
//!
//! Its columns are the mode's: `Zombies` the results screen's (score, kills,
//! critical kills, revives, downs); `FreeForAll` kills, deaths and K/D;
//! `Freestyle` style points, kills and deaths.
//!
//! The panel is `StateScoped(InGame)` and only shown while the key's held
//! (never over a menu or a kill cam); its rows are rebuilt whenever what they
//! show changes. Nothing to reset between games.

use bevy::prelude::*;
use lightyear::prelude::LocalId;
use shared::{GameMode, Lobby, LobbyMember};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::ui::{label_hud, ACCENT, TEXT, TEXT_DIM};
use crate::AppState;

/// Behind it all: dark glass, the world showing through a little.
const PANEL: Color = Color::srgba(0.05, 0.05, 0.06, 0.78);
/// The header bar (the results screen's red) and the column headings' strip.
const HEADER: Color = Color::srgba(0.6, 0.04, 0.05, 0.92);
const HEADINGS: Color = Color::srgba(0.0, 0.0, 0.0, 0.35);
/// Lines between rows, every other row's faint stripe, and ours.
const LINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);
const STRIPE: Color = Color::srgba(1.0, 1.0, 1.0, 0.03);
const MINE: Color = Color::srgba(0.96, 0.78, 0.2, 0.14);
/// The numbers.
const GOLD: Color = Color::srgb(0.98, 0.79, 0.24);

/// The rank column's width (% of the panel).
const RANK_W: f32 = 8.0;
/// A row's height (px).
const ROW_H: f32 = 44.0;

pub(crate) struct LeaderboardPlugin;

impl Plugin for LeaderboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), spawn_leaderboard)
            .add_systems(Update, update_leaderboard.run_if(in_state(AppState::InGame)));
    }
}

#[derive(Component)]
struct LeaderboardRoot;

/// What the panel shows — rebuilt only when this changes.
#[derive(Clone, PartialEq, Debug)]
struct Board {
    /// The mode, top left, and the state of the game, top right.
    mode: &'static str,
    info: String,
    headings: Vec<&'static str>,
    rows: Vec<Row>,
}

#[derive(Clone, PartialEq, Debug)]
struct Row {
    name: String,
    me: bool,
    values: Vec<String>,
}

fn spawn_leaderboard(mut commands: Commands) {
    commands.spawn((
        Name::new("Leaderboard"),
        LeaderboardRoot,
        StateScoped(AppState::InGame),
        // (Over the HUD, under menus.)
        GlobalZIndex(40),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(25.0),
            top: Val::Percent(25.0),
            width: Val::Percent(50.0),
            height: Val::Percent(50.0),
            flex_direction: FlexDirection::Column,
            border: UiRect::top(Val::Px(3.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor(ACCENT),
        Visibility::Hidden,
    ));
}

/// Show the panel while the key's held, and keep its rows up to date.
#[allow(clippy::too_many_arguments)]
fn update_leaderboard(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    menu: Res<crate::menu::Menu>,
    killcam: Res<crate::killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut root: Query<(Entity, &mut Visibility), With<LeaderboardRoot>>,
    // (With the panel it's on: each game's panel is a new one.)
    mut shown: Local<Option<(Entity, Board)>>,
    mut commands: Commands,
) {
    let Ok((root, mut vis)) = root.single_mut() else {
        return;
    };
    let me = local.iter().next().map(|l| l.0);
    let lobby = me.and_then(|me| lobbies.iter().find(|l| l.has(me)));
    let held = binds.leaderboard.pressed(&keys, &mouse) && !menu.is_open() && killcam.0.is_none();
    let Some(lobby) = lobby.filter(|_| held) else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    let board = board(lobby, me);
    if shown.as_ref().is_some_and(|(e, b)| *e == root && *b == board) {
        return;
    }
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|panel| build(panel, &asset_server, &board));
    *shown = Some((root, board));
}

/// The leaderboard for `lobby`'s game, as `me` sees it.
fn board(lobby: &Lobby, me: Option<lightyear::prelude::PeerId>) -> Board {
    let mut members: Vec<&LobbyMember> = lobby.members.iter().collect();
    let clock = (lobby.time_limit_secs > 0).then(|| {
        let t = lobby.time_left_secs;
        format!("{}:{:02}", t / 60, t % 60)
    });
    let (mode, info, headings): (_, String, Vec<&'static str>) = match lobby.mode {
        GameMode::Zombies => {
            members.sort_by(|a, b| b.score.cmp(&a.score).then(b.kills.cmp(&a.kills)));
            let info = if lobby.round == 0 { "STARTING".to_string() } else { format!("ROUND {}", lobby.round) };
            ("ZOMBIES", info, vec!["SCORE", "KILLS", "CRITICAL KILLS", "REVIVES", "DOWNS"])
        }
        GameMode::FreeForAll => {
            members.sort_by(|a, b| b.score.cmp(&a.score).then(a.deaths.cmp(&b.deaths)));
            let first_to = format!("FIRST TO {}", lobby.kill_limit);
            let info = clock.map_or(first_to.clone(), |c| format!("{first_to}   {c}"));
            ("FREE FOR ALL", info, vec!["KILLS", "DEATHS", "K/D"])
        }
        GameMode::Freestyle => {
            members.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then(b.kills.cmp(&a.kills))
                    .then(a.deaths.cmp(&b.deaths))
            });
            ("FREESTYLE", clock.unwrap_or_default(), vec!["SCORE", "KILLS", "DEATHS"])
        }
    };
    let rows = members
        .iter()
        .map(|m| Row {
            name: m.name.to_uppercase(),
            me: Some(m.peer) == me,
            values: match lobby.mode {
                GameMode::Zombies => vec![
                    crate::util::grouped(m.score),
                    m.kills.to_string(),
                    m.critical_kills.to_string(),
                    m.revives.to_string(),
                    m.downs.to_string(),
                ],
                GameMode::FreeForAll => vec![
                    crate::util::grouped(m.score),
                    m.deaths.to_string(),
                    format!("{:.2}", m.score as f32 / m.deaths.max(1) as f32),
                ],
                GameMode::Freestyle => vec![crate::util::grouped(m.score), m.kills.to_string(), m.deaths.to_string()],
            },
        })
        .collect();
    Board {
        mode,
        info,
        headings,
        rows,
    }
}

fn build(panel: &mut ChildSpawnerCommands, asset_server: &AssetServer, board: &Board) {
    // Each value column's width (% of the panel); the name takes the rest.
    let value_w = if board.headings.len() > 3 { 13.0 } else { 15.0 };
    let cell = |width: f32| Node {
        width: Val::Percent(width),
        flex_shrink: 0.0,
        height: Val::Percent(100.0),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };

    // The header bar: the mode, and the state of the game.
    panel
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(52.0),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::horizontal(Val::Px(20.0)),
                ..default()
            },
            BackgroundColor(HEADER),
        ))
        .with_children(|bar| {
            bar.spawn((label_hud(asset_server, board.mode, 32.0, TEXT), TextShadow::default()));
            bar.spawn((label_hud(asset_server, board.info.clone(), 26.0, TEXT), TextShadow::default()));
        });

    // The column headings.
    panel
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(32.0),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(HEADINGS),
        ))
        .with_children(|head| {
            head.spawn(cell(RANK_W));
            head.spawn(Node {
                flex_grow: 1.0,
                padding: UiRect::left(Val::Px(20.0)),
                ..default()
            })
            .with_child(label_hud(asset_server, "PLAYER", 16.0, TEXT_DIM));
            for h in &board.headings {
                head.spawn(cell(value_w)).with_child(label_hud(asset_server, *h, 16.0, TEXT_DIM));
            }
        });

    // Everyone, best first.
    panel
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|list| {
            for (i, row) in board.rows.iter().enumerate() {
                let background = if row.me {
                    MINE
                } else if i % 2 == 1 {
                    STRIPE
                } else {
                    Color::NONE
                };
                list.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(ROW_H),
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        border: UiRect::bottom(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(background),
                    BorderColor(LINE),
                ))
                .with_children(|line| {
                    line.spawn(cell(RANK_W))
                        .with_child(label_hud(asset_server, (i + 1).to_string(), 24.0, TEXT));
                    // The player's plate (ours edged in gold).
                    line.spawn((
                        Node {
                            flex_grow: 1.0,
                            height: Val::Percent(70.0),
                            align_items: AlignItems::Center,
                            padding: UiRect::left(Val::Px(14.0)),
                            border: UiRect::left(Val::Px(4.0)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BorderColor(if row.me { ACCENT } else { Color::srgba(1.0, 1.0, 1.0, 0.2) }),
                    ))
                    .with_child(label_hud(asset_server, row.name.clone(), 24.0, if row.me { ACCENT } else { TEXT }));
                    for (k, value) in row.values.iter().enumerate() {
                        line.spawn((
                            Node {
                                border: UiRect::left(Val::Px(1.0)),
                                height: Val::Percent(60.0),
                                ..cell(value_w)
                            },
                            BorderColor(LINE),
                        ))
                        // (The ranking column biggest.)
                        .with_child(label_hud(asset_server, value.clone(), if k == 0 { 26.0 } else { 22.0 }, GOLD));
                    }
                });
            }
        });
}

