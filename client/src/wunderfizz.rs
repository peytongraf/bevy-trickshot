//! Der Wunderfizz (`shared::wunderfizz`): the classic perk set's one machine
//! that sells every classic perk — the only place to get Death Perception and
//! PhD Flopper. It stands dark until round
//! [`shared::wunderfizz::ACTIVE_ROUND`], then fades in a purple light.
//!
//! At it, the interact key opens its menu ([`menu::Screen::Wunderfizz`],
//! which frees the cursor and freezes gameplay like the Pack-a-Punch's): every
//! classic perk's icon in a grid — owned / affordable / too expensive each
//! look different — with the hovered one's name, description and cost, and
//! our points. Clicking an affordable one asks the server for it
//! ([`shared::BuyPerk`] with `wunderfizz`); the menu stays open and the drink
//! plays behind it (`zombies_hud::sync_owned_perks` → `weapons::drink_arms`),
//! a second purchase cutting straight to the new drink. `Esc` / EXIT closes
//! it.
//!
//! Everything shown is read off the replicated lobby; the machine, prompt and
//! menu are `StateScoped(InGame)` and the menu closes whenever it stops
//! making sense (walked off, died, out of the game), so nothing carries over.

use bevy::audio::Volume;
use bevy::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::perks::{Perk, PerkSet};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::menu::{self, Screen};
use crate::net::GameClient;
use crate::ui::{ui_sound, UiSfx, UiSound};
use crate::zombies_hud::{perk_color, perk_icon_path, zombies_game, PerkMachineSettings, MONEY_YELLOW};
use crate::{killcam, AppState, Player, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) struct WunderfizzPlugin;

impl Plugin for WunderfizzPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WunderfizzMenu>()
            .add_systems(OnEnter(AppState::InGame), spawn_wunderfizz_prompt)
            .add_systems(OnExit(AppState::InGame), close_on_exit)
            .add_systems(
                Update,
                (
                    sync_wunderfizz_machine,
                    update_wunderfizz_prompt,
                    open_wunderfizz.run_if(menu::game_active.and(killcam::no_killcam)),
                    wunderfizz_lifecycle,
                    wunderfizz_input,
                    refresh_wunderfizz,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Its light's colour — Der Wunderfizz's purple.
const PURPLE: Color = Color::srgb(0.62, 0.22, 1.0);
/// Seconds its light takes to fade in once it wakes up.
const FADE_SECS: f32 = 2.5;

/// The open menu's state (meaningless while it's closed).
#[derive(Resource, Default)]
struct WunderfizzMenu {
    /// The perk the info panel shows: the one hovered last.
    hovered: Option<Perk>,
    /// EXIT clicked: close once the mouse button's up again, so the click
    /// can't carry on into a shot the moment gameplay input is back.
    close_pending: bool,
}

/// Where a perk stands for us.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PerkStatus {
    Owned,
    Buyable,
    TooPoor,
}

fn status(perk: Perk, owned: &[Perk], points: u32) -> PerkStatus {
    if owned.contains(&perk) {
        PerkStatus::Owned
    } else if points >= perk.cost() {
        PerkStatus::Buyable
    } else {
        PerkStatus::TooPoor
    }
}

/// Our lobby (a `Zombies` game with a Wunderfizz), points and perks, if we're
/// standing at the machine.
fn at_wunderfizz<'a>(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &'a Query<&Lobby>,
    machines: &PerkMachineSettings,
    player: &Transform,
) -> Option<(&'a Lobby, u32, &'a [Perk])> {
    let lobby = zombies_game(local, lobbies).filter(|l| shared::wunderfizz::present(l))?;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !shared::perks::in_range_of(machines.wunderfizz_placement(lobby.map).0, feet, 0.0) {
        return None;
    }
    let me = local.iter().next()?.0;
    let member = lobby.members.iter().find(|m| m.peer == me)?;
    Some((lobby, member.score, member.perks.as_slice()))
}

// --- the machine --------------------------------------------------------------

#[derive(Component)]
struct WunderfizzMachine;

#[derive(Component)]
struct WunderfizzModel;

#[derive(Component)]
struct WunderfizzLight;

/// Stand the machine on the map in a classic-perks `Zombies` game (and take it
/// away otherwise), where [`PerkMachineSettings`] says, its purple light
/// fading in once it's active.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_wunderfizz_machine(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    settings: Res<PerkMachineSettings>,
    asset_server: Res<AssetServer>,
    mut machines: Query<(Entity, &mut Transform), With<WunderfizzMachine>>,
    mut models: Query<&mut Transform, (With<WunderfizzModel>, Without<WunderfizzMachine>)>,
    mut lights: Query<(&mut PointLight, &mut Transform), (With<WunderfizzLight>, Without<WunderfizzMachine>, Without<WunderfizzModel>)>,
    // How far its light is faded in (0..=1); back to 0 whenever there's no
    // machine.
    mut glow: Local<f32>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies).filter(|l| shared::wunderfizz::present(l)) else {
        for (e, _) in &machines {
            commands.entity(e).despawn();
        }
        *glow = 0.0;
        return;
    };
    let (pos, yaw_deg) = settings.wunderfizz_placement(lobby.map);
    let place = Transform::from_translation(pos)
        .with_rotation(Quat::from_rotation_y(yaw_deg.to_radians()))
        .with_scale(Vec3::splat(shared::level::layout(lobby.map).wunderfizz.scale.max(0.01)));
    let half = settings.half_extents();
    if machines.is_empty() {
        commands
            .spawn((StateScoped(AppState::InGame), WunderfizzMachine, place, Visibility::default()))
            .with_children(|m| {
                m.spawn((
                    WunderfizzModel,
                    SceneRoot(
                        asset_server
                            .load(GltfAssetLabel::Scene(0).from_asset(crate::zombies_hud::WUNDERFIZZ_MODEL)),
                    ),
                    settings.wunderfizz_model_transform(),
                ));
                // Solid, like a perk machine (the server has the same box,
                // `server::collision`).
                m.spawn((
                    bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                    Transform::from_xyz(0.0, half.y, 0.0),
                ));
                m.spawn((
                    WunderfizzLight,
                    PointLight {
                        intensity: 0.0,
                        shadows_enabled: false,
                        ..settings.light.point_light(PURPLE)
                    },
                    Transform::from_translation(settings.light.offset),
                ));
            });
        return;
    }
    for (_, mut t) in &mut machines {
        t.set_if_neq(place);
    }
    for mut t in &mut models {
        t.set_if_neq(settings.wunderfizz_model_transform());
    }

    let target = if shared::wunderfizz::active(lobby) { 1.0 } else { 0.0 };
    let step = time.delta_secs() / FADE_SECS;
    *glow += (target - *glow).clamp(-step, step);
    let fade = *glow * *glow * (3.0 - 2.0 * *glow);
    for (mut light, mut t) in &mut lights {
        let mut lit = settings.light.point_light(PURPLE);
        lit.intensity *= fade;
        lit.shadows_enabled &= fade > 0.0;
        if light.intensity != lit.intensity || settings.is_changed() {
            *light = lit;
        }
        t.set_if_neq(Transform::from_translation(settings.light.offset));
    }
}

// --- the prompt at the machine --------------------------------------------------

#[derive(Component)]
struct WunderfizzPrompt;

#[derive(Component)]
struct WunderfizzPromptText;

const PROMPT_TITLE_BG: Color = Color::srgba(0.04, 0.04, 0.05, 0.92);
const PROMPT_LINE_BG: Color = Color::srgba(0.24, 0.07, 0.38, 0.9);

fn spawn_wunderfizz_prompt(mut commands: Commands, asset_server: Res<AssetServer>) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            WunderfizzPrompt,
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(PROMPT_TITLE_BG),
            ))
            .with_child((
                Text::new("DER WUNDERFIZZ"),
                TextFont {
                    font: heading,
                    font_size: 40.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(40.0), Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(PROMPT_LINE_BG),
            ))
            .with_child((
                WunderfizzPromptText,
                Text::new(""),
                TextFont {
                    font: body,
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.92, 1.0)),
            ));
        });
}

/// Show the prompt while we're at the machine (hidden behind menus, during a
/// kill cam and once dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_wunderfizz_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    machines: Res<PerkMachineSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut prompt: Single<&mut Visibility, With<WunderfizzPrompt>>,
    mut text: Single<&mut Text, With<WunderfizzPromptText>>,
) {
    let here = player
        .filter(|_| !menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .and_then(|p| at_wunderfizz(&local, &lobbies, &machines, &p));
    let Some((lobby, ..)) = here else {
        prompt.set_if_neq(Visibility::Hidden);
        return;
    };
    prompt.set_if_neq(Visibility::Inherited);
    let line = if shared::wunderfizz::active(lobby) {
        format!("Press {} to choose a perk", binds.interact.label().to_uppercase())
    } else {
        format!("Activates on round {}", shared::wunderfizz::ACTIVE_ROUND)
    };
    if text.0 != line {
        text.0 = line;
    }
}

/// The interact key at the machine once it's active: open the menu.
#[allow(clippy::too_many_arguments)]
fn open_wunderfizz(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    machines: Res<PerkMachineSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<WunderfizzMenu>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let Some(player) = player else {
        return;
    };
    let Some((lobby, ..)) = at_wunderfizz(&local, &lobbies, &machines, &player) else {
        return;
    };
    if !shared::wunderfizz::active(lobby) {
        return;
    }
    *state = WunderfizzMenu::default();
    menu.screen = Screen::Wunderfizz;
    menu.dirty = true;
}

// --- the menu ----------------------------------------------------------------------

#[derive(Component)]
struct WunderfizzMenuRoot;

/// A perk's cell in the grid (a button).
#[derive(Component, Clone, Copy)]
struct WfCell(Perk);

#[derive(Component, Clone, Copy)]
struct WfCellIcon(Perk);

/// The line under a cell's icon: OWNED, or its cost.
#[derive(Component, Clone, Copy)]
struct WfCellStatus(Perk);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum WfText {
    Name,
    Description,
    Cost,
    Points,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum WfButton {
    Exit,
}

const PANEL_W: f32 = 600.0;
const CELL: f32 = 124.0;
const ICON: f32 = 84.0;
const TITLE_BG: Color = Color::srgb(0.3, 0.08, 0.5);
const INFO_BG: Color = Color::srgba(0.06, 0.04, 0.09, 0.95);
const BODY_BG: Color = Color::srgba(0.1, 0.07, 0.14, 0.94);
const LIGHT_TEXT: Color = Color::srgb(0.94, 0.92, 0.97);
const FAINT_TEXT: Color = Color::srgba(0.94, 0.92, 0.97, 0.6);
const OWNED_GREEN: Color = Color::srgb(0.35, 0.85, 0.45);
const POOR_RED: Color = Color::srgb(0.9, 0.3, 0.3);

/// A key chip ("ESC") and its label — the footer's controls.
fn spawn_control(
    parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    which: WfButton,
    key: &str,
    label: &str,
    heading: &Handle<Font>,
) {
    let mut button = parent.spawn((
        which,
        Button,
        Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        BorderRadius::all(Val::Px(6.0)),
    ));
    if which == WfButton::Exit {
        button.insert(ui_sound(UiSound::BUTTON_BACK));
    }
    button.with_children(|b| {
        b.spawn((
            Node {
                min_width: Val::Px(40.0),
                height: Val::Px(32.0),
                padding: UiRect::horizontal(Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BorderColor(LIGHT_TEXT),
            BorderRadius::all(Val::Px(16.0)),
        ))
        .with_child((
            Text::new(key),
            TextFont {
                font: heading.clone(),
                font_size: 20.0,
                ..default()
            },
            TextColor(LIGHT_TEXT),
        ));
        b.spawn((
            Text::new(label),
            TextFont {
                font: heading.clone(),
                font_size: 30.0,
                ..default()
            },
            TextColor(LIGHT_TEXT),
        ));
    });
}

fn spawn_cell(
    parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    perk: Perk,
    asset_server: &AssetServer,
    heading: &Handle<Font>,
) {
    parent
        .spawn((
            WfCell(perk),
            Button,
            Node {
                width: Val::Px(CELL),
                height: Val::Px(CELL),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                border: UiRect::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor(Color::NONE),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_children(|cell| {
            cell.spawn((
                WfCellIcon(perk),
                ImageNode::new(asset_server.load(perk_icon_path(perk))),
                Node {
                    width: Val::Px(ICON),
                    height: Val::Px(ICON),
                    ..default()
                },
            ));
            cell.spawn((
                WfCellStatus(perk),
                Text::new(""),
                TextFont {
                    font: heading.clone(),
                    font_size: 20.0,
                    ..default()
                },
                TextColor(LIGHT_TEXT),
            ));
        });
}

fn spawn_wunderfizz_menu(commands: &mut Commands, asset_server: &AssetServer) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    let hfont = |size: f32| TextFont {
        font: heading.clone(),
        font_size: size,
        ..default()
    };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            WunderfizzMenuRoot,
            // Over the HUD, like the pause menu.
            GlobalZIndex(50),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                // Centred on the screen.
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.0, 0.05, 0.3)),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(PANEL_W),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    ..default()
                },
                BorderRadius::all(Val::Px(6.0)),
            ))
            .with_children(|panel| {
                // Title.
                panel
                    .spawn((
                        Node {
                            justify_content: JustifyContent::Center,
                            padding: UiRect::axes(Val::Px(16.0), Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(TITLE_BG),
                        BorderRadius::top(Val::Px(6.0)),
                    ))
                    .with_child((Text::new("DER WUNDERFIZZ"), hfont(46.0), TextColor(Color::WHITE)));
                // The hovered perk: its name, what it does and its cost.
                panel
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(4.0),
                            padding: UiRect::axes(Val::Px(22.0), Val::Px(14.0)),
                            min_height: Val::Px(124.0),
                            ..default()
                        },
                        BackgroundColor(INFO_BG),
                    ))
                    .with_children(|info| {
                        info.spawn(Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((WfText::Name, Text::new(""), hfont(42.0), TextColor(LIGHT_TEXT)));
                            row.spawn((WfText::Cost, Text::new(""), hfont(34.0), TextColor(MONEY_YELLOW)));
                        });
                        info.spawn((
                            WfText::Description,
                            Text::new(""),
                            TextFont {
                                font: body.clone(),
                                font_size: 19.0,
                                ..default()
                            },
                            TextColor(FAINT_TEXT),
                            TextLayout::default().with_linebreak(LineBreak::WordBoundary),
                        ));
                    });
                // The grid of perks, then the controls and our points.
                panel
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::new(Val::Px(22.0), Val::Px(22.0), Val::Px(18.0), Val::Px(14.0)),
                            row_gap: Val::Px(18.0),
                            ..default()
                        },
                        BackgroundColor(BODY_BG),
                        BorderRadius::bottom(Val::Px(6.0)),
                    ))
                    .with_children(|b| {
                        b.spawn(Node {
                            display: Display::Grid,
                            grid_template_columns: RepeatedGridTrack::px(4, CELL),
                            column_gap: Val::Px(12.0),
                            row_gap: Val::Px(12.0),
                            justify_content: JustifyContent::Center,
                            ..default()
                        })
                        .with_children(|grid| {
                            for &perk in PerkSet::Classic.perks() {
                                spawn_cell(grid, perk, asset_server, &heading);
                            }
                        });
                        b.spawn(Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|f| {
                            f.spawn(Node {
                                column_gap: Val::Px(24.0),
                                align_items: AlignItems::Center,
                                ..default()
                            })
                            .with_children(|c| {
                                spawn_control(c, WfButton::Exit, "ESC", "EXIT", &heading);
                            });
                            f.spawn((WfText::Points, Text::new(""), hfont(44.0), TextColor(MONEY_YELLOW)));
                        });
                    });
            });
        });
}

/// Build the menu when it opens and take it down when it closes; close it
/// once an EXIT click is released, or as soon as it no longer makes sense
/// (away from the machine, dead, out of the game).
#[allow(clippy::too_many_arguments)]
fn wunderfizz_lifecycle(
    mut menu: ResMut<menu::Menu>,
    mut state: ResMut<WunderfizzMenu>,
    mouse: Res<ButtonInput<MouseButton>>,
    death: Res<crate::death_effect::DeathEffect>,
    machines: Res<PerkMachineSettings>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    roots: Query<Entity, With<WunderfizzMenuRoot>>,
    mut commands: Commands,
) {
    if menu.screen == Screen::Wunderfizz {
        // The mouse back button exits like EXIT does (once it's released).
        if mouse.just_pressed(MouseButton::Back) {
            state.close_pending = true;
        }
        let still_here = !death.is_active()
            && player.is_some_and(|p| {
                at_wunderfizz(&local, &lobbies, &machines, &p).is_some_and(|(l, ..)| shared::wunderfizz::active(l))
            });
        let released = state.close_pending && mouse.get_pressed().next().is_none();
        if !still_here || released {
            menu.screen = Screen::None;
            menu.dirty = true;
            state.close_pending = false;
        }
    }
    let show = menu.screen == Screen::Wunderfizz && !state.close_pending;
    match (show, roots.is_empty()) {
        (true, true) => spawn_wunderfizz_menu(&mut commands, &asset_server),
        (false, false) => {
            for e in &roots {
                commands.entity(e).despawn();
            }
        }
        _ => {}
    }
}

/// Hovering a cell shows it in the info panel; clicking one (or PURCHASE, for
/// the one shown) buys it if we can — the menu stays open and the drink
/// plays — or plays the denied sound if we can't. EXIT closes it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn wunderfizz_input(
    menu: Res<menu::Menu>,
    mut state: ResMut<WunderfizzMenu>,
    sfx: Option<Res<UiSfx>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    cells: Query<(&WfCell, &Interaction)>,
    pressed_cells: Query<(&WfCell, &Interaction), Changed<Interaction>>,
    buttons: Query<(&WfButton, &Interaction), Changed<Interaction>>,
    mut sender: Query<&mut TriggerSender<shared::BuyPerk>, With<GameClient>>,
    mut commands: Commands,
) {
    if menu.screen != Screen::Wunderfizz || state.close_pending {
        return;
    }
    let hovered = cells.iter().find(|(_, i)| **i != Interaction::None).map(|(c, _)| c.0);
    if let Some(perk) = hovered.filter(|p| Some(*p) != state.hovered) {
        state.hovered = Some(perk);
        if let Some(sfx) = &sfx {
            commands.spawn((
                AudioPlayer::new(sfx.button_hover.clone()),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.6)),
            ));
        }
    }
    let mut buy: Option<Perk> = None;
    for (cell, interaction) in &pressed_cells {
        if *interaction == Interaction::Pressed {
            buy = Some(cell.0);
        }
    }
    for (button, interaction) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            WfButton::Exit => state.close_pending = true,
        }
    }
    let Some(perk) = buy else {
        return;
    };

    let me = local.iter().next().map(|l| l.0);
    let member = zombies_game(&local, &lobbies).and_then(|l| l.members.iter().find(|m| Some(m.peer) == me));
    let buyable = member.is_some_and(|m| status(perk, &m.perks, m.score) == PerkStatus::Buyable);
    let sent = buyable
        && sender.single_mut().is_ok_and(|mut s| {
            s.trigger::<shared::LobbyChannel>(shared::BuyPerk { perk, wunderfizz: true });
            true
        });
    if sent {
        // (The buy sound, jingle and drink follow once the server's sale
        // comes back — `zombies_hud::sync_owned_perks`.)
        info!("asked the server for {} from Der Wunderfizz", perk.label());
    } else if let Some(sfx) = &sfx {
        commands.spawn((AudioPlayer::new(sfx.denied.clone()), PlaybackSettings::DESPAWN));
    }
}

/// Keep the open menu's cells, info panel and points matching where we
/// stand.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn refresh_wunderfizz(
    menu: Res<menu::Menu>,
    state: Res<WunderfizzMenu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut cells: Query<(&WfCell, &Interaction, &mut BackgroundColor, &mut BorderColor), Without<WfButton>>,
    mut icons: Query<(&WfCellIcon, &mut ImageNode)>,
    mut statuses: Query<(&WfCellStatus, &mut Text, &mut TextColor), Without<WfText>>,
    mut texts: Query<(&WfText, &mut Text, &mut TextColor), Without<WfCellStatus>>,
    mut buttons: Query<(&WfButton, &Interaction, &mut BackgroundColor), Without<WfCell>>,
) {
    if menu.screen != Screen::Wunderfizz {
        return;
    }
    let me = local.iter().next().map(|l| l.0);
    let member = zombies_game(&local, &lobbies).and_then(|l| l.members.iter().find(|m| Some(m.peer) == me));
    let (owned, points): (&[Perk], u32) = member.map_or((&[], 0), |m| (m.perks.as_slice(), m.score));
    let status = |perk: Perk| status(perk, owned, points);

    for (cell, interaction, mut bg, mut border) in &mut cells {
        let hovered = *interaction != Interaction::None || state.hovered == Some(cell.0);
        let (fill, edge) = match status(cell.0) {
            // Owned: a quiet green wash.
            PerkStatus::Owned => (OWNED_GREEN.with_alpha(0.16), OWNED_GREEN.with_alpha(0.7)),
            // Affordable: lit up, edged in the perk's own colour.
            PerkStatus::Buyable => (Color::srgba(1.0, 1.0, 1.0, 0.12), perk_color(cell.0)),
            // Too expensive: dark, no edge.
            PerkStatus::TooPoor => (Color::srgba(0.0, 0.0, 0.0, 0.45), Color::srgba(1.0, 1.0, 1.0, 0.08)),
        };
        let (fill, edge) = if hovered {
            (fill.mix(&Color::WHITE, 0.12), Color::WHITE)
        } else {
            (fill, edge)
        };
        bg.set_if_neq(BackgroundColor(fill));
        border.set_if_neq(BorderColor(edge));
    }
    for (icon, mut image) in &mut icons {
        let tint = match status(icon.0) {
            PerkStatus::Owned => Color::srgba(1.0, 1.0, 1.0, 0.45),
            PerkStatus::Buyable => Color::WHITE,
            PerkStatus::TooPoor => Color::srgba(0.45, 0.45, 0.45, 0.8),
        };
        if image.color != tint {
            image.color = tint;
        }
    }
    for (s, mut text, mut color) in &mut statuses {
        let (line, c) = match status(s.0) {
            PerkStatus::Owned => ("OWNED".to_string(), OWNED_GREEN),
            PerkStatus::Buyable => (format!("${}", crate::util::grouped(s.0.cost())), MONEY_YELLOW),
            PerkStatus::TooPoor => (format!("${}", crate::util::grouped(s.0.cost())), POOR_RED),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }

    let shown = state.hovered;
    for (which, mut text, mut color) in &mut texts {
        let (line, c) = match (which, shown) {
            (WfText::Name, Some(p)) => (p.label().to_uppercase(), perk_color(p)),
            (WfText::Name, None) => ("CHOOSE A PERK".to_string(), LIGHT_TEXT),
            (WfText::Description, Some(p)) => (p.description().to_string(), FAINT_TEXT),
            (WfText::Description, None) => ("Hover over a perk to see what it does.".to_string(), FAINT_TEXT),
            (WfText::Cost, Some(p)) => match status(p) {
                PerkStatus::Owned => ("OWNED".to_string(), OWNED_GREEN),
                PerkStatus::Buyable => (format!("${}", crate::util::grouped(p.cost())), MONEY_YELLOW),
                PerkStatus::TooPoor => (format!("${}", crate::util::grouped(p.cost())), POOR_RED),
            },
            (WfText::Cost, None) => (String::new(), MONEY_YELLOW),
            (WfText::Points, _) => (format!("${}", crate::util::grouped(points)), MONEY_YELLOW),
        };
        if text.0 != line {
            text.0 = line;
        }
        color.set_if_neq(TextColor(c));
    }
    let shown_buyable = shown.is_some_and(|p| status(p) == PerkStatus::Buyable);
    for (button, interaction, mut bg) in &mut buttons {
        // PURCHASE only lights up when there's something to buy.
        let live = *button == WfButton::Exit || shown_buyable;
        let c = if live && *interaction != Interaction::None {
            Color::srgba(1.0, 1.0, 1.0, 0.12)
        } else {
            Color::NONE
        };
        bg.set_if_neq(BackgroundColor(c));
    }
}

/// Leaving the game with the menu somehow still up: close it.
fn close_on_exit(mut menu: ResMut<menu::Menu>, mut state: ResMut<WunderfizzMenu>) {
    if menu.screen == Screen::Wunderfizz {
        menu.screen = Screen::None;
        menu.dirty = true;
    }
    *state = WunderfizzMenu::default();
}
