//! `Zombies` exfil (`shared::exfil`), the client's side: the radio
//! (`models/props/exfil_radio.glb`, where the map's layout puts it), the
//! prompt at it on an exfil round — hold interact for
//! [`shared::exfil::HOLD_SECS`] to call it in, the prompt giving way to
//! "EXTRACTING" over the icon with a blue ring filling clockwise as it's
//! held (let go early and it starts over) — then, once it's called, the
//! white-out every screen gets, the four orange lines fading in on the ground
//! round the exfil area, and the timer (top centre) with a warning while
//! we're outside the area (only kills from inside it count). On an exfil
//! round that isn't called yet, a smaller notice there says it's available.
//!
//! Everything reads the replicated `Lobby` (the server runs the exfil —
//! `server::exfil`). The radio, UI and lines are `StateScoped(InGame)`, the
//! radio and lines are taken away whenever they stop applying, and the hold
//! and white-out ([`ExfilLocal`]) reset whenever there's no exfil to show —
//! nothing carries into the next game.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy_rapier3d::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::exfil::{Exfil, CALLING_SECS, HOLD_SECS};
use shared::level::ExfilArea;
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::{zombies_game, CARD_RED};
use crate::{killcam, menu, AppState, MapModel, Player, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) const RADIO_MODEL: &str = "models/props/exfil_radio.glb";
pub(crate) const EXFIL_ICON: &str = "textures/icons/exfil.png";
const ICON_SHADER: &str = "shaders/exfil_icon.wgsl";

/// The exfil's blue (its ring, the prompt) and the area lines' orange.
pub(crate) const EXFIL_BLUE: Color = Color::srgb(0.18, 0.58, 1.0);
pub(crate) const EXFIL_ORANGE: Color = Color::srgb(1.0, 0.45, 0.04);

/// The area lines: how wide (m), how high off the ground they float, the
/// longest straight piece (each follows the ground between its ends), and
/// how long (s) they take to fade in.
const LINE_WIDTH: f32 = 0.16;
const LINE_LIFT: f32 = 0.04;
const LINE_PIECE: f32 = 1.0;
const LINE_FADE_SECS: f32 = 1.5;

pub(crate) struct ExfilPlugin;

impl Plugin for ExfilPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<ExfilIconMaterial>::default())
            .init_resource::<ExfilLocal>()
            .add_systems(OnEnter(AppState::InGame), spawn_exfil_ui)
            .add_systems(OnExit(AppState::InGame), |mut local: ResMut<ExfilLocal>| *local = default())
            .add_systems(
                Update,
                (
                    sync_radio,
                    hold_to_call.run_if(menu::game_active.and(killcam::no_killcam)),
                    track_exfil,
                    (sync_area_lines, update_exfil_ui).after(track_exfil),
                )
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// This client's exfil bits: how long interact's been held at the radio
/// (and whether that's already called it), and when we saw the exfil get
/// called (`Time::elapsed_secs` — the white-out and the lines run off it).
/// Reset whenever there's no exfil running and on leaving the game.
#[derive(Resource, Default)]
struct ExfilLocal {
    held: f32,
    sent: bool,
    called_at: Option<f32>,
}

// --- the radio -----------------------------------------------------------------

#[derive(Component)]
struct ExfilRadio;

/// Put the radio on the map while we're in a `Zombies` game on a map that
/// has one (and take it away otherwise) — solid, like the server's
/// `shared::exfil::radio_box`.
fn sync_radio(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut radios: Query<(Entity, &mut Transform), With<ExfilRadio>>,
    mut commands: Commands,
) {
    let Some(at) = zombies_game(&local, &lobbies).and_then(|l| shared::level::layout(l.map).exfil_radio) else {
        for (e, _) in &radios {
            commands.entity(e).despawn();
        }
        return;
    };
    if let Some((_, mut t)) = radios.iter_mut().next() {
        t.set_if_neq(crate::util::placed(at));
        return;
    }
    let half = shared::exfil::RADIO_HALF_EXTENTS;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            ExfilRadio,
            crate::util::placed(at),
            Visibility::default(),
        ))
        .with_children(|r| {
            r.spawn(SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(RADIO_MODEL))));
            r.spawn((
                Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(Vec3::Y * half.y),
            ));
        });
}

/// Whether we can call the exfil from where we stand: it's available, and
/// we're at the radio.
fn at_radio(lobby: &Lobby, feet: Vec3) -> bool {
    shared::exfil::available(lobby)
        && shared::exfil::layout(lobby.map).is_some_and(|(radio, _)| shared::exfil::radio_in_range_of(radio, feet, 0.0))
}

/// Hold interact at the radio: once it's been held for
/// [`HOLD_SECS`] straight, ask the server to call the exfil (it checks
/// again). Letting go — or walking off — starts it over.
fn hold_to_call(
    time: Res<Time>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut state: ResMut<ExfilLocal>,
    mut sender: Query<&mut TriggerSender<shared::CallExfil>, With<GameClient>>,
) {
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let holding = binds.interact.pressed(&keys, &mouse)
        && zombies_game(&local, &lobbies).is_some_and(|l| at_radio(l, feet));
    if !holding {
        state.held = 0.0;
        state.sent = false;
        return;
    }
    state.held = (state.held + time.delta_secs()).min(HOLD_SECS);
    if state.held >= HOLD_SECS && !state.sent {
        state.sent = true;
        if let Ok(mut s) = sender.single_mut() {
            s.trigger::<shared::LobbyChannel>(shared::CallExfil);
        }
    }
}

/// Note when the exfil gets called (for the white-out and the lines), and
/// forget it all once there's no exfil running.
fn track_exfil(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut state: ResMut<ExfilLocal>,
) {
    let exfil = zombies_game(&local, &lobbies).map_or(Exfil::Idle, |l| l.exfil);
    if exfil.running() {
        if state.called_at.is_none() {
            state.called_at = Some(time.elapsed_secs());
        }
        // (No holding on once it's called.)
        state.held = 0.0;
    } else if state.called_at.is_some() {
        state.called_at = None;
    }
}

// --- the area lines ------------------------------------------------------------------

/// The four lines round the exfil area, and their (shared) material to fade.
#[derive(Component)]
struct AreaLines(Handle<StandardMaterial>);

/// Put the area's lines down once the exfil's called — fading in from the
/// white-out's peak — and take them away once it's over.
#[allow(clippy::too_many_arguments)]
fn sync_area_lines(
    time: Res<Time>,
    state: Res<ExfilLocal>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    rapier: ReadRapierContext,
    map_models: Query<(), With<MapModel>>,
    parents: Query<&ChildOf>,
    lines: Query<(Entity, &AreaLines)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let area = zombies_game(&local, &lobbies)
        .filter(|l| l.exfil.running())
        .and_then(|l| shared::level::layout(l.map).exfil_area);
    let (Some(area), Some(called_at)) = (area, state.called_at) else {
        for (e, _) in &lines {
            commands.entity(e).despawn();
        }
        return;
    };
    let t = time.elapsed_secs() - called_at - CALLING_SECS * 0.5;
    let alpha = (t / LINE_FADE_SECS).clamp(0.0, 1.0);
    if let Some((_, lines)) = lines.iter().next() {
        if let Some(m) = materials.get_mut(&lines.0) {
            if (m.base_color.alpha() - alpha).abs() > 1e-3 {
                m.base_color.set_alpha(alpha);
            }
        }
        return;
    }

    // The ground under a point, near the area's height (only the map's own
    // — not a machine's box or a player).
    let ground = |p: Vec3| -> f32 {
        let on_map = |mut e: Entity| loop {
            if map_models.contains(e) {
                return true;
            }
            match parents.get(e) {
                Ok(parent) => e = parent.parent(),
                Err(_) => return false,
            }
        };
        let from = Vec3::new(p.x, area.at.pos.y + 1.0, p.z);
        rapier
            .single()
            .ok()
            .and_then(|r| r.cast_ray(from, Vec3::NEG_Y, 3.0, true, QueryFilter::default().predicate(&on_map)))
            .map_or(area.at.pos.y, |(_, toi)| from.y - toi)
    };
    let material = materials.add(StandardMaterial {
        base_color: EXFIL_ORANGE.with_alpha(alpha),
        emissive: LinearRgba::from(EXFIL_ORANGE) * 3.0,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    commands
        .spawn((
            StateScoped(AppState::InGame),
            AreaLines(material.clone()),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .with_children(|root| {
            for (a, b) in edges(&area) {
                let n = ((b - a).length() / LINE_PIECE).ceil().max(1.0) as usize;
                let points: Vec<Vec3> = (0..=n)
                    .map(|i| {
                        let p = a.lerp(b, i as f32 / n as f32);
                        Vec3::new(p.x, ground(p) + LINE_LIFT, p.z)
                    })
                    .collect();
                for pair in points.windows(2) {
                    let (p, q) = (pair[0], pair[1]);
                    let along = q - p;
                    let len = along.length();
                    if len < 1e-3 {
                        continue;
                    }
                    root.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        NotShadowCaster,
                        // (A line's width longer, so the pieces' joins and
                        // the corners close up.)
                        Transform::from_translation((p + q) * 0.5)
                            .with_rotation(Quat::from_rotation_arc(Vec3::X, along / len))
                            .with_scale(Vec3::new(len + LINE_WIDTH, 0.02, LINE_WIDTH)),
                    ));
                }
            }
        });
}

/// The area's four sides.
fn edges(area: &ExfilArea) -> [(Vec3, Vec3); 4] {
    let c = area.corners();
    [(c[0], c[1]), (c[1], c[2]), (c[2], c[3]), (c[3], c[0])]
}

// --- the UI -----------------------------------------------------------------------------

/// The icon in its ring.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub(crate) struct ExfilIconMaterial {
    /// x: hold progress (0..=1), y: opacity, z: 1 to draw the ring. (A
    /// `Vec4` for WebGL2's 16-byte uniform alignment.)
    #[uniform(0)]
    params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    icon: Handle<Image>,
}

impl UiMaterial for ExfilIconMaterial {
    fn fragment_shader() -> ShaderRef {
        ICON_SHADER.into()
    }
}

/// The prompt at the radio: the icon, and what holding interact does.
#[derive(Component)]
struct ExfilPrompt;

#[derive(Component)]
struct ExfilPromptKey;

/// Shown instead while interact's held: "EXTRACTING" over the icon, its
/// ring filling.
#[derive(Component)]
struct ExfilExtracting;

#[derive(Component)]
struct ExfilExtractingIcon;

/// The white-out.
#[derive(Component)]
struct ExfilFlash;

/// Top centre: the timer once it's on (or the notice that it's
/// available), with its title and the line under it.
#[derive(Component)]
struct ExfilBanner;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BannerText {
    Title,
    Timer,
    Hint,
}

fn spawn_exfil_ui(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ExfilIconMaterial>>,
) {
    let font = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    let icon = asset_server.load(EXFIL_ICON);
    let heading = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let shadow = TextShadow {
        offset: Vec2::splat(2.0),
        color: Color::srgba(0.0, 0.0, 0.0, 0.8),
    };
    let centred_row = |top: f32| Node {
        position_type: PositionType::Absolute,
        top: Val::Percent(top),
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        justify_content: JustifyContent::Center,
        ..default()
    };

    // The prompt: icon on the left, what to do on the right.
    commands
        .spawn((StateScoped(AppState::InGame), GlobalZIndex(5), centred_row(57.0)))
        .with_children(|row| {
            row.spawn((
                ExfilPrompt,
                Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(16.0),
                    padding: UiRect::all(Val::Px(14.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    max_width: Val::Px(440.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(EXFIL_BLUE),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((
                    MaterialNode(materials.add(ExfilIconMaterial {
                        params: Vec4::new(0.0, 1.0, 0.0, 0.0),
                        icon: icon.clone(),
                    })),
                    Node {
                        width: Val::Px(64.0),
                        height: Val::Px(64.0),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
                card.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((Text::new("EXFIL"), heading(30.0), TextColor(EXFIL_BLUE)));
                    col.spawn((ExfilPromptKey, Text::new(""), heading(22.0), TextColor::WHITE));
                    col.spawn((
                        Text::new(format!(
                            "Kill every enemy from inside the exfil zone within {}:{:02}.",
                            shared::exfil::TIME_LIMIT_SECS / 60,
                            shared::exfil::TIME_LIMIT_SECS % 60
                        )),
                        TextFont {
                            font: body.clone(),
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                    ));
                });
            });
        });

    // While it's held: the word over the icon, its ring filling.
    commands
        .spawn((StateScoped(AppState::InGame), GlobalZIndex(5), centred_row(55.0)))
        .with_children(|row| {
            row.spawn((
                ExfilExtracting,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    ..default()
                },
                Visibility::Hidden,
            ))
            .with_children(|col| {
                col.spawn((Text::new("EXTRACTING"), heading(30.0), TextColor::WHITE, shadow));
                col.spawn((
                    ExfilExtractingIcon,
                    MaterialNode(materials.add(ExfilIconMaterial {
                        params: Vec4::new(0.0, 1.0, 1.0, 0.0),
                        icon: icon.clone(),
                    })),
                    Node {
                        width: Val::Px(84.0),
                        height: Val::Px(84.0),
                        ..default()
                    },
                ));
            });
        });

    // Top centre (under the compass, where the pre-game countdown goes —
    // never both at once).
    commands
        .spawn((
            StateScoped(AppState::InGame),
            ExfilBanner,
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(crate::hud::COMPASS_HEIGHT + 8.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn((BannerText::Title, Text::new(""), heading(24.0), TextColor(EXFIL_BLUE), shadow));
            col.spawn((BannerText::Timer, Text::new(""), heading(56.0), TextColor::WHITE, shadow));
            col.spawn((BannerText::Hint, Text::new(""), heading(18.0), TextColor::WHITE, shadow));
        });

    // The white-out, over the HUD.
    commands.spawn((
        StateScoped(AppState::InGame),
        ExfilFlash,
        GlobalZIndex(40),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::WHITE.with_alpha(0.0)),
        bevy::ui::FocusPolicy::Pass,
    ));
}

/// The prompt / extracting UI, the banner and the white-out, from the
/// replicated lobby and our hold. (The prompt and banner hide behind menus
/// and the kill cam like the rest of the HUD; the white-out doesn't.)
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_exfil_ui(
    time: Res<Time>,
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    state: Res<ExfilLocal>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut vis: ParamSet<(
        Single<&mut Visibility, With<ExfilPrompt>>,
        Single<&mut Visibility, With<ExfilExtracting>>,
        Single<&mut Visibility, With<ExfilBanner>>,
    )>,
    mut key_text: Single<&mut Text, (With<ExfilPromptKey>, Without<BannerText>)>,
    ring: Single<&MaterialNode<ExfilIconMaterial>, With<ExfilExtractingIcon>>,
    mut materials: ResMut<Assets<ExfilIconMaterial>>,
    mut flash: Single<&mut BackgroundColor, With<ExfilFlash>>,
    mut banner: Query<(&BannerText, &mut Text, &mut TextColor), Without<ExfilPromptKey>>,
) {
    let lobby = zombies_game(&local, &lobbies);
    let hud = !menu.is_open() && active_killcam.0.is_none();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let at = hud && lobby.is_some_and(|l| at_radio(l, feet));
    let holding = at && state.held > 0.0;
    let set = |mut v: Mut<Visibility>, on: bool| {
        v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    };
    set(vis.p0().into_inner(), at && !holding);
    set(vis.p1().into_inner(), holding);

    let wanted = format!("HOLD {} TO CALL IT IN", binds.interact.label().to_uppercase());
    if key_text.0 != wanted {
        key_text.0 = wanted;
    }
    if let Some(m) = materials.get_mut(&ring.0) {
        let p = (state.held / HOLD_SECS).clamp(0.0, 1.0);
        if (m.params.x - p).abs() > 1e-4 {
            m.params.x = p;
        }
    }

    // The white-out: up to full white half-way through the call, back down
    // by its end.
    let white = state.called_at.map_or(0.0, |at| {
        let t = (time.elapsed_secs() - at) / CALLING_SECS;
        if (0.0..1.0).contains(&t) {
            1.0 - (t * 2.0 - 1.0).abs()
        } else {
            0.0
        }
    });
    let c = Color::WHITE.with_alpha(white.powf(0.7));
    if flash.0 != c {
        flash.0 = c;
    }

    // The banner: the timer while it's on, the notice while it's there to
    // be called.
    let exfil = lobby.map(|l| l.exfil);
    let lines: Option<(String, String, String, Color)> = match (lobby, exfil) {
        (Some(l), Some(Exfil::Active { secs_left })) => {
            let inside = shared::exfil::layout(l.map).is_some_and(|(_, area)| area.contains(feet));
            let hint = if inside {
                "KILL EVERY ENEMY INSIDE THE EXFIL ZONE"
            } else {
                "GET IN THE EXFIL ZONE — KILLS OUTSIDE IT DON'T COUNT"
            };
            let color = if secs_left <= 10 { CARD_RED } else { Color::WHITE };
            Some(("EXFIL".into(), format!("{}:{:02}", secs_left / 60, secs_left % 60), hint.into(), color))
        }
        (Some(l), _) if shared::exfil::available(l) => Some((
            "EXFIL AVAILABLE".into(),
            String::new(),
            "USE THE RADIO TO CALL IT IN".into(),
            Color::WHITE,
        )),
        _ => None,
    };
    let shown = hud && lines.is_some();
    set(vis.p2().into_inner(), shown);
    let Some((title, timer, hint, timer_color)) = lines else { return };
    let inside = !matches!(exfil, Some(Exfil::Active { .. }))
        || lobby
            .and_then(|l| shared::exfil::layout(l.map))
            .is_some_and(|(_, area)| area.contains(feet));
    for (which, mut text, mut color) in &mut banner {
        let (wanted, wanted_color) = match which {
            BannerText::Title => (title.clone(), EXFIL_BLUE),
            BannerText::Timer => (timer.clone(), timer_color),
            BannerText::Hint => (hint.clone(), if inside { Color::WHITE } else { EXFIL_ORANGE }),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        color.set_if_neq(TextColor(wanted_color));
    }
}
