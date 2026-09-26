//! The map's lights and the `Zombies` power switch that turns them on.
//!
//! Break Point Night has [`MapLightSettings`]' lights (tuned from the debug
//! panel, "Map lights"). In `Zombies` they start off: a player pays at the
//! switch (a red placeholder cube for now — `shared::power`) and the server
//! sets `Lobby::power_on`, which fades them in for everyone. In the other
//! modes there's no switch, so they're simply on.
//!
//! The lights and the switch are `StateScoped(InGame)`, and the fade level
//! drops back to 0 whenever there are no lights — nothing carries into the
//! next game.

use bevy::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::{GameMode, Lobby, MapId};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::{my_lobby, zombies_game, CARD_RED, MONEY_YELLOW, PERK_CARD_WIDTH};
use crate::{killcam, menu, AppState, CurrentMap, Player, EYE_HEIGHT, HUD_FONT};

pub(crate) struct PowerPlugin;

impl Plugin for PowerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLightSettings>()
            .init_resource::<PowerLevel>()
            .add_systems(OnEnter(AppState::InGame), spawn_power_card)
            .add_systems(
                Update,
                (
                    sync_power_switch,
                    update_power_card,
                    turn_on_power.run_if(menu::game_active.and(killcam::no_killcam)),
                )
                    .run_if(in_state(AppState::InGame)),
            )
            // Not gated on `InGame`: it's what takes the lights away (and
            // resets the fade) once the game is left.
            .add_systems(Update, sync_map_lights);
    }
}

// --- map lights ------------------------------------------------------------

/// One of the map's lights.
#[derive(Clone, Copy)]
pub(crate) struct MapLight {
    pub(crate) enabled: bool,
    /// World position (m).
    pub(crate) pos: Vec3,
    pub(crate) color: [f32; 3],
    /// Lumens, at full power.
    pub(crate) intensity: f32,
    /// How far (m) it reaches.
    pub(crate) range: f32,
    /// Size of the glowing source (m) — softens highlights and shadows.
    pub(crate) radius: f32,
    pub(crate) shadows: bool,
}

/// Panel-tunable map lights ("Map lights (Break Point Night)").
#[derive(Resource, Clone)]
pub(crate) struct MapLightSettings {
    pub(crate) lights: [MapLight; 2],
    /// Seconds for the lights to fade in once the power's on.
    pub(crate) fade_secs: f32,
    /// Debug: act as if the power's on (untick and tick again to replay the
    /// fade).
    pub(crate) force_on: bool,
}

impl Default for MapLightSettings {
    fn default() -> Self {
        let light = |pos, color, intensity, range| MapLight {
            enabled: true,
            pos,
            color,
            intensity,
            range,
            radius: 0.3,
            shadows: true,
        };
        Self {
            lights: [
                light(Vec3::new(0.0, 18.0, -38.0), [1.0, 1.0, 1.0], 30_000_000.0, 90.0),
                light(Vec3::new(0.0, 18.0, 27.0), [1.0, 0.983, 0.959], 0.0, 60.0),
            ],
            fade_secs: 5.0,
            force_on: false,
        }
    }
}

/// How far the map lights are faded in, 0..=1.
#[derive(Resource, Default)]
struct PowerLevel(f32);

/// Which of [`MapLightSettings::lights`] a light is.
#[derive(Component)]
struct MapLightIndex(usize);

/// Put the map's lights up while in a game on a map that has them, fade them
/// toward on / off, and keep them matching the panel.
#[allow(clippy::too_many_arguments)]
fn sync_map_lights(
    state: Res<State<AppState>>,
    current: Res<CurrentMap>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    settings: Res<MapLightSettings>,
    time: Res<Time>,
    mut level: ResMut<PowerLevel>,
    mut lights: Query<(Entity, &MapLightIndex, &mut PointLight, &mut Transform)>,
    fresh: Query<(), Added<MapLightIndex>>,
    mut commands: Commands,
) {
    let wanted = *state.get() == AppState::InGame && current.0 == MapId::BreakPointNight;
    if !wanted {
        for (e, ..) in &lights {
            commands.entity(e).despawn();
        }
        level.0 = 0.0;
        return;
    }
    if lights.is_empty() {
        for i in 0..settings.lights.len() {
            commands.spawn((
                StateScoped(AppState::InGame),
                MapLightIndex(i),
                PointLight {
                    intensity: 0.0,
                    ..default()
                },
                Transform::default(),
            ));
        }
    }

    // In `Zombies` the power has to be turned on (and fades in); anywhere
    // else there's no switch, so they're just on.
    let lobby = my_lobby(&local, &lobbies);
    let zombies = lobby.is_some_and(|l| l.mode == GameMode::Zombies);
    let on = settings.force_on || !zombies || lobby.is_some_and(|l| l.power_on);
    let before = level.0;
    let target = if on { 1.0 } else { 0.0 };
    level.0 = if !zombies || settings.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / settings.fade_secs;
        level.0 + (target - level.0).clamp(-step, step)
    };
    if level.0 == before && !settings.is_changed() && fresh.is_empty() {
        return;
    }
    let fade = level.0 * level.0 * (3.0 - 2.0 * level.0);
    for (_, i, mut light, mut t) in &mut lights {
        let Some(s) = settings.lights.get(i.0) else {
            continue;
        };
        *light = PointLight {
            color: Color::srgb(s.color[0], s.color[1], s.color[2]),
            intensity: if s.enabled { s.intensity * fade } else { 0.0 },
            range: s.range,
            radius: s.radius,
            shadows_enabled: s.shadows && s.enabled && fade > 0.0,
            ..default()
        };
        t.translation = s.pos;
    }
}

// --- the switch ------------------------------------------------------------

/// The power switch's placeholder: a red cube.
#[derive(Component)]
struct PowerSwitch;

/// Placeholder cube's size (m).
const SWITCH_SIZE: f32 = 0.6;

/// Put the switch on the map while we're in a `Zombies` game on a map that
/// has one (and take it away otherwise).
fn sync_power_switch(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    switches: Query<Entity, With<PowerSwitch>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let pos = zombies_game(&local, &lobbies).and_then(|l| shared::power::switch_pos(l.map));
    let Some(pos) = pos else {
        for e in &switches {
            commands.entity(e).despawn();
        }
        return;
    };
    if !switches.is_empty() {
        return;
    }
    let red = Color::srgb(0.85, 0.08, 0.08);
    commands.spawn((
        StateScoped(AppState::InGame),
        PowerSwitch,
        Mesh3d(meshes.add(Cuboid::from_length(SWITCH_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: red,
            // A faint glow so it can be found in the dark.
            emissive: red.to_linear() * 1.5,
            ..default()
        })),
        Transform::from_translation(pos + Vec3::Y * SWITCH_SIZE * 0.5),
    ));
}

/// Whether we're at the switch with the power still off, and our points.
fn at_switch(lobby: &Lobby, me: lightyear::prelude::PeerId, feet: Vec3) -> Option<u32> {
    if lobby.power_on || !shared::power::in_range(lobby.map, feet, 0.0) {
        return None;
    }
    lobby.members.iter().find(|m| m.peer == me).map(|m| m.score)
}

/// The interact key at the switch, if we can afford it: ask the server to
/// turn the power on (it checks again, and flips `Lobby::power_on`).
fn turn_on_power(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut sender: Query<&mut TriggerSender<shared::TurnOnPower>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let (Some(me), Some(lobby)) = (local.iter().next().map(|l| l.0), zombies_game(&local, &lobbies)) else {
        return;
    };
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !at_switch(lobby, me, feet).is_some_and(|points| points >= shared::power::POWER_COST) {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::TurnOnPower);
    }
}

// --- the card (at the switch) ------------------------------------------------

/// The card shown at the switch while the power's off — the perk card's
/// style (`zombies_hud`), with the cost, our points and what the key does.
#[derive(Component)]
struct PowerCard;

/// The strip along the card's bottom saying what the interact key does.
#[derive(Component)]
struct PowerCardAction;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PowerCardText {
    Cost,
    Points,
    Action,
}

/// The power's colour on the card: a warm electric amber.
const POWER_AMBER: Color = Color::srgb(1.0, 0.72, 0.15);

fn spawn_power_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let body = asset_server.load(crate::BODY_FONT);
    let heading = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let faint = TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55));
    let divider = (
        Node {
            height: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.15)),
    );
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                PowerCard,
                Node {
                    width: Val::Px(PERK_CARD_WIDTH),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(POWER_AMBER),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((Text::new("POWER"), heading(38.0), TextColor(POWER_AMBER)));
                    col.spawn((
                        Text::new("The power is off. Turn it on to light up the map."),
                        TextFont {
                            font: body.clone(),
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    ));
                });
                card.spawn(divider);
                // Cost on the left, our points on the right.
                card.spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::End,
                    ..default()
                })
                .with_children(|footer| {
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("COST"), heading(16.0), faint));
                            col.spawn((PowerCardText::Cost, Text::new(""), heading(30.0), TextColor(MONEY_YELLOW)));
                        });
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::End,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("YOUR POINTS"), heading(16.0), faint));
                            col.spawn((PowerCardText::Points, Text::new(""), heading(30.0), TextColor::WHITE));
                        });
                });
                card.spawn((
                    PowerCardAction,
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((PowerCardText::Action, Text::new(""), heading(24.0), TextColor::WHITE));
            });
        });
}

/// Show the card while we're at the switch and the power's off (hidden behind
/// menus and during a kill cam, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_power_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<PowerCard>>,
    mut action_bar: Single<&mut BackgroundColor, With<PowerCardAction>>,
    mut texts: Query<(&PowerCardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let points = (!menu.is_open() && active_killcam.0.is_none())
        .then(|| at_switch(zombies_game(&local, &lobbies)?, local.iter().next()?.0, feet))
        .flatten();
    let Some(points) = points else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);

    let cost = shared::power::POWER_COST;
    let affordable = points >= cost;
    let (edge, bar, cost_color, points_color, action, action_color) = if affordable {
        (
            POWER_AMBER,
            POWER_AMBER.with_alpha(0.25),
            MONEY_YELLOW,
            Color::WHITE,
            format!("PRESS {} TO TURN ON THE POWER", binds.interact.label().to_uppercase()),
            Color::WHITE,
        )
    } else {
        (
            CARD_RED,
            CARD_RED.with_alpha(0.2),
            CARD_RED,
            CARD_RED,
            "NOT ENOUGH POINTS".to_string(),
            CARD_RED,
        )
    };
    if border.0 != edge {
        border.0 = edge;
    }
    if action_bar.0 != bar {
        action_bar.0 = bar;
    }
    for (which, mut text, mut color) in &mut texts {
        let (wanted, wanted_color) = match which {
            PowerCardText::Cost => (format!("${cost}"), cost_color),
            PowerCardText::Points => (format!("${points}"), points_color),
            PowerCardText::Action => (action.clone(), action_color),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        if color.0 != wanted_color {
            color.0 = wanted_color;
        }
    }
}
