//! The main menu / lobby browser and the lobby-room screen (`bevy_ui`, sharing
//! [`crate::ui`]).
//!
//! Screens follow [`crate::AppState`]:
//! * `MainMenu` — lobby browser. `CREATE LOBBY` / clicking a row sends a
//!   trigger and, once the replicated [`shared::Lobby`] shows us as a member,
//!   we move to `InLobby`.
//! * `InLobby` — member list, a `★` by the party leader, leader-only
//!   `START GAME`, and `LEAVE`. When our lobby's `started` flips true we move to
//!   `InGame`.
//!
//! The tree is rebuilt from scratch whenever [`LobbyUi::dirty`] is set (state
//! change or any replicated `Lobby` change), mirroring `menu.rs`.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use lightyear::prelude::*;

use crate::menu::{Menu, Screen};
use crate::net::GameClient;
use crate::settings::Settings;
use crate::ui::{
    label, label_hud, overlay_root, spawn_button_hud, ui_sound, UiSound, ACCENT, PANEL,
    PANEL_SOLID, ROW, ROW_HOVER, TEXT, TEXT_DIM, TRACK,
};
use crate::AppState;

pub struct LobbyUiPlugin;

impl Plugin for LobbyUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyUi>()
            .init_resource::<AutoLobby>()
            .init_resource::<ScoreboardDirty>()
            .init_resource::<LastMatch>()
            .init_resource::<LastOwnScore>()
            .init_resource::<BotSelection>()
            .add_systems(OnEnter(AppState::MainMenu), mark_dirty_now)
            .add_systems(OnEnter(AppState::InLobby), mark_dirty_now)
            .add_systems(
                OnEnter(AppState::InGame),
                (
                    despawn_lobby_ui,
                    mark_scoreboard_dirty,
                    spawn_match_timer,
                    sync_current_map,
                    reset_own_kill_tracking,
                ),
            )
            .add_systems(Update, catch_match_end)
            .add_systems(
                Update,
                drive_ingame_exit.run_if(in_state(AppState::InGame)),
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
                (
                    (watch_scores, rebuild_scoreboard).chain(),
                    update_match_timer,
                    play_ffa_kill_sound,
                )
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

/// What the leader's "add bots" controls are set to: how many, and at which
/// difficulty. Local UI state — nothing is sent until ADD BOTS is pressed, so
/// the leader can add 4 easy bots, change the selection, and add 5 harder ones.
#[derive(Resource)]
struct BotSelection {
    count: u8,
    difficulty: shared::bot_players::BotDifficulty,
}

impl Default for BotSelection {
    fn default() -> Self {
        Self {
            count: 1,
            difficulty: shared::bot_players::BotDifficulty::default(),
        }
    }
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

/// This client's own kill count (`LobbyMember::score` in `FreeForAll`) as of
/// the last time [`play_ffa_kill_sound`] checked it. Reset on every fresh
/// `OnEnter(AppState::InGame)` ([`reset_own_kill_tracking`]) so a rematch's
/// score resetting to `0` doesn't get read as "score went down" and need to
/// climb back past the previous match's total before it plays a sound again.
#[derive(Resource, Default)]
struct LastOwnScore(Option<u32>);

fn reset_own_kill_tracking(mut last: ResMut<LastOwnScore>) {
    last.0 = None;
}

/// Plays the "kill enemy" sound for this client's own kills in
/// `FreeForAll` — detected as this client's own `LobbyMember::score`
/// increasing, since a `FreeForAll` kill isn't otherwise reported to the
/// killer's client in any more direct way: `PlayerKilled` is a
/// server-only event, and the paired `KillCam` message (see
/// `killcam::receive_killcam`) is only ever sent to the victim.
/// `Freestyle`'s equivalent is `TrickScoredEvent`, already handled by
/// `spawn_score_popup`.
fn play_ffa_kill_sound(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby, Changed<shared::Lobby>>,
    sounds: Res<crate::GameSounds>,
    mut last: ResMut<LastOwnScore>,
    mut commands: Commands,
) {
    let Some(me) = local_peer(&local) else {
        return;
    };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me)) else {
        return;
    };
    if lobby.mode != shared::GameMode::FreeForAll {
        return;
    }
    let Some(member) = lobby.members.iter().find(|m| m.peer == me) else {
        return;
    };
    if let Some(prev) = last.0 {
        if member.score > prev {
            commands.spawn((
                AudioPlayer::new(sounds.kill_enemy.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
    last.0 = Some(member.score);
}

/// Pick up our lobby's selected map on the way into `InGame` (falling back to
/// the default, `BasicMap`, if we somehow aren't in a lobby). `pub`
/// so `main`'s `start_ambient` can order itself `.after` this — both run on
/// `OnEnter(AppState::InGame)`, and `start_ambient` needs this frame's fresh
/// `CurrentMap`, not whatever it was left at after the previous match.
pub(crate) fn sync_current_map(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut current: ResMut<crate::CurrentMap>,
) {
    let me = local_peer(&local);
    let map = me
        .and_then(|me| lobbies.iter().find(|l| l.has(me)))
        .map(|l| l.map)
        .unwrap_or_default();
    current.set_if_neq(crate::CurrentMap(map));
}

/// Leave `InGame` the moment we're no longer in the lobby — because we left
/// (the pause-menu button already sent us on our way), or because the leader
/// pulled the whole party. A normally-finished match — `started` false but
/// we're still a member — is left to the existing lobby-room flow.
fn drive_ingame_exit(
    mut next: ResMut<NextState<AppState>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    let Some(me) = local_peer(&local) else { return };
    if !lobbies.iter().any(|l| l.has(me)) {
        next.set(AppState::MainMenu);
    }
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
    /// Opens `menu::Screen::Loadout` from the main menu. The in-game entry
    /// point is `menu::Btn::OpenLoadout` instead (inside the pause menu) —
    /// see that screen's doc comment for why it looks the same either way.
    OpenLoadout,
    CreateLobby,
    Join(Entity),
    Start,
    Leave,
    TimeDown,
    TimeUp,
    KillDown,
    KillUp,
    SetMode(shared::GameMode),
    SetMap(shared::MapId),
    /// The leader's bot counter (1..=`MAX_BOTS`).
    BotCountDown,
    BotCountUp,
    /// Pick the difficulty the next ADD BOTS will use.
    BotDifficulty(shared::bot_players::BotDifficulty),
    /// Add the selected count at the selected difficulty.
    AddBots,
    /// Remove every bot from the lobby.
    ClearBots,
}

/// Match-length step for the leader's − / + buttons (seconds).
const TIME_STEP_SECS: u32 = 60;
const TIME_MIN_SECS: u32 = 60;
const TIME_MAX_SECS: u32 = 45 * 60;

/// `FreeForAll` kill-limit step for the leader's − / + buttons. Mirrors
/// `server::lobby::{MIN,MAX}_KILL_LIMIT`.
const KILL_STEP: u32 = 5;
const KILL_MIN: u32 = 5;
const KILL_MAX: u32 = 100;

/// Who won the last finished match (name, score) — shown back in the lobby room.
#[derive(Resource, Default)]
struct LastMatch(Option<(String, u32)>);

/// A match just ended: remember the winner and force the room to re-render.
fn catch_match_end(
    mut ended: EventReader<crate::MatchEndedEvent>,
    mut last: ResMut<LastMatch>,
    mut ui: ResMut<LobbyUi>,
) {
    for ev in ended.read() {
        last.0 = Some((ev.winner.clone(), ev.score));
        ui.dirty = true;
    }
}

fn despawn_lobby_ui(mut commands: Commands, roots: Query<Entity, With<LobbyUiRoot>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut ui: ResMut<LobbyUi>,
    state: Res<State<AppState>>,
    last_match: Res<LastMatch>,
    asset_server: Res<AssetServer>,
    roots: Query<Entity, With<LobbyUiRoot>>,
    local: Query<&LocalId, With<GameClient>>,
    connected: Query<(), (With<GameClient>, With<Connected>)>,
    lobbies: Query<(Entity, &shared::Lobby)>,
    bot_selection: Res<BotSelection>,
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
        AppState::MainMenu => build_browser(&mut commands, &asset_server, online, &lobbies),
        AppState::InLobby => {
            if let Some((_, lobby)) = me.and_then(|me| lobbies.iter().find(|(_, l)| l.has(me))) {
                build_room(
                    &mut commands,
                    &asset_server,
                    lobby,
                    me,
                    last_match.0.as_ref(),
                    &bot_selection,
                );
            }
        }
        _ => {}
    }
}

fn build_browser(
    commands: &mut Commands,
    asset_server: &AssetServer,
    online: bool,
    lobbies: &Query<(Entity, &shared::Lobby)>,
) {
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
                col.spawn(label_hud(asset_server, "BEVY TRICKSHOT", 46.0, TEXT));
                col.spawn((
                    Node {
                        width: Val::Px(90.0),
                        height: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                ));
                col.spawn(label_hud(
                    asset_server,
                    if online { "CONNECTED" } else { "CONNECTING…" },
                    14.0,
                    if online { TEXT_DIM } else { ACCENT },
                ));

                // what's new — see `changelog::ENTRIES` doc comment: add a line
                // there with every shipped feature or fix, this panel is the
                // only place players see it.
                col.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(16.0)),
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    BorderRadius::all(Val::Px(8.0)),
                ))
                .with_children(|panel| {
                    panel.spawn(label_hud(
                        asset_server,
                        format!("WHAT'S NEW — v{}", crate::updater::current_version()),
                        15.0,
                        TEXT_DIM,
                    ));
                    for (version, notes) in crate::changelog::ENTRIES {
                        panel.spawn(label_hud(asset_server, format!("v{version}"), 14.0, ACCENT));
                        for note in *notes {
                            panel.spawn(crate::ui::label_body(
                                asset_server,
                                format!("•  {note}"),
                                14.0,
                                TEXT,
                            ));
                        }
                    }
                });

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
                    panel.spawn(label_hud(asset_server, "LOBBIES", 15.0, TEXT_DIM));
                    if open.is_empty() {
                        panel
                            .spawn(Node {
                                flex_grow: 1.0,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            })
                            .with_children(|e| {
                                e.spawn(label_hud(asset_server, "NO LOBBIES AVAILABLE", 20.0, TEXT_DIM));
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
                                    ui_sound(UiSound::MENU),
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
                                    row.spawn(label_hud(asset_server, lobby.name.clone(), 18.0, TEXT));
                                    row.spawn(label_hud(
                                        asset_server,
                                        format!("{}/8", lobby.real_count()),
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
                    spawn_button_hud(
                        row,
                        asset_server,
                        "CREATE LOBBY",
                        20.0,
                        MenuBtn::CreateLobby,
                        ACCENT,
                        ACCENT,
                        PANEL_SOLID,
                        UiSound::MENU,
                    );
                    spawn_button_hud(
                        row,
                        asset_server,
                        "LOADOUT",
                        20.0,
                        MenuBtn::OpenLoadout,
                        ROW,
                        ROW_HOVER,
                        TEXT,
                        UiSound::MENU,
                    );
                });
            });
        });
}

/// One segmented mode-picker button — accent-solid when `mode` is the
/// lobby's current selection, a plain row button otherwise.
fn spawn_mode_button(
    row: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    text: &str,
    mode: shared::GameMode,
    current: shared::GameMode,
) {
    let selected = mode == current;
    spawn_button_hud(
        row,
        asset_server,
        text,
        15.0,
        MenuBtn::SetMode(mode),
        if selected { ACCENT } else { ROW },
        if selected { ACCENT } else { ROW_HOVER },
        if selected { PANEL_SOLID } else { TEXT },
        UiSound::MENU,
    );
}

/// One segmented map-picker button — same look as [`spawn_mode_button`].
fn spawn_map_button(
    row: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    text: &str,
    map: shared::MapId,
    current: shared::MapId,
) {
    let selected = map == current;
    spawn_button_hud(
        row,
        asset_server,
        text,
        15.0,
        MenuBtn::SetMap(map),
        if selected { ACCENT } else { ROW },
        if selected { ACCENT } else { ROW_HOVER },
        if selected { PANEL_SOLID } else { TEXT },
        UiSound::MENU,
    );
}

fn build_room(
    commands: &mut Commands,
    asset_server: &AssetServer,
    lobby: &shared::Lobby,
    me: Option<PeerId>,
    last_match: Option<&(String, u32)>,
    bot_selection: &BotSelection,
) {
    let is_leader = me == Some(lobby.leader);
    let mins = lobby.time_limit_secs / 60;
    let is_ffa = lobby.mode == shared::GameMode::FreeForAll;

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
                col.spawn(label_hud(asset_server, lobby.name.to_uppercase(), 34.0, TEXT));
                col.spawn((
                    Node {
                        width: Val::Px(70.0),
                        height: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                ));

                col.spawn(label_hud(
                    asset_server,
                    if is_ffa {
                        format!(
                            "{}   \u{2022}   {}   \u{2022}   {mins} MIN   \u{2022}   {} KILLS",
                            lobby.mode.label(),
                            lobby.map.label(),
                            lobby.kill_limit,
                        )
                    } else {
                        format!(
                            "{}   \u{2022}   {}   \u{2022}   {mins} MIN",
                            lobby.mode.label(),
                            lobby.map.label(),
                        )
                    },
                    14.0,
                    TEXT_DIM,
                ));

                if let Some((winner, score)) = last_match {
                    col.spawn(label_hud(
                        asset_server,
                        format!("LAST MATCH — {winner} won with {score}"),
                        14.0,
                        ACCENT,
                    ));
                }

                // Leader-only controls, while the lobby is still waiting.
                if is_leader && !lobby.started {
                    col.spawn(Node {
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(label_hud(asset_server, "MODE", 14.0, TEXT_DIM));
                        spawn_mode_button(
                            row,
                            asset_server,
                            "FREESTYLE",
                            shared::GameMode::Freestyle,
                            lobby.mode,
                        );
                        spawn_mode_button(
                            row,
                            asset_server,
                            "FREE FOR ALL",
                            shared::GameMode::FreeForAll,
                            lobby.mode,
                        );
                    });

                    col.spawn(Node {
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(label_hud(asset_server, "MAP", 14.0, TEXT_DIM));
                        spawn_map_button(
                            row,
                            asset_server,
                            "BASIC MAP",
                            shared::MapId::BasicMap,
                            lobby.map,
                        );
                        spawn_map_button(
                            row,
                            asset_server,
                            "SHIPMENT",
                            shared::MapId::Shipment,
                            lobby.map,
                        );
                        spawn_map_button(
                            row,
                            asset_server,
                            "SHIPMENT DAY",
                            shared::MapId::ShipmentDay,
                            lobby.map,
                        );
                        spawn_map_button(
                            row,
                            asset_server,
                            "ASCENSION",
                            shared::MapId::Ascension,
                            lobby.map,
                        );
                    });

                    col.spawn(Node {
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(label_hud(asset_server, "TIME LIMIT", 14.0, TEXT_DIM));
                        spawn_button_hud(
                            row, asset_server, "\u{2212}", 18.0, MenuBtn::TimeDown, ROW, ROW_HOVER, TEXT,
                            UiSound::MENU,
                        );
                        row.spawn(label_hud(asset_server, format!("{mins} min"), 16.0, TEXT));
                        spawn_button_hud(
                            row, asset_server, "+", 18.0, MenuBtn::TimeUp, ROW, ROW_HOVER, TEXT,
                            UiSound::MENU,
                        );
                    });

                    if is_ffa {
                        col.spawn(Node {
                            column_gap: Val::Px(10.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(label_hud(asset_server, "KILL LIMIT", 14.0, TEXT_DIM));
                            spawn_button_hud(
                                row, asset_server, "\u{2212}", 18.0, MenuBtn::KillDown, ROW,
                                ROW_HOVER, TEXT, UiSound::MENU,
                            );
                            row.spawn(label_hud(
                                asset_server,
                                format!("{} kills", lobby.kill_limit),
                                16.0,
                                TEXT,
                            ));
                            spawn_button_hud(
                                row, asset_server, "+", 18.0, MenuBtn::KillUp, ROW, ROW_HOVER, TEXT,
                                UiSound::MENU,
                            );
                        });

                        // Bots: a counter and a difficulty, then ADD. Sent as one
                        // request each press, so different counts / difficulties
                        // can be stacked (e.g. 4 recruits, then 5 veterans).
                        let bots_in = lobby.bot_count();
                        col.spawn(Node {
                            column_gap: Val::Px(10.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(label_hud(asset_server, "BOTS", 14.0, TEXT_DIM));
                            spawn_button_hud(
                                row, asset_server, "\u{2212}", 18.0, MenuBtn::BotCountDown, ROW,
                                ROW_HOVER, TEXT, UiSound::MENU,
                            );
                            row.spawn(label_hud(
                                asset_server,
                                format!("{}", bot_selection.count),
                                16.0,
                                TEXT,
                            ));
                            spawn_button_hud(
                                row, asset_server, "+", 18.0, MenuBtn::BotCountUp, ROW, ROW_HOVER,
                                TEXT, UiSound::MENU,
                            );
                            spawn_button_hud(
                                row,
                                asset_server,
                                &format!(
                                    "ADD {} {} BOT{}",
                                    bot_selection.count,
                                    bot_selection.difficulty.label(),
                                    if bot_selection.count == 1 { "" } else { "S" },
                                ),
                                15.0,
                                MenuBtn::AddBots,
                                ACCENT,
                                ACCENT,
                                PANEL_SOLID,
                                UiSound::MENU,
                            );
                            if bots_in > 0 {
                                spawn_button_hud(
                                    row, asset_server, "CLEAR", 15.0, MenuBtn::ClearBots, ROW,
                                    ROW_HOVER, TEXT, UiSound::MENU,
                                );
                            }
                        });
                        col.spawn(Node {
                            column_gap: Val::Px(10.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(label_hud(asset_server, "DIFFICULTY", 14.0, TEXT_DIM));
                            for d in shared::bot_players::BotDifficulty::ALL {
                                let selected = d == bot_selection.difficulty;
                                spawn_button_hud(
                                    row,
                                    asset_server,
                                    d.label(),
                                    15.0,
                                    MenuBtn::BotDifficulty(d),
                                    if selected { ACCENT } else { ROW },
                                    if selected { ACCENT } else { ROW_HOVER },
                                    if selected { PANEL_SOLID } else { TEXT },
                                    UiSound::MENU,
                                );
                            }
                        });
                    }
                }

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
                    panel.spawn(label_hud(
                        asset_server,
                        format!("PARTY  ({}/8)", lobby.real_count()),
                        15.0,
                        TEXT_DIM,
                    ));
                    for m in lobby.members.iter().filter(|m| m.bot.is_none()) {
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
                                    row.spawn(label_hud(asset_server, "\u{2605}", 18.0, ACCENT)); // ★
                                }
                                row.spawn(label_hud(asset_server, m.name.clone(), 18.0, TEXT));
                            });
                    }

                    // Bots go in compact chips (there can be up to 20), each
                    // with its difficulty.
                    if lobby.bot_count() > 0 {
                        panel.spawn(label_hud(
                            asset_server,
                            format!(
                                "BOTS  ({}/{})",
                                lobby.bot_count(),
                                shared::bot_players::MAX_BOTS
                            ),
                            15.0,
                            TEXT_DIM,
                        ));
                        panel
                            .spawn(Node {
                                width: Val::Percent(100.0),
                                flex_wrap: FlexWrap::Wrap,
                                column_gap: Val::Px(8.0),
                                row_gap: Val::Px(6.0),
                                ..default()
                            })
                            .with_children(|chips| {
                                for m in lobby.members.iter().filter(|m| m.bot.is_some()) {
                                    chips
                                        .spawn((
                                            Node {
                                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                                column_gap: Val::Px(8.0),
                                                align_items: AlignItems::Center,
                                                ..default()
                                            },
                                            BackgroundColor(TRACK),
                                            BorderRadius::all(Val::Px(5.0)),
                                        ))
                                        .with_children(|chip| {
                                            chip.spawn(label_hud(
                                                asset_server,
                                                m.name.clone(),
                                                15.0,
                                                TEXT,
                                            ));
                                            if let Some(d) = m.bot {
                                                chip.spawn(label_hud(
                                                    asset_server,
                                                    d.label(),
                                                    11.0,
                                                    TEXT_DIM,
                                                ));
                                            }
                                        });
                                }
                            });
                    }
                });

                if is_leader {
                    col.spawn(label_hud(asset_server, "You are the party leader.", 13.0, TEXT_DIM));
                } else {
                    col.spawn(label_hud(
                        asset_server,
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
                        spawn_button_hud(
                            row,
                            asset_server,
                            "START GAME",
                            20.0,
                            MenuBtn::Start,
                            ACCENT,
                            ACCENT,
                            PANEL_SOLID,
                            UiSound::MENU,
                        );
                    }
                    spawn_button_hud(
                        row,
                        asset_server,
                        "LEAVE",
                        20.0,
                        MenuBtn::Leave,
                        ROW,
                        ROW_HOVER,
                        TEXT,
                        UiSound::MENU_BACK,
                    );
                });
            });
        });
}

// --- input ------------------------------------------------------------

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn handle_clicks(
    q: Query<(&Interaction, &MenuBtn), Changed<Interaction>>,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut create: Query<&mut TriggerSender<shared::CreateLobby>, With<GameClient>>,
    mut join: Query<&mut TriggerSender<shared::JoinLobby>, With<GameClient>>,
    mut leave: Query<&mut TriggerSender<shared::LeaveLobby>, With<GameClient>>,
    mut start: Query<&mut TriggerSender<shared::StartGame>, With<GameClient>>,
    mut set_time: Query<&mut TriggerSender<shared::SetTimeLimit>, With<GameClient>>,
    mut set_kills: Query<&mut TriggerSender<shared::SetKillLimit>, With<GameClient>>,
    mut set_mode: Query<&mut TriggerSender<shared::SetGameMode>, With<GameClient>>,
    mut set_map: Query<&mut TriggerSender<shared::SetMap>, With<GameClient>>,
    mut bot_selection: ResMut<BotSelection>,
    mut ui: ResMut<LobbyUi>,
    (mut add_bots, mut clear_bots): (
        Query<&mut TriggerSender<shared::AddBots>, With<GameClient>>,
        Query<&mut TriggerSender<shared::ClearBots>, With<GameClient>>,
    ),
) {
    let name = player_name(&settings);
    let my_lobby = || {
        let me = local.iter().next().map(|l| l.0)?;
        lobbies.iter().find(|l| l.has(me))
    };
    let mut nudge_time = |delta: i64| {
        if let (Some(cur), Ok(mut s)) = (my_lobby().map(|l| l.time_limit_secs), set_time.single_mut()) {
            let secs = (cur as i64 + delta).clamp(TIME_MIN_SECS as i64, TIME_MAX_SECS as i64) as u32;
            s.trigger::<shared::LobbyChannel>(shared::SetTimeLimit { secs });
        }
    };
    let mut nudge_kills = |delta: i64| {
        if let (Some(cur), Ok(mut s)) = (my_lobby().map(|l| l.kill_limit), set_kills.single_mut()) {
            let kills = (cur as i64 + delta).clamp(KILL_MIN as i64, KILL_MAX as i64) as u32;
            s.trigger::<shared::LobbyChannel>(shared::SetKillLimit { kills });
        }
    };

    for (interaction, btn) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn {
            MenuBtn::OpenLoadout => {
                menu.loadout_return = menu.screen;
                menu.screen = Screen::Loadout;
                menu.dirty = true;
            }
            MenuBtn::TimeDown => nudge_time(-(TIME_STEP_SECS as i64)),
            MenuBtn::TimeUp => nudge_time(TIME_STEP_SECS as i64),
            MenuBtn::KillDown => nudge_kills(-(KILL_STEP as i64)),
            MenuBtn::KillUp => nudge_kills(KILL_STEP as i64),
            MenuBtn::BotCountDown => {
                bot_selection.count = bot_selection.count.saturating_sub(1).max(1);
                ui.dirty = true;
            }
            MenuBtn::BotCountUp => {
                bot_selection.count = (bot_selection.count + 1).min(shared::bot_players::MAX_BOTS as u8);
                ui.dirty = true;
            }
            MenuBtn::BotDifficulty(d) => {
                bot_selection.difficulty = *d;
                ui.dirty = true;
            }
            MenuBtn::AddBots => {
                if let Ok(mut s) = add_bots.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::AddBots {
                        count: bot_selection.count,
                        difficulty: bot_selection.difficulty,
                    });
                }
            }
            MenuBtn::ClearBots => {
                if let Ok(mut s) = clear_bots.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::ClearBots);
                }
            }
            MenuBtn::SetMode(mode) => {
                if let Ok(mut s) = set_mode.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::SetGameMode { mode: *mode });
                }
            }
            MenuBtn::SetMap(map) => {
                if let Ok(mut s) = set_map.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::SetMap { map: *map });
                }
            }
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

    // Only shown while we're a member of a lobby.
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
            let header = if lobby.mode == shared::GameMode::FreeForAll {
                "KILLS"
            } else {
                "SCORES"
            };
            panel.spawn(label(header, 14.0, TEXT_DIM));
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

// --- match timer (top centre) --------------------------------------

#[derive(Component)]
struct MatchTimerLabel;

/// One text element at the top of the screen; blank while we're not in a lobby.
fn spawn_match_timer(mut commands: Commands) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(14.0),
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_child((
            MatchTimerLabel,
            Text::new(""),
            TextFont {
                font_size: 30.0,
                ..default()
            },
            TextColor(TEXT),
        ));
}

fn update_match_timer(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut text: Query<&mut Text, With<MatchTimerLabel>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let left = local
        .iter()
        .next()
        .map(|l| l.0)
        .and_then(|me| lobbies.iter().find(|l| l.has(me)))
        .map(|l| l.time_left_secs);
    let wanted = match left {
        Some(s) => format!("{}:{:02}", s / 60, s % 60),
        None => String::new(),
    };
    if text.0 != wanted {
        text.0 = wanted;
    }
}
