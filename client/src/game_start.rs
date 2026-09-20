//! The "waiting for party" screen shown right after a lobby game starts,
//! until every member's client has finished loading the match's assets
//! (currently just the map — see `environment::map::MapLoadState`). Keeps
//! nobody from seeing the (possibly still-loading, pop-in-prone) game world
//! before it's actually ready.
//!
//! Reuses [`menu::Screen::LoadingGame`] to freeze gameplay input the same way
//! the pause menu does, but this module owns the actual overlay content
//! (map/mode header + a per-member loading/ready row) since it needs live
//! [`shared::Lobby`] data `menu.rs` doesn't otherwise touch.

use bevy::prelude::*;
use lightyear::prelude::*;

use crate::menu::{Menu, Screen};
use crate::net::GameClient;
use crate::ui::{label, label_hud, overlay_root, ACCENT, PANEL_SOLID, TEXT, TEXT_DIM, TRACK};
use crate::{map_ready, AppState, CurrentMap, MapLoadState};

pub struct GameStartPlugin;

impl Plugin for GameStartPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SentReady>()
            .add_systems(OnEnter(AppState::InGame), enter_loading_gate)
            .add_systems(OnExit(AppState::InGame), exit_loading_gate)
            .add_systems(
                Update,
                (send_assets_ready, close_when_ready, rebuild)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Component)]
struct LoadingGateUi;

/// Whether this client has already sent `shared::AssetsReady` for the
/// current game, so it isn't sent again every frame once the map's loaded.
/// Reset on every fresh `OnEnter(AppState::InGame)`.
#[derive(Resource, Default)]
struct SentReady(bool);

fn local_peer(local: &Query<&LocalId, With<GameClient>>) -> Option<PeerId> {
    local.iter().next().map(|l| l.0)
}

/// Safety net: if this screen is somehow still up when `InGame` is left (a
/// disconnect mid-load, say — there's no normal way to back out of it), drop
/// it rather than leaving `Menu` permanently reporting "a menu is open" to
/// whatever reads that once we're back at the main menu. `LoadingGateUi`
/// itself is already `StateScoped` and needs no cleanup here.
fn exit_loading_gate(mut menu: ResMut<Menu>) {
    if menu.screen == Screen::LoadingGame {
        menu.screen = Screen::None;
    }
}

/// Shows the screen if we just entered a *started lobby* game.
fn enter_loading_gate(
    mut menu: ResMut<Menu>,
    mut sent: ResMut<SentReady>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    sent.0 = false;
    let Some(me) = local_peer(&local) else { return };
    if lobbies.iter().any(|l| l.has(me)) {
        menu.screen = Screen::LoadingGame;
    }
}

/// Reports this client's own load as soon as it's done — once per game.
fn send_assets_ready(
    map: Res<MapLoadState>,
    current: Res<CurrentMap>,
    mut sent: ResMut<SentReady>,
    mut sender: Query<&mut TriggerSender<shared::AssetsReady>, With<GameClient>>,
) {
    if sent.0 || !map_ready(&map, current.0) {
        return;
    }
    let Ok(mut s) = sender.single_mut() else {
        return;
    };
    s.trigger::<shared::LobbyChannel>(shared::AssetsReady);
    sent.0 = true;
}

/// Once every party member (per the replicated `Lobby`) is `loaded`, drop
/// back to normal gameplay.
fn close_when_ready(
    mut menu: ResMut<Menu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    if menu.screen != Screen::LoadingGame {
        return;
    }
    let Some(me) = local_peer(&local) else { return };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me)) else {
        return;
    };
    if lobby.members.iter().all(|m| m.loaded) {
        menu.screen = Screen::None;
    }
}

/// Rebuilds the overlay whenever it's freshly shown, hidden, or *our own*
/// lobby changed (a member's ready status flipped) while it's up — other
/// lobbies elsewhere on the server keep replicating the whole time we're
/// in-game, so this only reacts to the one we're actually in.
fn rebuild(
    mut commands: Commands,
    menu: Res<Menu>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<(Entity, &shared::Lobby)>,
    changed_lobbies: Query<(), Changed<shared::Lobby>>,
    existing: Query<Entity, With<LoadingGateUi>>,
) {
    if menu.screen != Screen::LoadingGame {
        for e in &existing {
            commands.entity(e).despawn();
        }
        return;
    }
    let Some(me) = local_peer(&local) else { return };
    let Some((lobby_entity, lobby)) = lobbies.iter().find(|(_, l)| l.has(me)) else {
        return;
    };
    if !existing.is_empty() && !changed_lobbies.contains(lobby_entity) {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }

    commands
        .spawn((
            LoadingGateUi,
            GlobalZIndex(50),
            StateScoped(AppState::InGame),
            overlay_root(true),
        ))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Px(420.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(32.0)),
                row_gap: Val::Px(14.0),
                ..default()
            })
            .with_children(|card| {
                card.spawn(label_hud(&asset_server, lobby.map.label(), 32.0, TEXT));
                card.spawn(label(lobby.mode.label(), 15.0, TEXT_DIM));
                card.spawn((
                    Node {
                        width: Val::Px(90.0),
                        height: Val::Px(4.0),
                        margin: UiRect::vertical(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                ));
                card.spawn(label("LOADING…", 14.0, TEXT_DIM));

                card.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(14.0)),
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(PANEL_SOLID),
                    BorderRadius::all(Val::Px(8.0)),
                ))
                .with_children(|panel| {
                    for member in &lobby.members {
                        let (status, color) = if member.loaded {
                            ("READY", ACCENT)
                        } else {
                            ("LOADING…", TEXT_DIM)
                        };
                        let name_color = if member.peer == me { ACCENT } else { TEXT };
                        panel
                            .spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Row,
                                    justify_content: JustifyContent::SpaceBetween,
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                                    ..default()
                                },
                                BackgroundColor(TRACK),
                                BorderRadius::all(Val::Px(6.0)),
                            ))
                            .with_children(|row| {
                                row.spawn(label(member.name.clone(), 16.0, name_color));
                                row.spawn(label(status, 14.0, color));
                            });
                    }
                });
            });
        });
}
