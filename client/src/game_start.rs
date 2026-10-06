//! The "waiting for party" screen shown right after a lobby game starts,
//! until every member's client has finished loading the match's assets
//! (currently just the map — see `environment::map::MapLoadState`). Keeps
//! nobody from seeing the (possibly still-loading, pop-in-prone) game world
//! before it's actually ready.
//!
//! "Loaded" means standing on solid ground, not just the map's scene being
//! spawned: its collision is built from that scene a step later
//! (`AsyncSceneCollider`), and only then can the player's ground ray
//! (`player::apply_gravity`) find it. So a client only reports ready once
//! (1) the map's in, (2) the server's start spot for us has arrived and
//! been applied (`net::PendingRespawn`; every mode but a `Freestyle` game
//! on a map without hand-placed spawns has one —
//! `shared::spawns::has_start_spawn`), and (3) it's put us on the map's
//! floor nearest that spot ([`settle_on_ground`] — a start spot is only
//! *meant* to be at ground level, and can sit a little under the terrain
//! or over a drop). Until then gameplay's frozen, so nobody falls. Should
//! no floor turn up within [`GROUND_WAIT_SECS`], it reports ready anyway
//! rather than hold the party forever.
//!
//! Reuses [`menu::Screen::LoadingGame`] to freeze gameplay input the same way
//! the pause menu does, but this module owns the actual overlay content
//! (map/mode header + a per-member loading/ready row) since it needs live
//! [`shared::Lobby`] data `menu.rs` doesn't otherwise touch.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use lightyear::prelude::*;

use crate::menu::{Menu, Screen};
use crate::net::GameClient;
use crate::ui::{label, label_hud, overlay_root, ACCENT, PANEL_SOLID, TEXT, TEXT_DIM, TRACK};
use crate::{map_ready, AppState, CurrentMap, MapLoadState, MapModel, Player, PlayerPhysics, EYE_HEIGHT};

/// How far (m) above and below the start spot to look for its floor.
const FLOOR_SEARCH_UP: f32 = 15.0;
const FLOOR_SEARCH_DOWN: f32 = 40.0;
/// Most surfaces looked at along that line.
const FLOOR_SEARCH_HITS: usize = 16;

/// How long (s) after the map's in to keep waiting for ground under our
/// spawn before reporting ready anyway (a spawn over nothing — no point
/// holding the whole party forever).
const GROUND_WAIT_SECS: f32 = 10.0;

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
/// current game, so it isn't sent again every frame once the map's loaded,
/// and how long (s) it's been waiting for ground under its spawn since the
/// map came in. Reset on every fresh `OnEnter(AppState::InGame)`.
#[derive(Resource, Default)]
struct SentReady {
    sent: bool,
    ground_wait: f32,
}

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
    mut pending_spawn: ResMut<crate::net::PendingRespawn>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    *sent = SentReady::default();
    pending_spawn.new_game();
    let Some(me) = local_peer(&local) else { return };
    if lobbies.iter().any(|l| l.has(me)) {
        menu.screen = Screen::LoadingGame;
    }
}

/// Reports this client's own load as soon as it's done — once per game: the
/// map's in, our start spot has been applied, and we've been put on its
/// floor (see the module docs).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn send_assets_ready(
    time: Res<Time>,
    map: Res<MapLoadState>,
    current: Res<CurrentMap>,
    pending_spawn: Res<crate::net::PendingRespawn>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    rapier: ReadRapierContext,
    mut player: Query<(&mut Transform, &mut PlayerPhysics), With<Player>>,
    map_models: Query<(), With<MapModel>>,
    parents: Query<&ChildOf>,
    mut sent: ResMut<SentReady>,
    mut sender: Query<&mut TriggerSender<shared::AssetsReady>, With<GameClient>>,
) {
    if sent.sent || !map_ready(&map, current.0) {
        return;
    }
    let Some(me) = local_peer(&local) else { return };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me) && l.started) else {
        return;
    };
    sent.ground_wait += time.delta_secs();
    let timed_out = sent.ground_wait >= GROUND_WAIT_SECS;
    if !timed_out {
        // The server's start spot first (or we'd settle wherever the rig
        // happened to be).
        if shared::spawns::has_start_spawn(lobby.mode, lobby.map) && !pending_spawn.start_applied() {
            return;
        }
        let Ok((mut transform, mut physics)) = player.single_mut() else {
            return;
        };
        let Ok(rapier) = rapier.single() else { return };
        // Only the map's own collision counts (not a perk machine's box).
        let on_map = |mut e: Entity| loop {
            if map_models.contains(e) {
                return true;
            }
            match parents.get(e) {
                Ok(parent) => e = parent.parent(),
                Err(_) => return false,
            }
        };
        let feet = transform.translation - Vec3::Y * EYE_HEIGHT;
        let Some(floor) = settle_on_ground(feet, |from, max| {
            rapier
                .cast_ray_and_get_normal(from, Vec3::NEG_Y, max, true, QueryFilter::default().predicate(&on_map))
                .map(|(_, hit)| (hit.time_of_impact, hit.normal.y))
        }) else {
            return;
        };
        transform.translation.y = floor + EYE_HEIGHT;
        *physics = PlayerPhysics {
            grounded: true,
            ..default()
        };
    }
    let Ok(mut s) = sender.single_mut() else {
        return;
    };
    if timed_out {
        warn!("no floor under our start spot after {GROUND_WAIT_SECS}s — reporting ready anyway");
    } else {
        info!("map loaded and standing on its floor — ready");
    }
    s.trigger::<shared::LobbyChannel>(shared::AssetsReady);
    sent.sent = true;
}

/// The floor (its height) nearest feet at `feet`: every upward-facing
/// surface on the vertical line through them, from [`FLOOR_SEARCH_UP`] above
/// to [`FLOOR_SEARCH_DOWN`] below, and whichever's closest in height —
/// `cast(from, max)` casts down from `from` up to `max` m, giving the
/// distance to the hit and its normal's `y`. `None` while there's no floor
/// there (the map's collision isn't in yet).
fn settle_on_ground(feet: Vec3, cast: impl Fn(Vec3, f32) -> Option<(f32, f32)>) -> Option<f32> {
    let bottom = feet.y - FLOOR_SEARCH_DOWN;
    let mut from = feet + Vec3::Y * FLOOR_SEARCH_UP;
    let mut best: Option<f32> = None;
    for _ in 0..FLOOR_SEARCH_HITS {
        let Some((toi, normal_y)) = cast(from, from.y - bottom) else {
            break;
        };
        let y = from.y - toi;
        if normal_y > 0.5 && best.is_none_or(|b| (y - feet.y).abs() < (b - feet.y).abs()) {
            best = Some(y);
        }
        // On past this surface.
        from.y = y - 0.01;
        if from.y <= bottom {
            break;
        }
    }
    best
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
