//! The main menu / lobby browser and the lobby-room screen (`bevy_ui`, sharing
//! [`crate::ui`]).
//!
//! Screens follow [`crate::AppState`]:
//! * `MainMenu` — lobby browser. `PRACTICE` → solo `InGame` (no networking);
//!   `CREATE LOBBY` / clicking a row sends a trigger and, once the replicated
//!   [`shared::Lobby`] shows us as a member, we move to `InLobby`.
//! * `InLobby` — member list, a `★` by the party leader, leader-only
//!   `START GAME`, and `LEAVE`. When our lobby's `started` flips true we move to
//!   `InGame`.
//!
//! The tree is rebuilt from scratch whenever [`LobbyUi::dirty`] is set (state
//! change or any replicated `Lobby` change), mirroring `menu.rs`.

use bevy::prelude::*;
use lightyear::prelude::*;

use crate::net::GameClient;
use crate::settings::Settings;
use crate::ui::{
    label, overlay_root, spawn_button, ACCENT, PANEL, PANEL_SOLID, ROW, ROW_HOVER, TEXT, TEXT_DIM,
    TRACK,
};
use crate::AppState;

pub struct LobbyUiPlugin;

impl Plugin for LobbyUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyUi>()
            .init_resource::<AutoLobby>()
            .init_resource::<ScoreboardDirty>()
            .add_systems(OnEnter(AppState::MainMenu), mark_dirty_now)
            .add_systems(OnEnter(AppState::InLobby), mark_dirty_now)
            .add_systems(
                OnEnter(AppState::InGame),
                (despawn_lobby_ui, mark_scoreboard_dirty),
            )
            .add_systems(
                Update,
                (
                    watch_lobbies,
                    auto_lobby.run_if(in_state(AppState::MainMenu)),
                    auto_start.run_if(in_state(AppState::InLobby)),
                    drive_transitions,
                    rebuild,
                    handle_clicks,
                )
                    .chain()
                    .run_if(in_menu),
            )
            .add_systems(
                Update,
                (watch_scores, rebuild_scoreboard)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn in_menu(state: Res<State<AppState>>) -> bool {
    matches!(state.get(), AppState::MainMenu | AppState::InLobby)
}

#[derive(Resource, Default)]
struct LobbyUi {
    dirty: bool,
}

fn mark_dirty_now(mut ui: ResMut<LobbyUi>) {
    ui.dirty = true;
}

fn player_name(settings: &Settings) -> String {
    settings
        .username
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Player".to_string())
}

fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

/// Dev convenience (persisted settings or `TRICKSHOT_AUTO_CREATE` /
/// `TRICKSHOT_AUTO_JOIN` env vars): once connected and the lobby list has
/// settled, auto-join an open lobby, else auto-create one. Fires once.
#[derive(Resource, Default)]
struct AutoLobby {
    done: bool,
    settle: u32,
}

fn auto_lobby(
    mut auto: ResMut<AutoLobby>,
    settings: Res<Settings>,
    connected: Query<(), (With<GameClient>, With<Connected>)>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<(Entity, &shared::Lobby)>,
    mut create: Query<&mut TriggerSender<shared::CreateLobby>, With<GameClient>>,
    mut join: Query<&mut TriggerSender<shared::JoinLobby>, With<GameClient>>,
) {
    if auto.done || connected.is_empty() {
        return;
    }
    let want_create = settings.dev_auto_create_lobby || env_flag("TRICKSHOT_AUTO_CREATE");
    let want_join = settings.dev_auto_join_lobby || env_flag("TRICKSHOT_AUTO_JOIN");
    if !want_create && !want_join {
        auto.done = true;
        return;
    }

    // Let a bit of replication arrive before deciding there are no lobbies.
    auto.settle += 1;
    if auto.settle < 40 {
        return;
    }

    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    if lobbies.iter().any(|(_, l)| l.has(me)) {
        auto.done = true;
        return;
    }

    let name = player_name(&settings);
    let open = lobbies.iter().find(|(_, l)| !l.started).map(|(e, _)| e);

    if want_join {
        if let Some(e) = open {
            if let Ok(mut s) = join.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::JoinLobby {
                    lobby: e,
                    player_name: name,
                });
                info!("[auto] joining lobby {e:?}");
                auto.done = true;
            }
            return;
        }
        if !want_create {
            return; // keep waiting for a lobby to appear
        }
    }

    if want_create {
        if let Ok(mut s) = create.single_mut() {
            s.trigger::<shared::LobbyChannel>(shared::CreateLobby {
                name: String::new(),
                player_name: name,
            });
            info!("[auto] creating lobby");
            auto.done = true;
        }
    }
}

/// Re-render on any replicated lobby change or when we learn our own peer id.
fn watch_lobbies(
    mut ui: ResMut<LobbyUi>,
    // `Changed` also fires on insert, so this covers newly-replicated lobbies too.
    changed: Query<(), Changed<shared::Lobby>>,
    mut removed: RemovedComponents<shared::Lobby>,
    local_added: Query<(), (With<GameClient>, Added<LocalId>)>,
) {
    if !changed.is_empty() || removed.read().count() > 0 || !local_added.is_empty() {
        ui.dirty = true;
    }
}

/// Our peer id, once connected.
fn local_peer(q: &Query<&LocalId, With<GameClient>>) -> Option<PeerId> {
    q.iter().next().map(|l| l.0)
}

/// Move between menu screens based on where the replicated lobby state puts us.
fn drive_transitions(
    state: Res<State<AppState>>,
    mut next: ResMut<NextState<AppState>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    let Some(me) = local_peer(&local) else { return };
    let mine = lobbies.iter().find(|l| l.has(me));

    match state.get() {
        AppState::MainMenu => {
            if mine.is_some() {
                next.set(AppState::InLobby);
            }
        }
        AppState::InLobby => match mine {
            None => next.set(AppState::MainMenu),
            Some(l) if l.started => next.set(AppState::InGame),
            _ => {}
        },
        _ => {}
    }
}

/// Dev convenience (`TRICKSHOT_AUTO_START` env): the party leader starts the game
/// a few seconds after landing in the lobby, so two dev clients need no clicks.
fn auto_start(
    mut fired: Local<bool>,
    mut settle: Local<u32>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut start: Query<&mut TriggerSender<shared::StartGame>, With<GameClient>>,
) {
    if *fired || !env_flag("TRICKSHOT_AUTO_START") {
        return;
    }
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me)) else {
        return;
    };
    if lobby.leader != me {
        return;
    }
    *settle += 1;
    if *settle < 360 {
        return; // ~6s at 60fps, so a joiner has time to appear
    }
    if let Ok(mut s) = start.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::StartGame);
        info!("[auto] starting game");
        *fired = true;
    }
}

// --- rendering ----------------------------------------------------------

#[derive(Component)]
struct LobbyUiRoot;

/// Click intent for a menu button.
#[derive(Component, Clone)]
enum MenuBtn {
    Practice,
    CreateLobby,
    Join(Entity),
    Start,
    Leave,
}

fn despawn_lobby_ui(mut commands: Commands, roots: Query<Entity, With<LobbyUiRoot>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

fn rebuild(
    mut commands: Commands,
    mut ui: ResMut<LobbyUi>,
    state: Res<State<AppState>>,
    roots: Query<Entity, With<LobbyUiRoot>>,
    local: Query<&LocalId, With<GameClient>>,
    connected: Query<(), (With<GameClient>, With<Connected>)>,
    lobbies: Query<(Entity, &shared::Lobby)>,
) {
    if !ui.dirty {
        return;
    }
    ui.dirty = false;
    for e in &roots {
        commands.entity(e).despawn();
    }

    let me = local_peer(&local);
    let online = !connected.is_empty();

    match state.get() {
        AppState::MainMenu => build_browser(&mut commands, online, &lobbies),
        AppState::InLobby => {
            if let Some((_, lobby)) = me.and_then(|me| lobbies.iter().find(|(_, l)| l.has(me))) {
                build_room(&mut commands, lobby, me);
            }
        }
        _ => {}
    }
}

fn build_browser(commands: &mut Commands, online: bool, lobbies: &Query<(Entity, &shared::Lobby)>) {
    let open: Vec<(Entity, &shared::Lobby)> = lobbies.iter().filter(|(_, l)| !l.started).collect();

    commands
        .spawn((LobbyUiRoot, GlobalZIndex(10), overlay_root(true)))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Px(760.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(20.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn(label("BEVY TRICKSHOT", 46.0, TEXT));
                col.spawn((
                    Node {
                        width: Val::Px(90.0),
                        height: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                ));
                col.spawn(label(
                    if online { "CONNECTED" } else { "CONNECTING…" },
                    14.0,
                    if online { TEXT_DIM } else { ACCENT },
                ));

                // lobby list
                col.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(280.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(16.0)),
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    BorderRadius::all(Val::Px(8.0)),
                ))
                .with_children(|panel| {
                    panel.spawn(label("LOBBIES", 15.0, TEXT_DIM));
                    if open.is_empty() {
                        panel
                            .spawn(Node {
                                flex_grow: 1.0,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            })
                            .with_children(|e| {
                                e.spawn(label("NO LOBBIES AVAILABLE", 20.0, TEXT_DIM));
                            });
                    } else {
                        for (entity, lobby) in &open {
                            panel
                                .spawn((
                                    Button,
                                    Interaction::default(),
                                    MenuBtn::Join(*entity),
                                    crate::ui::Hoverable {
                                        normal: ROW,
                                        hover: ROW_HOVER,
                                    },
                                    Node {
                                        width: Val::Percent(100.0),
                                        padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                                        justify_content: JustifyContent::SpaceBetween,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    BackgroundColor(ROW),
                                    BorderRadius::all(Val::Px(5.0)),
                                ))
                                .with_children(|row| {
                                    row.spawn(label(lobby.name.clone(), 18.0, TEXT));
                                    row.spawn(label(
                                        format!("{}/8", lobby.members.len()),
                                        16.0,
                                        TEXT_DIM,
                                    ));
                                });
                        }
                    }
                });

                // actions
                col.spawn(Node {
                    column_gap: Val::Px(14.0),
                    ..default()
                })
                .with_children(|row| {
                    spawn_button(
                        row,
                        "CREATE LOBBY",
                        20.0,
                        MenuBtn::CreateLobby,
                        ACCENT,
                        ACCENT,
                        PANEL_SOLID,
                    );
                    spawn_button(
                        row,
                        "PRACTICE",
                        20.0,
                        MenuBtn::Practice,
                        ROW,
                        ROW_HOVER,
                        TEXT,
                    );
                });
            });
        });
}

fn build_room(commands: &mut Commands, lobby: &shared::Lobby, me: Option<PeerId>) {
    let is_leader = me == Some(lobby.leader);

    commands
        .spawn((LobbyUiRoot, GlobalZIndex(10), overlay_root(true)))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Px(620.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(18.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn(label(lobby.name.to_uppercase(), 34.0, TEXT));
                col.spawn((
                    Node {
                        width: Val::Px(70.0),
                        height: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                ));

                col.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(16.0)),
                        row_gap: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    BorderRadius::all(Val::Px(8.0)),
                ))
                .with_children(|panel| {
                    panel.spawn(label(
                        format!("PARTY  ({}/8)", lobby.members.len()),
                        15.0,
                        TEXT_DIM,
                    ));
                    for m in &lobby.members {
                        panel
                            .spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                                    column_gap: Val::Px(10.0),
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(TRACK),
                                BorderRadius::all(Val::Px(5.0)),
                            ))
                            .with_children(|row| {
                                if m.peer == lobby.leader {
                                    row.spawn(label("\u{2605}", 18.0, ACCENT)); // ★
                                }
                                row.spawn(label(m.name.clone(), 18.0, TEXT));
                            });
                    }
                });

                if is_leader {
                    col.spawn(label("You are the party leader.", 13.0, TEXT_DIM));
                } else {
                    col.spawn(label(
                        "Waiting for the party leader to start…",
                        13.0,
                        TEXT_DIM,
                    ));
                }

                col.spawn(Node {
                    column_gap: Val::Px(14.0),
                    ..default()
                })
                .with_children(|row| {
                    if is_leader {
                        spawn_button(
                            row,
                            "START GAME",
                            20.0,
                            MenuBtn::Start,
                            ACCENT,
                            ACCENT,
                            PANEL_SOLID,
                        );
                    }
                    spawn_button(row, "LEAVE", 20.0, MenuBtn::Leave, ROW, ROW_HOVER, TEXT);
                });
            });
        });
}

// --- input ------------------------------------------------------------

#[allow(clippy::type_complexity)]
fn handle_clicks(
    q: Query<(&Interaction, &MenuBtn), Changed<Interaction>>,
    mut next: ResMut<NextState<AppState>>,
    settings: Res<Settings>,
    mut create: Query<&mut TriggerSender<shared::CreateLobby>, With<GameClient>>,
    mut join: Query<&mut TriggerSender<shared::JoinLobby>, With<GameClient>>,
    mut leave: Query<&mut TriggerSender<shared::LeaveLobby>, With<GameClient>>,
    mut start: Query<&mut TriggerSender<shared::StartGame>, With<GameClient>>,
) {
    let name = player_name(&settings);

    for (interaction, btn) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn {
            MenuBtn::Practice => next.set(AppState::InGame),
            MenuBtn::CreateLobby => {
                if let Ok(mut s) = create.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::CreateLobby {
                        name: String::new(),
                        player_name: name.clone(),
                    });
                }
            }
            MenuBtn::Join(entity) => {
                if let Ok(mut s) = join.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::JoinLobby {
                        lobby: *entity,
                        player_name: name.clone(),
                    });
                }
            }
            MenuBtn::Start => {
                if let Ok(mut s) = start.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::StartGame);
                }
            }
            MenuBtn::Leave => {
                if let Ok(mut s) = leave.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::LeaveLobby);
                }
            }
        }
    }
}

// --- in-game scoreboard (left edge) ----------------------------------

#[derive(Component)]
struct Scoreboard;

/// Set whenever the scoreboard needs rebuilding (a `Lobby` changed, or we just
/// entered the game).
#[derive(Resource, Default)]
struct ScoreboardDirty(bool);

fn mark_scoreboard_dirty(mut dirty: ResMut<ScoreboardDirty>) {
    dirty.0 = true;
}

fn watch_scores(mut dirty: ResMut<ScoreboardDirty>, changed: Query<(), Changed<shared::Lobby>>) {
    if !changed.is_empty() {
        dirty.0 = true;
    }
}

fn rebuild_scoreboard(
    mut commands: Commands,
    mut dirty: ResMut<ScoreboardDirty>,
    existing: Query<Entity, With<Scoreboard>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    if !dirty.0 {
        return;
    }
    dirty.0 = false;
    for e in &existing {
        commands.entity(e).despawn();
    }

    // Only in a lobby game (solo Practice has no lobby → no scoreboard).
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me)) else {
        return;
    };

    let mut rows: Vec<(&str, u32, bool)> = lobby
        .members
        .iter()
        .map(|m| (m.name.as_str(), m.score, m.peer == me))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));

    commands
        .spawn((
            Scoreboard,
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                top: Val::Px(96.0),
                min_width: Val::Px(200.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_children(|panel| {
            panel.spawn(label("SCORES", 14.0, TEXT_DIM));
            for (name, score, is_me) in rows {
                let col = if is_me { ACCENT } else { TEXT };
                panel
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(16.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(label(name, 17.0, col));
                        row.spawn(label(score.to_string(), 17.0, col));
                    });
            }
        });
}
