//! `SearchAndDestroy` (`shared::search_destroy`) on our side: the HUD —
//! the round score and clock (the fuse once the bomb's planted) at the top,
//! the round's banner (its number and our side before it starts, won or
//! lost and how once it's over), what holding interact does here (plant,
//! defuse) and its bar, and the A / B and dropped-bomb markers — the sites'
//! rings and the bomb on the ground, beeping once it's planted and blowing
//! up when the server says ([`shared::SndBombExploded`]).
//!
//! Holding interact goes to the server with our input
//! ([`PlayerInput::snd_use`]); it decides what that does. Before a round
//! we're held at our spawn ([`not_frozen`], on the movement and the
//! weapon), and once we're killed we're out until the next — watching a
//! teammate (`revive::spectate`) — with no input ([`SndLocal::dead`],
//! `menu::game_active`).
//!
//! Everything here is `StateScoped(InGame)`, and [`SndLocal`] is reset on
//! the way in and out, so nothing carries into the next game.

use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy::window::PrimaryWindow;
use lightyear::prelude::input::client::InputSet;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::{LocalId, MessageReceiver, PeerId};
use shared::search_destroy::{
    layout, site_at, within, Bomb, Phase, Site, Team, DEFUSE_RADIUS, DEFUSE_SECS, FUSE_SECS, PLANT_SECS, SITE_RADIUS,
};
use shared::{GameMode, Lobby, PlayerInput};

use crate::audio_engine::prelude::*;
use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::player::WorldModelCamera;
use crate::zombies_hud::my_lobby;
use crate::{killcam, menu, AppState, Player, EYE_HEIGHT, HUD_FONT};

/// The planted bomb's beep.
const BEEP_PATH: &str = "audio/combat/hit_marker.mp3";
/// Seconds between beeps with the whole fuse left, and right at the end.
const BEEP_SLOW_SECS: f32 = 1.0;
const BEEP_FAST_SECS: f32 = 0.15;
/// The banners' greens and reds.
const WON: Color = Color::srgb(0.35, 0.85, 0.4);
const LOST: Color = Color::srgb(0.9, 0.25, 0.22);
/// The bomb's (and a planted site's) red.
const BOMB_RED: Color = Color::srgb(1.0, 0.18, 0.12);
const BAR_W: f32 = 320.0;
const MARKER_W: f32 = 54.0;
/// How high (m) over a site's middle its marker floats.
const SITE_MARKER_HEIGHT: f32 = 2.2;
/// The bomb model's size (m).
const BOMB_SIZE: Vec3 = Vec3::new(0.42, 0.18, 0.3);

pub(crate) struct SearchDestroyPlugin;

impl Plugin for SearchDestroyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SndLocal>()
            .add_systems(Startup, load_sounds)
            .add_systems(OnEnter(AppState::InGame), (reset_local, spawn_hud))
            .add_systems(OnExit(AppState::InGame), reset_local)
            // Before anything's run conditions read it this frame.
            .add_systems(PreUpdate, track_local.after(bevy::input::InputSystem))
            .add_systems(
                FixedPreUpdate,
                write_snd_input
                    .in_set(InputSet::WriteClientInputs)
                    .after(crate::net::write_input),
            )
            .add_systems(
                Update,
                (
                    update_scoreboard,
                    update_banner,
                    update_prompt,
                    update_markers,
                    sync_world,
                    beep_and_blow_up,
                )
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Where we stand in a running game — read by run conditions, the input
/// and the HUD.
#[derive(Resource, Default)]
pub(crate) struct SndLocal {
    /// Killed this round: out until the next.
    pub(crate) dead: bool,
    /// Held at our spawn for the pre-round countdown.
    frozen: bool,
    /// Holding interact, alive and in a round.
    holding: bool,
    /// Who the server last said is planting / defusing, and since when
    /// (`Time` seconds) — the bar's clock.
    using: Option<(PeerId, f32)>,
    /// When the planted bomb beeps next.
    next_beep: f32,
    /// When we were killed (`Time` seconds), while we're out.
    dead_since: Option<f32>,
}

impl SndLocal {
    /// Out long enough that a kill cam that hasn't started by now isn't
    /// coming — time to watch someone (`revive::spectate`).
    pub(crate) fn dead_long_enough(&self, now: f32) -> bool {
        self.dead_since.is_some_and(|t| now - t >= KILLCAM_WAIT_SECS)
    }
}

/// How long (s) after we're killed we wait for our kill cam before
/// watching a teammate anyway.
const KILLCAM_WAIT_SECS: f32 = 3.5;

fn reset_local(mut local: ResMut<SndLocal>) {
    *local = SndLocal::default();
}

/// Run condition: not held at our spawn before a round.
pub(crate) fn not_frozen(local: Res<SndLocal>) -> bool {
    !local.frozen
}

/// Our lobby while it's a running `SearchAndDestroy` game.
pub(crate) fn snd_game<'a>(local: &Query<&LocalId, With<GameClient>>, lobbies: &'a Query<&Lobby>) -> Option<&'a Lobby> {
    my_lobby(local, lobbies).filter(|l| l.started && l.mode == GameMode::SearchAndDestroy)
}

#[allow(clippy::too_many_arguments)]
fn track_local(
    time: Res<Time>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    menu: Res<menu::Menu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut state: ResMut<SndLocal>,
) {
    let me = local.iter().next().map(|l| l.0);
    let Some((lobby, me)) = snd_game(&local, &lobbies).zip(me) else {
        if state.dead || state.frozen || state.holding || state.using.is_some() {
            *state = SndLocal::default();
        }
        return;
    };
    let snd = &lobby.snd;
    let dead = snd.is_dead(me);
    let frozen = snd.round > 0 && snd.phase == Phase::PreRound;
    let holding = !dead && snd.phase == Phase::Live && !menu.is_open() && binds.interact.pressed(&keys, &mouse);
    if state.dead != dead {
        state.dead = dead;
        state.dead_since = dead.then(|| time.elapsed_secs());
    }
    if state.frozen != frozen {
        state.frozen = frozen;
    }
    if state.holding != holding {
        state.holding = holding;
    }
    if state.using.map(|(p, _)| p) != snd.using {
        state.using = snd.using.map(|p| (p, time.elapsed_secs()));
    }
}

fn write_snd_input(local: Res<SndLocal>, mut q: Query<&mut ActionState<PlayerInput>, With<InputMarker<PlayerInput>>>) {
    if let Ok(mut action) = q.single_mut() {
        action.snd_use = local.holding;
    }
}

#[derive(Resource)]
struct SndSounds {
    beep: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(SndSounds {
        beep: asset_server.load(BEEP_PATH),
    });
}

fn team_color(team: Team) -> Color {
    let [r, g, b] = team.rgb();
    Color::srgb(r, g, b)
}

fn clock(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

// --- the HUD -----------------------------------------------------------

/// The score and clock, top centre.
#[derive(Component)]
struct ScoreRoot;
#[derive(Component)]
struct ScoreWins(Team);
#[derive(Component)]
struct ScoreClock;
#[derive(Component)]
struct ScoreInfo;

/// The round's banner, upper middle.
#[derive(Component)]
struct BannerRoot;
#[derive(Component)]
struct BannerTitle;
#[derive(Component)]
struct BannerSub;

/// What holding interact does here, and its bar, low centre.
#[derive(Component)]
struct PromptRoot;
#[derive(Component)]
struct PromptText;
#[derive(Component)]
struct PromptBar;
#[derive(Component)]
struct PromptFill;

/// A marker over a site, or the dropped bomb.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Site(Site),
    Bomb,
}
#[derive(Component)]
struct MarkerText;

fn spawn_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let text = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let shadow = TextShadow {
        offset: Vec2::splat(2.0),
        color: Color::srgba(0.0, 0.0, 0.0, 0.8),
    };
    let centred = |top: Val| Node {
        position_type: PositionType::Absolute,
        top,
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(2.0),
        ..default()
    };

    commands
        .spawn((
            ScoreRoot,
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            centred(Val::Px(crate::hud::COMPASS_HEIGHT + 6.0)),
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(18.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(4.0)),
                ..default()
            })
            .insert((BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)), BorderRadius::all(Val::Px(6.0))))
            .with_children(|row| {
                row.spawn((ScoreWins(Team::Red), Text::new("0"), text(34.0), TextColor(team_color(Team::Red)), shadow));
                row.spawn((ScoreClock, Text::new(""), text(30.0), TextColor::WHITE, shadow));
                row.spawn((ScoreWins(Team::Blue), Text::new("0"), text(34.0), TextColor(team_color(Team::Blue)), shadow));
            });
            col.spawn((ScoreInfo, Text::new(""), text(16.0), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)), shadow));
        });

    commands
        .spawn((BannerRoot, StateScoped(AppState::InGame), GlobalZIndex(6), centred(Val::Percent(24.0)), Visibility::Hidden))
        .with_children(|col| {
            col.spawn((BannerTitle, Text::new(""), text(64.0), TextColor::WHITE, shadow));
            col.spawn((BannerSub, Text::new(""), text(24.0), TextColor::WHITE, shadow));
        });

    commands
        .spawn((PromptRoot, StateScoped(AppState::InGame), GlobalZIndex(5), centred(Val::Percent(60.0)), Visibility::Hidden))
        .with_children(|col| {
            col.spawn((PromptText, Text::new(""), text(22.0), TextColor::WHITE, shadow));
            col.spawn((
                PromptBar,
                Node {
                    width: Val::Px(BAR_W),
                    height: Val::Px(10.0),
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.2)),
                Visibility::Hidden,
            ))
            .with_child((
                PromptFill,
                Node {
                    width: Val::Percent(0.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::WHITE),
            ));
        });

    for marker in [Marker::Site(Site::A), Marker::Site(Site::B), Marker::Bomb] {
        let label = match marker {
            Marker::Site(s) => s.label(),
            Marker::Bomb => "BOMB",
        };
        commands
            .spawn((
                marker,
                StateScoped(AppState::InGame),
                GlobalZIndex(4),
                Node {
                    position_type: PositionType::Absolute,
                    min_width: Val::Px(MARKER_W),
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                BorderColor(Color::WHITE),
                BorderRadius::all(Val::Px(4.0)),
                Visibility::Hidden,
            ))
            .with_child((MarkerText, Text::new(label), text(22.0), TextColor::WHITE, shadow));
    }
}

/// Whether the HUD's up (not behind a menu or the kill cam).
fn hud_up(menu: &menu::Menu, killcam: &killcam::ActiveKillCam) -> bool {
    !menu.is_open() && killcam.0.is_none()
}

fn show(vis: &mut Mut<Visibility>, on: bool) {
    vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
}

fn set_text(text: &mut Text, wanted: String) {
    if text.0 != wanted {
        text.0 = wanted;
    }
}

#[allow(clippy::type_complexity)]
fn update_scoreboard(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut root: Single<&mut Visibility, With<ScoreRoot>>,
    mut wins: Query<(&ScoreWins, &mut Text), (Without<ScoreClock>, Without<ScoreInfo>)>,
    mut clock_text: Single<(&mut Text, &mut TextColor), (With<ScoreClock>, Without<ScoreWins>, Without<ScoreInfo>)>,
    mut info: Single<&mut Text, (With<ScoreInfo>, Without<ScoreWins>, Without<ScoreClock>)>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = snd_game(&local, &lobbies).filter(|l| l.snd.round > 0);
    show(&mut root, lobby.is_some() && hud_up(&menu, &active_killcam));
    let Some(lobby) = lobby else { return };
    let snd = &lobby.snd;
    for (w, mut text) in &mut wins {
        set_text(&mut text, snd.wins_of(w.0).to_string());
    }
    let planted = snd.bomb.planted() && snd.phase == Phase::Live;
    let (text, color) = &mut *clock_text;
    set_text(text, if planted { format!("BOMB {}", clock(snd.secs_left)) } else { clock(snd.secs_left) });
    let want = if planted { BOMB_RED } else { Color::WHITE };
    if color.0 != want {
        color.0 = want;
    }
    let side = me.and_then(|me| lobby.team_of(me)).map(|t| if snd.attacking(t) { "ATTACKING" } else { "DEFENDING" });
    let first_to = lobby.rounds_to_win();
    set_text(
        &mut info,
        match side {
            Some(side) => format!("ROUND {}  ·  {side}  ·  FIRST TO {first_to}", snd.round),
            None => format!("ROUND {}  ·  FIRST TO {first_to}", snd.round),
        },
    );
}

#[allow(clippy::type_complexity)]
fn update_banner(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut root: Single<&mut Visibility, With<BannerRoot>>,
    mut title: Single<(&mut Text, &mut TextColor), (With<BannerTitle>, Without<BannerSub>)>,
    mut sub: Single<&mut Text, (With<BannerSub>, Without<BannerTitle>)>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = snd_game(&local, &lobbies).filter(|l| l.snd.round > 0);
    let mine = lobby.zip(me).and_then(|(l, me)| l.team_of(me));
    let wanted = lobby.and_then(|l| {
        let snd = &l.snd;
        match snd.phase {
            Phase::PreRound => {
                let what = match mine {
                    Some(t) if snd.attacking(t) => "ATTACK — PLANT THE BOMB AT A OR B",
                    Some(_) => "DEFEND — DON'T LET THEM PLANT",
                    None => "",
                };
                Some((format!("ROUND {}", snd.round), Color::WHITE, format!("{what}\nSTARTS IN {}", snd.secs_left)))
            }
            Phase::Over { winner, how } => {
                let (text, color) = match mine {
                    Some(t) if t == winner => ("ROUND WON", WON),
                    Some(_) => ("ROUND LOST", LOST),
                    None => (winner.label(), team_color(winner)),
                };
                Some((text.to_string(), color, how.label().to_string()))
            }
            Phase::Live => None,
        }
    });
    show(&mut root, wanted.is_some() && hud_up(&menu, &active_killcam));
    if let Some((t, color, s)) = wanted {
        let (text, c) = &mut *title;
        set_text(text, t);
        if c.0 != color {
            c.0 = color;
        }
        set_text(&mut sub, s);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_prompt(
    time: Res<Time>,
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    state: Res<SndLocal>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut root: Single<&mut Visibility, (With<PromptRoot>, Without<PromptBar>)>,
    mut bar: Single<&mut Visibility, (With<PromptBar>, Without<PromptRoot>)>,
    mut fill: Single<&mut Node, With<PromptFill>>,
    mut text: Single<&mut Text, With<PromptText>>,
) {
    let me = local.iter().next().map(|l| l.0);
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let key = binds.interact.label().to_uppercase();
    let wanted: Option<(String, Option<f32>)> = snd_game(&local, &lobbies).zip(me).and_then(|(l, me)| {
        let snd = &l.snd;
        if state.dead || snd.phase != Phase::Live {
            return None;
        }
        let attacking = l.team_of(me).is_some_and(|t| snd.attacking(t));
        let using = state.using.filter(|(p, _)| *p == me).map(|(_, since)| time.elapsed_secs() - since);
        match snd.bomb {
            Bomb::Carried(c) if c == me => match (using, site_at(l.map, feet)) {
                (Some(t), _) => Some(("PLANTING THE BOMB".to_string(), Some(t / PLANT_SECS))),
                (None, Some(site)) => Some((format!("HOLD {key} TO PLANT THE BOMB AT {}", site.label()), None)),
                (None, None) => Some(("YOU HAVE THE BOMB — PLANT IT AT A OR B".to_string(), None)),
            },
            Bomb::Planted { at, .. } if !attacking => match using {
                Some(t) => Some(("DEFUSING THE BOMB".to_string(), Some(t / DEFUSE_SECS))),
                None if within(Vec3::from_array(at), feet, DEFUSE_RADIUS) => {
                    Some((format!("HOLD {key} TO DEFUSE THE BOMB"), None))
                }
                None => None,
            },
            _ => None,
        }
    });
    show(&mut root, wanted.is_some() && hud_up(&menu, &active_killcam));
    let Some((wanted, progress)) = wanted else { return };
    set_text(&mut text, wanted);
    show(&mut bar, progress.is_some());
    let width = Val::Percent(progress.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0);
    if fill.width != width {
        fill.width = width;
    }
}

/// The site markers (red for a planted one) and, for the attackers, the
/// dropped bomb's — over the world, or along the screen's edge.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_markers(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    camera: Single<(Entity, &Camera), With<WorldModelCamera>>,
    transforms: TransformHelper,
    window: Single<&Window, With<PrimaryWindow>>,
    mut markers: Query<(&Marker, &mut Node, &mut Visibility, &mut BorderColor)>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = snd_game(&local, &lobbies).filter(|l| l.snd.round > 0);
    let (cam_entity, cam) = camera.into_inner();
    let cam_gt = transforms.compute_global_transform(cam_entity).ok();
    let size = window.size();
    for (marker, mut node, mut vis, mut border) in &mut markers {
        let wanted: Option<(Vec3, Color)> = lobby.and_then(|l| {
            let snd = &l.snd;
            let attacking = me.and_then(|me| l.team_of(me)).is_some_and(|t| snd.attacking(t));
            let sites = layout(l.map);
            match (*marker, snd.bomb) {
                (Marker::Site(s), Bomb::Planted { site, .. }) if site == s => {
                    Some((sites.site(s) + Vec3::Y * SITE_MARKER_HEIGHT, BOMB_RED))
                }
                // (Once it's planted, only that site matters.)
                (Marker::Site(_), Bomb::Planted { .. }) => None,
                (Marker::Site(s), _) => Some((sites.site(s) + Vec3::Y * SITE_MARKER_HEIGHT, Color::WHITE)),
                (Marker::Bomb, Bomb::Dropped(at)) if attacking => Some((Vec3::from_array(at) + Vec3::Y * 0.8, BOMB_RED)),
                (Marker::Bomb, _) => None,
            }
        });
        let screen = wanted.zip(cam_gt).map(|((anchor, color), gt)| {
            let ahead = (anchor - gt.translation()).dot(*gt.forward()) > 0.0;
            let at = match cam.world_to_viewport(&gt, anchor) {
                Ok(p) if ahead => p,
                _ => {
                    let local = gt.compute_matrix().inverse().transform_point3(anchor);
                    let dir = Vec2::new(local.x, -local.y).normalize_or(Vec2::Y);
                    size / 2.0 + dir * size.length()
                }
            };
            (at, color)
        });
        let Some((at, color)) = screen.filter(|_| hud_up(&menu, &active_killcam)) else {
            show(&mut vis, false);
            continue;
        };
        let half = MARKER_W / 2.0;
        let (left, top) = (
            Val::Px(at.x.clamp(half + 8.0, size.x - half - 8.0) - half),
            Val::Px(at.y.clamp(24.0, size.y - 24.0) - 16.0),
        );
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        if border.0 != color {
            border.0 = color;
        }
        show(&mut vis, true);
    }
}

// --- the world -----------------------------------------------------------

/// A site's ring on the ground.
#[derive(Component)]
struct SiteRing;
/// The bomb, lying dropped or planted...
#[derive(Component)]
struct BombModel;
/// ...and its light, blinking once it's planted.
#[derive(Component)]
struct BombLight;

/// The sites' rings (once a game's on), and the bomb wherever it's lying.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_world(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    rings: Query<(), With<SiteRing>>,
    mut bomb: Query<(&mut Transform, &mut Visibility), (With<BombModel>, Without<BombLight>)>,
    mut light: Query<&mut Visibility, (With<BombLight>, Without<BombModel>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(lobby) = snd_game(&local, &lobbies) else { return };
    if rings.is_empty() {
        let sites = layout(lobby.map);
        let ring = meshes.add(Mesh::from(Annulus::new(SITE_RADIUS - 0.18, SITE_RADIUS)));
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.55),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        });
        for at in sites.sites {
            commands.spawn((
                SiteRing,
                StateScoped(AppState::InGame),
                Mesh3d(ring.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(at + Vec3::Y * 0.04).with_rotation(Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)),
            ));
        }
        let body = materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.13, 0.12),
            perceptual_roughness: 0.7,
            ..default()
        });
        let glow = materials.add(StandardMaterial {
            base_color: BOMB_RED,
            emissive: LinearRgba::rgb(12.0, 1.2, 0.6),
            unlit: true,
            ..default()
        });
        commands
            .spawn((
                BombModel,
                StateScoped(AppState::InGame),
                Mesh3d(meshes.add(Cuboid::from_size(BOMB_SIZE))),
                MeshMaterial3d(body),
                Transform::default(),
                Visibility::Hidden,
            ))
            .with_child((
                BombLight,
                Mesh3d(meshes.add(Sphere::new(0.035))),
                MeshMaterial3d(glow),
                Transform::from_xyz(0.1, BOMB_SIZE.y / 2.0 + 0.01, 0.06),
                Visibility::Hidden,
            ));
        return;
    }
    let (at, planted) = match lobby.snd.bomb {
        Bomb::Dropped(at) => (Some(at), false),
        Bomb::Planted { at, .. } if lobby.snd.phase != Phase::PreRound => (Some(at), true),
        _ => (None, false),
    };
    for (mut t, mut vis) in &mut bomb {
        show(&mut vis, at.is_some());
        if let Some(at) = at {
            let want = Vec3::from_array(at) + Vec3::Y * BOMB_SIZE.y / 2.0;
            if t.translation != want {
                t.translation = want;
            }
        }
    }
    // (Blinking faster as the fuse runs out.)
    let left = lobby.snd.secs_left as f32 / FUSE_SECS as f32;
    let blink = planted && (time.elapsed_secs() / (0.2 + 0.6 * left)).fract() < 0.5;
    for mut vis in &mut light {
        show(&mut vis, blink || (at.is_some() && !planted));
    }
}

/// The planted bomb beeps (faster as the fuse runs out), and blows up when
/// the server says.
fn beep_and_blow_up(
    time: Res<Time>,
    sounds: Option<Res<SndSounds>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut state: ResMut<SndLocal>,
    mut exploded: Query<&mut MessageReceiver<shared::SndBombExploded>>,
    mut boom: EventWriter<crate::vfx::Explosion>,
    mut commands: Commands,
) {
    for mut rx in &mut exploded {
        for msg in rx.receive() {
            boom.write(crate::vfx::Explosion {
                feet: Vec3::from_array(msg.at),
                variant: 0,
                phd: false,
                dog: false,
                boss: true,
                frag: false,
                nuke: false,
            });
        }
    }
    let now = time.elapsed_secs();
    let planted = snd_game(&local, &lobbies).and_then(|l| match l.snd.bomb {
        Bomb::Planted { at, .. } if l.snd.phase == Phase::Live => Some((Vec3::from_array(at), l.snd.secs_left)),
        _ => None,
    });
    let Some((at, secs_left)) = planted else {
        state.next_beep = 0.0;
        return;
    };
    if now < state.next_beep {
        return;
    }
    let left = secs_left as f32 / FUSE_SECS as f32;
    state.next_beep = now + BEEP_FAST_SECS + (BEEP_SLOW_SECS - BEEP_FAST_SECS) * left;
    if let Some(sounds) = sounds {
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.beep.clone()),
            Transform::from_translation(at),
            crate::positional_playback(Volume::Linear(1.0)),
        ));
    }
}
