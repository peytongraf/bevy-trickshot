//! `Zombies`' Rampage Inducer (`shared::rampage`), the client's side: the
//! machine (`models/props/rampage_inducer.glb`, where the map's layout puts
//! it — placed in the level editor) with an orange light glowing out of its
//! glass globe; the Call of Duty prompt at it — hold interact for
//! [`HOLD_SECS`] to activate it (or, while it's on, to deactivate it), the
//! prompt giving way to ACTIVATING / DEACTIVATING over the icon, a blue ring
//! filling round it as it's held (let go early and it starts over); and,
//! while it's on, for everyone in the lobby: the start sound as it comes on
//! (not positional), the rampage music looping until it goes off, then the
//! stop sound, and yellow lightning crackling behind the round number
//! (`round_counter`), fading in and out with it.
//!
//! Everything reads the replicated `Lobby::rampage` (the server flips it —
//! `server::rampage`). The machine and UI are `StateScoped(InGame)`, the
//! machine's taken away whenever it stops applying, and the hold, the
//! lightning's fade and what's been heard ([`RampageLocal`]) reset on
//! leaving the game — nothing carries into the next one.
//!
//! Its look and sounds are tuned in the debug panel's "Rampage Inducer
//! (Zombies)" section ([`RampageSettings`], [`RampageDebug`]).

use bevy::audio::Volume;
use bevy::ecs::system::SystemParam;
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy_egui::egui;
use bevy_rapier3d::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::rampage::{GLOBE_CENTRE, HALF_EXTENTS, HOLD_SECS};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::settings::Settings;
use crate::zombies_hud::zombies_game;
use crate::{killcam, menu, AppState, Player, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) const RAMPAGE_MODEL: &str = "models/props/rampage_inducer.glb";
pub(crate) const RAMPAGE_ICON: &str = "textures/icons/rampage_inducer_icon.png";
const ICON_SHADER: &str = "shaders/rampage_icon.wgsl";
const LIGHTNING_SHADER: &str = "shaders/rampage_lightning.wgsl";
const START_SOUND: &str = "audio/zombies/rampage/rampage_start.mp3";
const STOP_SOUND: &str = "audio/zombies/rampage/rampage_stop.mp3";
const MUSIC: &str = "audio/zombies/rampage/rampage_music.mp3";

/// Where the icon's circle is in its picture (`rampage_inducer_icon.png`,
/// 1536 × 1024, a 445 px-radius disc in the middle): its centre, then its
/// radius across and down (uv).
const ICON_CIRCLE: Vec4 = Vec4::new(0.5, 0.5, 445.0 / 1536.0, 445.0 / 1024.0);

/// The inducer's orange (the prompt's edge and title; its light's default).
pub(crate) const RAMPAGE_ORANGE: Color = Color::srgb(1.0, 0.45, 0.08);

pub(crate) struct RampagePlugin;

impl Plugin for RampagePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            UiMaterialPlugin::<RampageIconMaterial>::default(),
            UiMaterialPlugin::<RampageLightningMaterial>::default(),
        ))
        .init_resource::<RampageSettings>()
        .init_resource::<RampageLocal>()
        .add_systems(Startup, load_sounds)
        .add_systems(OnEnter(AppState::InGame), spawn_rampage_ui)
        .add_systems(OnExit(AppState::InGame), |mut local: ResMut<RampageLocal>| *local = default())
        .add_systems(
            Update,
            (
                sync_inducer,
                hold_to_toggle.run_if(menu::game_active.and(killcam::no_killcam)),
                rampage_audio,
                (update_rampage_ui, update_inducer_light, update_lightning),
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Panel-tunable look and sound ("Rampage Inducer (Zombies)").
#[derive(Resource, Clone, PartialEq, Debug)]
pub(crate) struct RampageSettings {
    /// The light out of its globe: colour, how bright while it's off and
    /// while it's on, how far it reaches (m), how much it flickers while
    /// it's on (0..1), and how high up the machine it sits (× its height as
    /// made — the globe's middle by default).
    pub(crate) light_color: [f32; 3],
    pub(crate) light_intensity_off: f32,
    pub(crate) light_intensity_on: f32,
    pub(crate) light_range: f32,
    pub(crate) light_flicker: f32,
    pub(crate) light_height: f32,
    /// Volumes (linear, before master volume).
    pub(crate) start_volume: f32,
    pub(crate) stop_volume: f32,
    pub(crate) music_volume: f32,
    /// The lightning behind the round number: colour and brightness, how
    /// many bolts, how often (a second) each reshapes, how jagged, its core
    /// thickness and glow (px), how far out round the number it reaches (×
    /// the number's size, each side), how much of its edge fades, and how
    /// long (s) it fades in / out.
    pub(crate) lightning_color: [f32; 3],
    pub(crate) lightning_brightness: f32,
    pub(crate) lightning_bolts: f32,
    pub(crate) lightning_rate: f32,
    pub(crate) lightning_jag: f32,
    pub(crate) lightning_core: f32,
    pub(crate) lightning_glow: f32,
    pub(crate) lightning_pad: f32,
    pub(crate) lightning_edge: f32,
    pub(crate) fade_in_secs: f32,
    pub(crate) fade_out_secs: f32,
    /// Debug: show the lightning (and the light at full) as if it were on.
    pub(crate) preview: bool,
}

impl Default for RampageSettings {
    fn default() -> Self {
        Self {
            light_color: [1.0, 0.45, 0.08],
            light_intensity_off: 25_000.0,
            light_intensity_on: 90_000.0,
            light_range: 8.0,
            light_flicker: 0.25,
            light_height: 1.0,
            start_volume: 1.0,
            stop_volume: 1.0,
            music_volume: 0.6,
            lightning_color: [1.0, 0.82, 0.15],
            lightning_brightness: 1.2,
            lightning_bolts: 3.0,
            lightning_rate: 9.0,
            lightning_jag: 0.12,
            lightning_core: 2.0,
            lightning_glow: 5.0,
            lightning_pad: 0.45,
            lightning_edge: 0.35,
            fade_in_secs: 0.6,
            fade_out_secs: 0.9,
            preview: false,
        }
    }
}

/// This client's side of it: how long interact's been held at the machine
/// (and whether that's already been sent), whether it was on last frame
/// (`None` outside a `Zombies` game — so joining one with it already on
/// doesn't play the start sound), and how far the lightning's faded in.
#[derive(Resource, Default)]
struct RampageLocal {
    held: f32,
    sent: bool,
    was_on: Option<bool>,
    fade: f32,
}

#[derive(Resource)]
struct RampageSounds {
    start: Handle<AudioSource>,
    stop: Handle<AudioSource>,
    music: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(RampageSounds {
        start: asset_server.load(START_SOUND),
        stop: asset_server.load(STOP_SOUND),
        music: asset_server.load(MUSIC),
    });
}

/// Whether the inducer can be used from where `feet` is, in `lobby`'s game.
fn at_inducer(lobby: &Lobby, feet: Vec3) -> bool {
    shared::rampage::available(lobby)
        && shared::rampage::layout(lobby.map).is_some_and(|at| shared::rampage::in_range_of(at, feet, 0.0))
}

// --- the machine ------------------------------------------------------------

/// The machine (its model, solid box and light are children).
#[derive(Component)]
struct RampageInducer;

/// The light out of its globe.
#[derive(Component)]
struct InducerLight;

/// Keep the machine where the map's layout puts it in a `Zombies` game —
/// none anywhere else.
fn sync_inducer(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut machines: Query<(Entity, &mut Transform), With<RampageInducer>>,
    mut commands: Commands,
) {
    let Some(at) = zombies_game(&local, &lobbies).and_then(|l| shared::rampage::layout(l.map)) else {
        for (e, _) in &machines {
            commands.entity(e).despawn();
        }
        return;
    };
    if let Some((_, mut t)) = machines.iter_mut().next() {
        t.set_if_neq(crate::util::placed(at));
        return;
    }
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RampageInducer,
            crate::util::placed(at),
            Visibility::default(),
        ))
        .with_children(|m| {
            m.spawn(SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(RAMPAGE_MODEL))));
            m.spawn((
                Collider::cuboid(HALF_EXTENTS.x, HALF_EXTENTS.y, HALF_EXTENTS.z),
                Transform::from_translation(Vec3::Y * HALF_EXTENTS.y),
            ));
            m.spawn((
                InducerLight,
                PointLight {
                    intensity: 0.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_translation(GLOBE_CENTRE),
                NotShadowCaster,
            ));
        });
}

/// The globe's light: dim while it's off, bright (and flickering) while
/// it's on, easing between with the lightning's fade.
fn update_inducer_light(
    time: Res<Time>,
    settings: Res<RampageSettings>,
    local: Res<RampageLocal>,
    mut lights: Query<(&mut PointLight, &mut Transform), With<InducerLight>>,
) {
    let s = &*settings;
    let t = time.elapsed_secs();
    // (A couple of out-of-step waves — an uneven flicker.)
    let wobble = ((t * 11.0).sin() * 0.6 + (t * 23.7).sin() * 0.4) * 0.5 + 0.5;
    let flicker = 1.0 - s.light_flicker * local.fade * wobble;
    let intensity = (s.light_intensity_off + (s.light_intensity_on - s.light_intensity_off) * local.fade) * flicker;
    let [r, g, b] = s.light_color;
    for (mut light, mut transform) in &mut lights {
        light.intensity = intensity.max(0.0);
        light.range = s.light_range;
        light.color = Color::srgb(r, g, b);
        let at = GLOBE_CENTRE * Vec3::new(1.0, s.light_height, 1.0);
        if transform.translation != at {
            transform.translation = at;
        }
    }
}

/// Hold interact at the machine: once it's been held for [`HOLD_SECS`]
/// straight, ask the server to flip it (it checks again). Letting go — or
/// walking off — starts it over; still holding after it's flipped does
/// nothing more until it's let go.
#[allow(clippy::too_many_arguments)]
fn hold_to_toggle(
    time: Res<Time>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut state: ResMut<RampageLocal>,
    mut sender: Query<&mut TriggerSender<shared::ToggleRampage>, With<GameClient>>,
) {
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let lobby = zombies_game(&local, &lobbies).filter(|l| at_inducer(l, feet));
    let Some(lobby) = lobby.filter(|_| binds.interact.pressed(&keys, &mouse)) else {
        state.held = 0.0;
        state.sent = false;
        return;
    };
    if state.sent {
        return;
    }
    state.held = (state.held + time.delta_secs()).min(HOLD_SECS);
    if state.held >= HOLD_SECS {
        state.sent = true;
        state.held = 0.0;
        if let Ok(mut s) = sender.single_mut() {
            s.trigger::<shared::LobbyChannel>(shared::ToggleRampage { on: !lobby.rampage });
        }
    }
}

// --- sound -------------------------------------------------------------------------

/// The rampage music, looping while it's on.
#[derive(Component)]
struct RampageMusic;

/// As it comes on: its start sound and the music; as it goes off: the music
/// out and its stop sound — for everyone in the lobby, not positional. And
/// the lightning's fade, toward on or off.
#[allow(clippy::too_many_arguments)]
fn rampage_audio(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<RampageSounds>>,
    settings: Res<RampageSettings>,
    game_settings: Res<Settings>,
    mut state: ResMut<RampageLocal>,
    mut music: Query<(Entity, Option<&mut AudioSink>), With<RampageMusic>>,
    mut commands: Commands,
) {
    let s = &*settings;
    let on = zombies_game(&local, &lobbies).map(|l| l.rampage);
    let now_on = on == Some(true);
    if let Some(sounds) = &sounds {
        let one_shot = |clip: &Handle<AudioSource>, volume: f32, commands: &mut Commands| {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(clip.clone()),
                // (`GlobalVolume` — master volume — is multiplied in at spawn.)
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
            ));
        };
        match (state.was_on, on) {
            (Some(false), Some(true)) => one_shot(&sounds.start, s.start_volume, &mut commands),
            (Some(true), Some(false)) => one_shot(&sounds.stop, s.stop_volume, &mut commands),
            _ => {}
        }
        if now_on && music.is_empty() {
            commands.spawn((
                RampageMusic,
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.music.clone()),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(s.music_volume)),
            ));
        }
    }
    for (e, sink) in &mut music {
        if !now_on {
            commands.entity(e).despawn();
        } else if let Some(mut sink) = sink {
            // (Following the panel and master volume while it plays.)
            let volume = Volume::Linear(s.music_volume * game_settings.master_volume);
            if sink.volume() != volume {
                sink.set_volume(volume);
            }
        }
    }
    state.was_on = on;

    let target = if now_on || s.preview { 1.0 } else { 0.0 };
    let secs = if target > state.fade { s.fade_in_secs } else { s.fade_out_secs };
    let step = time.delta_secs() / secs.max(0.01);
    state.fade = if target > state.fade {
        (state.fade + step).min(1.0)
    } else {
        (state.fade - step).max(0.0)
    };
}

// --- the UI ---------------------------------------------------------------------------

/// The icon in its ring.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub(crate) struct RampageIconMaterial {
    /// x: hold progress (0..=1), y: opacity, z: 1 to draw the ring.
    #[uniform(0)]
    params: Vec4,
    /// [`ICON_CIRCLE`].
    #[uniform(1)]
    circle: Vec4,
    #[texture(2)]
    #[sampler(3)]
    icon: Handle<Image>,
}

impl UiMaterial for RampageIconMaterial {
    fn fragment_shader() -> ShaderRef {
        ICON_SHADER.into()
    }
}

/// The lightning behind the round number (`round_counter` spawns the panel
/// behind the number and keeps it round it; this keeps its look).
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone, Default)]
pub(crate) struct RampageLightningMaterial {
    /// x: time (s), y: fade, z: brightness, w: bolts.
    #[uniform(0)]
    params: Vec4,
    /// rgb: colour, w: glow (px).
    #[uniform(1)]
    colour: Vec4,
    /// x: reshapes a second, y: jaggedness, z: core (px), w: edge fade.
    #[uniform(2)]
    look: Vec4,
}

impl UiMaterial for RampageLightningMaterial {
    fn fragment_shader() -> ShaderRef {
        LIGHTNING_SHADER.into()
    }
}

/// The lightning panel behind the round number.
#[derive(Component)]
pub(crate) struct RoundLightning;

/// The prompt at the machine: the icon, and what holding interact does.
#[derive(Component)]
struct RampagePrompt;

#[derive(Component, Clone, Copy, PartialEq)]
enum PromptText {
    /// "HOLD [F] TO ACTIVATE" / "...DEACTIVATE".
    Key,
    /// What that does.
    Effect,
}

/// Shown instead while interact's held: ACTIVATING / DEACTIVATING over the
/// icon, its ring filling.
#[derive(Component)]
struct RampageHolding;

#[derive(Component)]
struct RampageHoldingText;

#[derive(Component)]
struct RampageHoldingIcon;

fn spawn_rampage_ui(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<RampageIconMaterial>>,
) {
    let font = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    let icon = asset_server.load(RAMPAGE_ICON);
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
                RampagePrompt,
                Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(16.0),
                    padding: UiRect::all(Val::Px(14.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    max_width: Val::Px(480.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(RAMPAGE_ORANGE),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((
                    MaterialNode(materials.add(RampageIconMaterial {
                        params: Vec4::new(0.0, 1.0, 0.0, 0.0),
                        circle: ICON_CIRCLE,
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
                    col.spawn((Text::new("RAMPAGE INDUCER"), heading(30.0), TextColor(RAMPAGE_ORANGE)));
                    col.spawn((PromptText::Key, Text::new(""), heading(22.0), TextColor::WHITE));
                    col.spawn((
                        PromptText::Effect,
                        Text::new(""),
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
                RampageHolding,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    ..default()
                },
                Visibility::Hidden,
            ))
            .with_children(|col| {
                col.spawn((RampageHoldingText, Text::new(""), heading(30.0), TextColor::WHITE, shadow));
                col.spawn((
                    RampageHoldingIcon,
                    MaterialNode(materials.add(RampageIconMaterial {
                        params: Vec4::new(0.0, 1.0, 1.0, 0.0),
                        circle: ICON_CIRCLE,
                        icon,
                    })),
                    Node {
                        width: Val::Px(84.0),
                        height: Val::Px(84.0),
                        ..default()
                    },
                ));
            });
        });
}

/// The prompt / holding UI, from the replicated lobby and our hold (hidden
/// behind menus and the kill cam, like the rest of the HUD).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_rampage_ui(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    state: Res<RampageLocal>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut vis: ParamSet<(
        Single<&mut Visibility, With<RampagePrompt>>,
        Single<&mut Visibility, With<RampageHolding>>,
    )>,
    mut texts: Query<(&PromptText, &mut Text), Without<RampageHoldingText>>,
    mut holding_text: Single<&mut Text, (With<RampageHoldingText>, Without<PromptText>)>,
    ring: Single<&MaterialNode<RampageIconMaterial>, With<RampageHoldingIcon>>,
    mut materials: ResMut<Assets<RampageIconMaterial>>,
) {
    let lobby = zombies_game(&local, &lobbies);
    let hud = !menu.is_open() && active_killcam.0.is_none();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let at = hud && lobby.is_some_and(|l| at_inducer(l, feet));
    let holding = at && state.held > 0.0;
    let on = lobby.is_some_and(|l| l.rampage);
    let set = |mut v: Mut<Visibility>, show: bool| {
        v.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
    };
    set(vis.p0().into_inner(), at && !holding);
    set(vis.p1().into_inner(), holding);

    let key = binds.interact.label().to_uppercase();
    for (which, mut text) in &mut texts {
        let wanted = match (which, on) {
            (PromptText::Key, false) => format!("HOLD [{key}] TO ACTIVATE RAMPAGE INDUCER"),
            (PromptText::Key, true) => format!("HOLD [{key}] TO DEACTIVATE RAMPAGE INDUCER"),
            (PromptText::Effect, false) => {
                "Zombies move faster and spawn quicker. Survive the onslaught.".to_string()
            }
            (PromptText::Effect, true) => "Return the horde to its normal pace.".to_string(),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
    let wanted = if on { "DEACTIVATING" } else { "ACTIVATING" };
    if holding_text.0 != wanted {
        holding_text.0 = wanted.to_string();
    }
    if let Some(m) = materials.get_mut(&ring.0) {
        let p = (state.held / HOLD_SECS).clamp(0.0, 1.0);
        if (m.params.x - p).abs() > 1e-4 {
            m.params.x = p;
        }
    }
}

/// Keep the round number's lightning to the settings and the fade.
fn update_lightning(
    time: Res<Time>,
    settings: Res<RampageSettings>,
    state: Res<RampageLocal>,
    panels: Query<&MaterialNode<RampageLightningMaterial>, With<RoundLightning>>,
    mut materials: ResMut<Assets<RampageLightningMaterial>>,
) {
    let s = &*settings;
    for panel in &panels {
        let Some(m) = materials.get_mut(&panel.0) else { continue };
        let [r, g, b] = s.lightning_color;
        m.params = Vec4::new(time.elapsed_secs_wrapped(), state.fade, s.lightning_brightness, s.lightning_bolts);
        m.colour = Vec4::new(r, g, b, s.lightning_glow);
        m.look = Vec4::new(s.lightning_rate, s.lightning_jag, s.lightning_core, s.lightning_edge);
    }
}

// --- debug ----------------------------------------------------------------------------

/// The debug panel's "Rampage Inducer (Zombies)" section.
#[derive(SystemParam)]
pub(crate) struct RampageDebug<'w> {
    settings: ResMut<'w, RampageSettings>,
}

fn colour(ui: &mut egui::Ui, value: &mut [f32; 3], label: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.color_edit_button_rgb(value);
    });
}

impl RampageDebug<'_> {
    /// Its section in the main debug panel (`debug_ui`).
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        // (Edited on a copy, so it only counts as changed when something
        // really moved.)
        let mut s = (*self.settings).clone();
        ui.label("Placed in the level editor. Hold interact at it (1 s) to turn it on or off.");
        ui.checkbox(&mut s.preview, "Preview the lightning and the light as if it were on");
        ui.collapsing("Light", |ui| {
            colour(ui, &mut s.light_color, "colour");
            ui.add(egui::Slider::new(&mut s.light_intensity_off, 0.0f32..=500_000.0).text("intensity, off"));
            ui.add(egui::Slider::new(&mut s.light_intensity_on, 0.0f32..=500_000.0).text("intensity, on"));
            ui.add(egui::Slider::new(&mut s.light_range, 0.0f32..=40.0).text("range (m)"));
            ui.add(egui::Slider::new(&mut s.light_flicker, 0.0f32..=1.0).text("flicker while on"));
            ui.add(egui::Slider::new(&mut s.light_height, 0.0f32..=3.0).text("height (× the globe's)"));
        });
        ui.collapsing("Sound", |ui| {
            ui.add(egui::Slider::new(&mut s.start_volume, 0.0f32..=3.0).text("start sound"));
            ui.add(egui::Slider::new(&mut s.stop_volume, 0.0f32..=3.0).text("stop sound"));
            ui.add(egui::Slider::new(&mut s.music_volume, 0.0f32..=3.0).text("music"));
        });
        ui.collapsing("Round number lightning", |ui| {
            colour(ui, &mut s.lightning_color, "colour");
            ui.add(egui::Slider::new(&mut s.lightning_brightness, 0.0f32..=3.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.lightning_bolts, 0.0f32..=6.0).step_by(1.0).text("bolts"));
            ui.add(egui::Slider::new(&mut s.lightning_rate, 0.5f32..=30.0).text("reshapes a second"));
            ui.add(egui::Slider::new(&mut s.lightning_jag, 0.0f32..=0.5).text("jaggedness"));
            ui.add(egui::Slider::new(&mut s.lightning_core, 0.3f32..=8.0).text("core thickness (px)"));
            ui.add(egui::Slider::new(&mut s.lightning_glow, 0.5f32..=30.0).text("glow (px)"));
            ui.add(egui::Slider::new(&mut s.lightning_pad, 0.0f32..=1.5).text("reaches out round the number"));
            ui.add(egui::Slider::new(&mut s.lightning_edge, 0.0f32..=1.0).text("edge fade"));
            ui.add(egui::Slider::new(&mut s.fade_in_secs, 0.0f32..=5.0).text("fades in over (s)"));
            ui.add(egui::Slider::new(&mut s.fade_out_secs, 0.0f32..=5.0).text("fades out over (s)"));
        });
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                s = RampageSettings {
                    preview: s.preview,
                    ..default()
                };
            }
            if ui.button("Print settings to console").clicked() {
                info!("Rampage Inducer: {s:#?}");
            }
        });
        if s != *self.settings {
            *self.settings = s;
        }
    }
}
