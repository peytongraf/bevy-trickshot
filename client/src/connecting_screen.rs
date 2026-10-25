//! The "CONNECTING" cover: a full-screen overlay, styled like the launch
//! loading screen (`loading_screen`), over the main menu and lobby room
//! whenever there's no connection to the server — on launch until the first
//! connect, and again if the connection drops. It says what's going on: the
//! address being dialed, which attempt this is, and — once one has failed —
//! why, and when the next try is (`net::ServerLink`, which retries on its
//! own). After a failure it offers RETRY NOW and CONTINUE OFFLINE (the
//! level / weapon editors don't need the server); Esc and the mouse back
//! button do the same as CONTINUE OFFLINE.
//!
//! Losing the connection in a lobby room or mid-game drops back to the main
//! menu: the lobby went with the connection.
//!
//! The "continue offline" choice lasts until the next connect, so a later
//! drop shows the cover again.
//!
//! The prod server sleeps while nobody's playing (`net::ServerWake`), so the
//! first player in usually waits a few seconds for it to start: the cover
//! says "WAKING SERVER" then, with how long it's been, instead of the failed
//! attempts that are expected meanwhile.

use bevy::prelude::*;
use lightyear::prelude::*;

use crate::menu::Menu;
use crate::net::{GameClient, ServerLink, ServerWake};
use crate::ui::{label_body, label_hud, spawn_button_hud, UiSound, ACCENT, ROW, ROW_HOVER, TEXT, TEXT_DIM};
use crate::AppState;

/// Above the menus (`GlobalZIndex(50)`–`(60)`), below the launch loading
/// screen.
const Z: i32 = 100;
const COVER: Color = Color::srgb(0.02, 0.02, 0.025);
/// A knock the server answers this fast was on an already-running one —
/// don't flash "WAKING SERVER" for it.
const WAKING_SHOWN_AFTER_SECS: f32 = 1.0;

pub(crate) struct ConnectingScreenPlugin;

impl Plugin for ConnectingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Offline>().add_systems(
            Update,
            (
                leave_on_disconnect,
                sync_cover,
                connecting_back.before(crate::menu::menu_toggle),
                (update_cover, cover_clicks).after(sync_cover),
            ),
        );
    }
}

/// The player picked CONTINUE OFFLINE; cleared on connecting.
#[derive(Resource, Default)]
struct Offline(bool);

#[derive(Component)]
struct ConnectingCover;

#[derive(Component)]
struct ConnectingTitle;

#[derive(Component)]
struct ConnectingStatus;

#[derive(Component)]
struct ConnectingDetail;

/// The row of buttons, shown once an attempt has failed.
#[derive(Component)]
struct ConnectingButtons;

#[derive(Component, Clone, Copy)]
enum CoverBtn {
    Retry,
    Offline,
}

fn connected(q: &Query<(), (With<GameClient>, With<Connected>)>) -> bool {
    !q.is_empty()
}

/// The lobby (or game) we were in went with the connection: back to the
/// main menu, where the cover shows.
fn leave_on_disconnect(
    state: Res<State<AppState>>,
    mut next: ResMut<NextState<AppState>>,
    q: Query<(), (With<GameClient>, With<Connected>)>,
    mut was: Local<bool>,
) {
    let now = connected(&q);
    if *was && !now && matches!(state.get(), AppState::InLobby | AppState::InGame) {
        next.set(AppState::MainMenu);
    }
    *was = now;
}

fn sync_cover(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    state: Res<State<AppState>>,
    q: Query<(), (With<GameClient>, With<Connected>)>,
    mut offline: ResMut<Offline>,
    covers: Query<Entity, With<ConnectingCover>>,
) {
    let online = connected(&q);
    if online {
        offline.0 = false;
    }
    let show = !online && !offline.0 && matches!(state.get(), AppState::MainMenu | AppState::InLobby);
    match (show, covers.is_empty()) {
        (true, true) => spawn_cover(&mut commands, &asset_server),
        (false, false) => {
            for e in &covers {
                commands.entity(e).despawn();
            }
        }
        _ => {}
    }
}

fn spawn_cover(commands: &mut Commands, asset_server: &AssetServer) {
    commands
        .spawn((
            ConnectingCover,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(COVER),
            GlobalZIndex(Z),
            // Swallows clicks meant for the menu underneath.
            Interaction::default(),
            bevy::ui::FocusPolicy::Block,
        ))
        .with_children(|c| {
            c.spawn((ConnectingTitle, label_hud(asset_server, "CONNECTING", 34.0, TEXT)));
            c.spawn((ConnectingStatus, label_body(asset_server, "", 16.0, TEXT_DIM)));
            c.spawn((ConnectingDetail, label_body(asset_server, "", 13.0, TEXT_DIM)));
            c.spawn((
                ConnectingButtons,
                Node {
                    column_gap: Val::Px(12.0),
                    margin: UiRect::top(Val::Px(16.0)),
                    display: Display::None,
                    ..default()
                },
            ))
            .with_children(|row| {
                spawn_button_hud(row, asset_server, "RETRY NOW", 20.0, CoverBtn::Retry, ROW, ROW_HOVER, TEXT, UiSound::MENU);
                spawn_button_hud(
                    row,
                    asset_server,
                    "CONTINUE OFFLINE",
                    20.0,
                    CoverBtn::Offline,
                    ROW,
                    ROW_HOVER,
                    TEXT,
                    UiSound::MENU_BACK,
                );
            });
        });
}

#[allow(clippy::type_complexity)]
fn update_cover(
    link: Res<ServerLink>,
    wake: Res<ServerWake>,
    time: Res<Time>,
    mut title: Query<(&mut Text, &mut TextColor), With<ConnectingTitle>>,
    mut status: Query<(&mut Text, &mut TextColor), (With<ConnectingStatus>, Without<ConnectingTitle>)>,
    mut detail: Query<&mut Text, (With<ConnectingDetail>, Without<ConnectingStatus>, Without<ConnectingTitle>)>,
    mut buttons: Query<&mut Node, With<ConnectingButtons>>,
) {
    let waking = wake.waking_for.filter(|&t| t >= WAKING_SHOWN_AFTER_SECS);

    // A slow pulse on the title while we wait, like the loading screen's.
    let pulse = 0.55 + 0.35 * (time.elapsed_secs() * 3.0).sin();
    let heading = if waking.is_some() { "WAKING SERVER" } else { "CONNECTING" };
    for (mut t, mut c) in &mut title {
        if t.0 != heading {
            t.0 = heading.to_string();
        }
        c.0 = TEXT.with_alpha(pulse);
    }

    let (line, color) = match (waking, &link.last_error, link.retry_in) {
        (Some(secs), ..) => (
            format!("The server was asleep. Starting it up, this takes a few seconds… {}s", secs as u32),
            TEXT_DIM,
        ),
        (None, Some(err), Some(left)) => (format!("{err} Retrying in {}s…", left.ceil().max(1.0) as u32), ACCENT),
        (None, Some(err), None) => (format!("{err} Trying again…"), ACCENT),
        (None, None, _) => ("Reaching the server…".to_string(), TEXT_DIM),
    };
    for (mut t, mut c) in &mut status {
        if t.0 != line {
            t.0 = line.clone();
        }
        c.0 = color;
    }

    let addr = link.addr.map(|a| a.to_string()).unwrap_or_default();
    let info = if link.attempt > 1 {
        format!("{addr}  ·  attempt {}", link.attempt)
    } else {
        addr
    };
    for mut t in &mut detail {
        if t.0 != info {
            t.0 = info.clone();
        }
    }

    let display = if link.last_error.is_some() { Display::Flex } else { Display::None };
    for mut n in &mut buttons {
        if n.display != display {
            n.display = display;
        }
    }
}

fn cover_clicks(
    q: Query<(&Interaction, &CoverBtn), Changed<Interaction>>,
    mut link: ResMut<ServerLink>,
    mut offline: ResMut<Offline>,
) {
    for (interaction, btn) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn {
            CoverBtn::Retry => link.retry_now(),
            CoverBtn::Offline => offline.0 = true,
        }
    }
}

/// Esc / the mouse back button on the cover: CONTINUE OFFLINE (and nothing
/// else — the settings menu doesn't open behind it).
fn connecting_back(
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    menu: Res<Menu>,
    link: Res<ServerLink>,
    covers: Query<(), With<ConnectingCover>>,
    mut offline: ResMut<Offline>,
) {
    if covers.is_empty() || menu.rebinding.is_some() {
        return;
    }
    if mouse.just_pressed(MouseButton::Back) || keys.just_pressed(KeyCode::Escape) {
        mouse.clear_just_pressed(MouseButton::Back);
        keys.clear_just_pressed(KeyCode::Escape);
        // (Before an attempt has failed there's no CONTINUE OFFLINE button
        // to stand in for yet.)
        if link.last_error.is_some() {
            offline.0 = true;
        }
    }
}
