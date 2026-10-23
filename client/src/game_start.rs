//! The "waiting for party" screen shown right after a lobby game starts,
//! until every member's client has finished loading the match's assets
//! (the map — see `environment::map::MapLoadState` — and the sounds asked
//! for so far, `audio_engine::PendingSounds`, or they'd play late). Keeps
//! nobody from seeing the (possibly still-loading, pop-in-prone) game world
//! before it's actually ready.
//!
//! "Loaded" means standing on solid ground, not just the map's scene being
//! spawned: its collision is built from that scene a step later
//! (`AsyncSceneCollider`), and only then can the player's ground ray
//! (`player::apply_gravity`) find it. So a client only reports ready once
//! (1) the map and its sounds are in, (2) the server's start spot for us
//! has arrived and been applied (`net::PendingRespawn`; every mode but a
//! `Freestyle` game on a map without hand-placed spawns has one —
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
//!
//! A `Zombies` game gets a Call of Duty Zombies style screen instead: the
//! zombies backdrop (`menu_backdrop::zombies_background`), the map's name
//! over three boxes telling the player what they're there to do
//! ([`ZOMBIES_OBJECTIVES`]), and the party's loading rows tucked into the
//! bottom-right corner.
//!
//! Two steps, the server waiting on both: everyone's assets first (each
//! client's `shared::AssetsReady` — `LobbyMember::assets`, what the rows
//! show), then everyone's intro (`shared::IntroFinished` — `loaded`, which
//! starts the game). While anyone's still loading, a `Zombies` game shows the
//! zombies backdrop, the map's name and the party's rows. Once they're all
//! in, its intro plays for everyone at once ([`ZombiesIntro`]): for
//! [`INTRO_SECS`] the map's own art (`menu_backdrop::intro_art` — Der
//! Tempel's screenshot, the zombies art elsewhere) pans slowly across under
//! the backdrop's horror layers, rising out of black, the map's name huge
//! and white at the top, the three boxes ([`ZOMBIES_OBJECTIVES`]) fading in
//! after [`INTRO_BOXES_AT`], to the intro music (fading out as it ends). It's
//! skipped with debug mode on — unless anyone else is playing too — and
//! every other mode has none: those report `IntroFinished` straight away.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use bevy::time::Real;

use crate::audio_engine::prelude::*;
use lightyear::prelude::*;

use crate::menu::{Menu, Screen};
use crate::menu_backdrop::{intro_art, intro_background, zombies_background, IntroPan};
use crate::net::GameClient;
use crate::ui::{label, label_hud, overlay_root, ACCENT, PANEL_SOLID, TEXT, TEXT_DIM, TRACK};
use crate::{map_ready, AppState, CurrentMap, MapLoadState, MapModel, Player, PlayerPhysics, EYE_HEIGHT};

/// How far (m) above and below the start spot to look for its floor.
const FLOOR_SEARCH_UP: f32 = 15.0;
const FLOOR_SEARCH_DOWN: f32 = 40.0;
/// Most surfaces looked at along that line.
const FLOOR_SEARCH_HITS: usize = 16;

/// The `Zombies` intro's three boxes: a title and a line on each.
const ZOMBIES_OBJECTIVES: [(&str, &str); 3] = [
    (
        "SURVIVE",
        "Hold out round after round. Revive your downed teammates — if everyone goes down, it's over.",
    ),
    (
        "TURN ON THE POWER",
        "Find the power switch — nothing sells until it's on: no perks, no Pack-a-Punch.",
    ),
    (
        "PACK-A-PUNCH",
        "Earn points from kills, then feed your weapon to the machine for something far deadlier.",
    ),
];
/// Their accent: blood red.
const ZOMBIES_RED: Color = Color::srgb(0.78, 0.08, 0.06);

/// How long (s) the `Zombies` intro lasts (we're not ready before it's
/// over)...
const INTRO_SECS: f32 = 5.0;
/// ...when its three boxes start fading in, and how long (s) they take...
const INTRO_BOXES_AT: f32 = 2.0;
const INTRO_BOXES_FADE_SECS: f32 = 0.8;
/// ...and how long (s) its music takes to fade out as it ends.
const INTRO_MUSIC_FADE_SECS: f32 = 1.8;

/// How long (s) after the map's in to keep waiting for ground under our
/// spawn before reporting ready anyway (a spawn over nothing — no point
/// holding the whole party forever).
const GROUND_WAIT_SECS: f32 = 10.0;

pub struct GameStartPlugin;

impl Plugin for GameStartPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SentReady>()
            .init_resource::<ZombiesIntro>()
            .add_systems(OnEnter(AppState::InGame), enter_loading_gate)
            .add_systems(OnExit(AppState::InGame), exit_loading_gate)
            .add_systems(
                Update,
                (send_assets_ready, run_intro, close_when_ready, rebuild, fade_intro_boxes, fade_intro_music)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Component)]
struct LoadingGateUi;

/// This game's `Zombies` intro: whether it plays one (decided on entering
/// the game), and since when (`Time<Real>` seconds — once everyone's
/// loaded). Cleared on entering and leaving a game.
#[derive(Resource, Default)]
struct ZombiesIntro {
    wanted: bool,
    started: Option<f32>,
}

impl ZombiesIntro {
    /// Seconds into it, if there is one.
    fn age(&self, now: f32) -> Option<f32> {
        self.started.map(|s| now - s)
    }

    /// It's still on — not ready to play yet.
    fn holds(&self, now: f32) -> bool {
        self.age(now).is_some_and(|a| a < INTRO_SECS)
    }

    /// How far its boxes have faded in (0..1; 1 with no intro).
    fn boxes(&self, now: f32) -> f32 {
        self.age(now)
            .map_or(1.0, |a| ((a - INTRO_BOXES_AT) / INTRO_BOXES_FADE_SECS).clamp(0.0, 1.0))
    }
}

/// On an intro box's parts: their full colours, faded in with the boxes
/// ([`fade_intro_boxes`]).
#[derive(Component, Clone, Copy)]
struct IntroFade {
    bg: Option<Color>,
    border: Option<Color>,
    text: Option<Color>,
}

/// The intro's music.
#[derive(Component)]
struct IntroMusic;

/// Whether this client has already sent `shared::AssetsReady` for the
/// current game, so it isn't sent again every frame once the map's loaded,
/// and how long (s) it's been waiting for ground under its spawn since the
/// map came in. Reset on every fresh `OnEnter(AppState::InGame)`.
#[derive(Resource, Default)]
struct SentReady {
    sent: bool,
    ground_wait: f32,
    /// `shared::IntroFinished` sent, too.
    intro_sent: bool,
}

fn local_peer(local: &Query<&LocalId, With<GameClient>>) -> Option<PeerId> {
    local.iter().next().map(|l| l.0)
}

/// Safety net: if this screen is somehow still up when `InGame` is left (a
/// disconnect mid-load, say — there's no normal way to back out of it), drop
/// it rather than leaving `Menu` permanently reporting "a menu is open" to
/// whatever reads that once we're back at the main menu. `LoadingGateUi`
/// itself is already `StateScoped` and needs no cleanup here.
fn exit_loading_gate(mut menu: ResMut<Menu>, mut intro: ResMut<ZombiesIntro>) {
    if menu.screen == Screen::LoadingGame {
        menu.screen = Screen::None;
    }
    *intro = ZombiesIntro::default();
}

/// Shows the screen if we just entered a *started lobby* game — and, for a
/// `Zombies` one, starts its intro and music unless debug mode's on with
/// nobody else playing.
#[allow(clippy::too_many_arguments)]
fn enter_loading_gate(
    settings: Res<crate::settings::Settings>,
    mut menu: ResMut<Menu>,
    mut sent: ResMut<SentReady>,
    mut intro: ResMut<ZombiesIntro>,
    mut pending_spawn: ResMut<crate::net::PendingRespawn>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    *sent = SentReady::default();
    *intro = ZombiesIntro::default();
    pending_spawn.new_game();
    let Some(me) = local_peer(&local) else { return };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me)) else {
        return;
    };
    menu.screen = Screen::LoadingGame;
    let others = lobby.real_peers().len() > 1;
    intro.wanted = lobby.mode == shared::GameMode::Zombies && (!settings.debug_mode || others);
}

/// Once every member's assets are in: start the intro (and its music), if
/// this game has one — and report it played through ([`INTRO_SECS`] later,
/// or at once without one).
#[allow(clippy::too_many_arguments)]
fn run_intro(
    time: Res<Time<Real>>,
    menu: Res<Menu>,
    sounds: Res<crate::GameSounds>,
    mut intro: ResMut<ZombiesIntro>,
    mut sent: ResMut<SentReady>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut sender: Query<&mut TriggerSender<shared::IntroFinished>, With<GameClient>>,
    mut commands: Commands,
) {
    if menu.screen != Screen::LoadingGame || sent.intro_sent {
        return;
    }
    let Some(me) = local_peer(&local) else { return };
    let Some(lobby) = lobbies.iter().find(|l| l.has(me) && l.started) else {
        return;
    };
    if !lobby.members.iter().all(|m| m.assets) {
        return;
    }
    let now = time.elapsed_secs();
    if intro.wanted && intro.started.is_none() {
        intro.started = Some(now);
        commands.spawn((
            StateScoped(AppState::InGame),
            IntroMusic,
            AudioPlayer::new(sounds.zombies_intro.clone()),
            PlaybackSettings::DESPAWN,
        ));
    }
    if intro.holds(now) {
        return;
    }
    let Ok(mut s) = sender.single_mut() else { return };
    s.trigger::<shared::LobbyChannel>(shared::IntroFinished);
    sent.intro_sent = true;
}

/// Fade the intro's music out over its last [`INTRO_MUSIC_FADE_SECS`], and
/// stop it once it's silent.
fn fade_intro_music(
    time: Res<Time<Real>>,
    intro: Res<ZombiesIntro>,
    mut music: Query<(Entity, Option<&mut crate::audio_engine::AudioSink>), With<IntroMusic>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (e, sink) in &mut music {
        let Some(age) = intro.age(now) else {
            commands.entity(e).despawn();
            continue;
        };
        let left = INTRO_SECS - age;
        let volume = (left / INTRO_MUSIC_FADE_SECS).clamp(0.0, 1.0);
        if volume <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        if let Some(mut sink) = sink {
            if sink.volume().to_linear() != volume {
                sink.set_volume(crate::audio_engine::Volume::Linear(volume));
            }
        }
    }
}

/// Fade the intro's boxes in after [`INTRO_BOXES_AT`].
fn fade_intro_boxes(
    time: Res<Time<Real>>,
    intro: Res<ZombiesIntro>,
    mut parts: Query<(&IntroFade, Option<&mut BackgroundColor>, Option<&mut BorderColor>, Option<&mut TextColor>)>,
) {
    let f = intro.boxes(time.elapsed_secs());
    for (fade, bg, border, text) in &mut parts {
        if let (Some(c), Some(mut bg)) = (fade.bg, bg) {
            bg.0 = c.with_alpha(c.alpha() * f);
        }
        if let (Some(c), Some(mut border)) = (fade.border, border) {
            border.0 = c.with_alpha(c.alpha() * f);
        }
        if let (Some(c), Some(mut text)) = (fade.text, text) {
            text.0 = c.with_alpha(c.alpha() * f);
        }
    }
}

/// Reports this client's own load as soon as it's done — once per game: the
/// map and the sounds are in, our start spot has been applied, and we've
/// been put on its floor (see the module docs).
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
    (asset_server, mut sounds): (Res<AssetServer>, ResMut<crate::audio_engine::PendingSounds>),
) {
    if sent.sent || !map_ready(&map, current.0) || !sounds.all_loaded(&asset_server) {
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
#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    menu: Res<Menu>,
    time: Res<Time<Real>>,
    intro: Res<ZombiesIntro>,
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
    if !existing.is_empty() && !changed_lobbies.contains(lobby_entity) && !intro.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }

    if lobby.mode == shared::GameMode::Zombies {
        match intro.started {
            Some(started) => build_zombies_cinematic(&mut commands, &asset_server, lobby, started, intro.boxes(time.elapsed_secs())),
            None => build_zombies_loading(&mut commands, &asset_server, lobby, me),
        }
    } else {
        build_party_card(&mut commands, &asset_server, lobby, me);
    }
}

/// One row per party member: their name, and whether they're loaded yet.
fn member_rows(panel: &mut ChildSpawnerCommands, lobby: &shared::Lobby, me: PeerId) {
    for member in &lobby.members {
        let (status, color) = if member.assets {
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
                    column_gap: Val::Px(16.0),
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
}

/// Every mode but `Zombies`: the map and mode over the party's rows, centred.
fn build_party_card(commands: &mut Commands, asset_server: &AssetServer, lobby: &shared::Lobby, me: PeerId) {
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
                card.spawn(label_hud(asset_server, lobby.map.label(), 32.0, TEXT));
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
                .with_children(|panel| member_rows(panel, lobby, me));
            });
        });
}

/// `Zombies`, while anyone's still loading: the scary backdrop, the map's
/// name and the party's rows (who's loaded) — the objectives wait for the
/// intro.
fn build_zombies_loading(commands: &mut Commands, asset_server: &AssetServer, lobby: &shared::Lobby, me: PeerId) {
    commands
        .spawn((
            LoadingGateUi,
            GlobalZIndex(50),
            StateScoped(AppState::InGame),
            overlay_root(true),
        ))
        .with_children(|root| {
            zombies_background(root, asset_server);

            root.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|center| {
                center.spawn(label_hud(asset_server, lobby.map.label(), 64.0, TEXT));
                center.spawn(label_hud(asset_server, lobby.mode.label(), 20.0, ZOMBIES_RED));
                center
                    .spawn((
                        Node {
                            width: Val::Px(400.0),
                            flex_direction: FlexDirection::Column,
                            margin: UiRect::top(Val::Px(36.0)),
                            padding: UiRect::all(Val::Px(14.0)),
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::BLACK.with_alpha(0.6)),
                        BorderRadius::all(Val::Px(8.0)),
                    ))
                    .with_children(|panel| {
                        panel.spawn(label("WAITING FOR PLAYERS…", 14.0, TEXT_DIM));
                        member_rows(panel, lobby, me);
                    });
            });
        });
}

/// One of the three objective boxes ([`ZOMBIES_OBJECTIVES`]) — faded in to
/// `fade` (0..1), and kept fading in by [`fade_intro_boxes`].
fn objective_box(boxes: &mut ChildSpawnerCommands, asset_server: &AssetServer, n: usize, title: &str, line: &str, fade: f32) {
    let bg = Color::srgba(0.02, 0.0, 0.0, 0.72);
    let border = ZOMBIES_RED.with_alpha(0.55);
    let faded = |c: Color| c.with_alpha(c.alpha() * fade);
    let mut card = boxes.spawn((
        Node {
            width: Val::Px(300.0),
            min_height: Val::Px(190.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            padding: UiRect::all(Val::Px(22.0)),
            border: UiRect {
                top: Val::Px(3.0),
                ..UiRect::all(Val::Px(1.0))
            },
            ..default()
        },
        BackgroundColor(faded(bg)),
        BorderColor(faded(border)),
        BorderRadius::all(Val::Px(4.0)),
    ));
    card.insert(IntroFade { bg: Some(bg), border: Some(border), text: None });
    card.with_children(|card| {
        for (text, size, color, hud) in [
            (format!("{:02}", n + 1), 16.0, ZOMBIES_RED, true),
            (title.to_string(), 28.0, TEXT, true),
            (line.to_string(), 15.0, TEXT_DIM, false),
        ] {
            let mut e = if hud {
                card.spawn(label_hud(asset_server, text, size, faded(color)))
            } else {
                card.spawn(label(text, size, faded(color)))
            };
            e.insert(IntroFade { bg: None, border: None, text: Some(color) });
        }
    });
}

/// `Zombies` with its intro playing (everyone's loaded): the map's intro art
/// panning across under the horror layers
/// (`menu_backdrop::intro_background`), the map's name huge and white at
/// the top, and the three boxes fading in (`boxes` — how far they are
/// already).
fn build_zombies_cinematic(commands: &mut Commands, asset_server: &AssetServer, lobby: &shared::Lobby, started: f32, boxes: f32) {
    let (art, size) = intro_art(lobby.map);
    commands
        .spawn((
            LoadingGateUi,
            GlobalZIndex(50),
            StateScoped(AppState::InGame),
            overlay_root(true),
        ))
        .with_children(|root| {
            intro_background(root, asset_server, art, IntroPan { size, started, secs: INTRO_SECS });

            // The map's name, huge and white at the top.
            root.spawn(Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(7.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|top| {
                top.spawn((label_hud(asset_server, lobby.map.label(), 120.0, Color::WHITE), TextShadow::default()));
                top.spawn(label_hud(asset_server, lobby.mode.label(), 24.0, ZOMBIES_RED));
            });

            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(24.0),
                margin: UiRect::top(Val::Px(120.0)),
                ..default()
            })
            .with_children(|row| {
                for (n, (title, line)) in ZOMBIES_OBJECTIVES.iter().enumerate() {
                    objective_box(row, asset_server, n, title, line, boxes);
                }
            });
        });
}
