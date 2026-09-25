//! `Zombies`-only client pieces: the "enemies left" counter (left edge,
//! vertically centred), every member's points / health / name panel (bottom
//! left, ours at the bottom of the stack), the perk machines (placeholder boxes for now) with
//! the card shown while standing at one, and switching on what an owned perk does
//! (Shroom Tea: the shroom screen effect; Nitro Brew: the [`NitroBrew`]
//! speed multipliers).
//!
//! Everything reads the replicated `Lobby` — the server owns the round, the
//! count, points and purchases (`server::zombies`) — so nothing here needs
//! resetting between games: it's all derived fresh each frame, and the
//! spawned UI / machines are `StateScoped(InGame)`.

use bevy::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::perks::Perk;
use shared::{GameMode, Lobby};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::{killcam, menu, AppState, GameSounds, Player, ShroomPerk, EYE_HEIGHT, HUD_FONT};

pub(crate) struct ZombiesHudPlugin;

impl Plugin for ZombiesHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), spawn_zombies_hud)
            .add_systems(
                Update,
                (
                    update_enemies_left,
                    sync_perk_machines,
                    update_perk_card,
                    update_perk_icons,
                    update_party_panels,
                    buy_perk.run_if(menu::game_active.and(killcam::no_killcam)),
                )
                    .run_if(in_state(AppState::InGame)),
            )
            .init_resource::<NitroBrew>()
            // Not gated on `InGame`: it's what switches the effects back *off*
            // once the game is left.
            .add_systems(Update, sync_owned_perks);
    }
}

/// Each perk's colour — its machine and its bottle in the drinking arms.
pub(crate) fn perk_color(perk: Perk) -> Color {
    match perk {
        Perk::ShroomTea => Color::srgb_u8(0x6a, 0x1f, 0xbf),
        Perk::NitroBrew => Color::srgb_u8(0xff, 0xd4, 0x00),
    }
}

/// Nitro Brew's multipliers (the debug panel's "Nitro Brew" section) and
/// whether we own it right now (`sync_owned_perks`, so it's off again the
/// moment a game ends or is left). Each accessor is the multiplier while the
/// perk's active, `1.0` otherwise.
#[derive(Resource)]
pub(crate) struct NitroBrew {
    pub(crate) owned: bool,
    /// Debug: act as if owned, to tune without buying it. Never saved.
    pub(crate) debug_force: bool,
    pub(crate) move_mult: f32,
    pub(crate) ads_mult: f32,
    pub(crate) reload_mult: f32,
    pub(crate) rechamber_mult: f32,
    pub(crate) swap_mult: f32,
}

impl Default for NitroBrew {
    fn default() -> Self {
        Self {
            owned: false,
            debug_force: false,
            move_mult: 1.15,
            ads_mult: 1.5,
            reload_mult: 1.5,
            rechamber_mult: 1.5,
            swap_mult: 1.5,
        }
    }
}

impl NitroBrew {
    fn pick(&self, mult: f32) -> f32 {
        if self.owned || self.debug_force {
            mult.max(0.01)
        } else {
            1.0
        }
    }
    pub(crate) fn movement(&self) -> f32 {
        self.pick(self.move_mult)
    }
    pub(crate) fn ads(&self) -> f32 {
        self.pick(self.ads_mult)
    }
    pub(crate) fn reload(&self) -> f32 {
        self.pick(self.reload_mult)
    }
    pub(crate) fn rechamber(&self) -> f32 {
        self.pick(self.rechamber_mult)
    }
    pub(crate) fn swap(&self) -> f32 {
        self.pick(self.swap_mult)
    }
}

/// Our lobby, if we're in one.
fn my_lobby<'a>(local: &Query<&LocalId, With<GameClient>>, lobbies: &'a Query<&Lobby>) -> Option<&'a Lobby> {
    let me = local.iter().next()?.0;
    lobbies.iter().find(|l| l.has(me))
}

/// Our lobby while it's a running `Zombies` game.
fn zombies_game<'a>(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &'a Query<&Lobby>,
) -> Option<&'a Lobby> {
    my_lobby(local, lobbies).filter(|l| l.started && l.mode == GameMode::Zombies)
}

// --- enemies left ------------------------------------------------------

#[derive(Component)]
struct EnemiesLeftRoot;

#[derive(Component)]
struct EnemiesLeftCount;

/// The "enemies left" counter, party panels, owned-perk icons and the
/// (hidden until needed) perk card.
fn spawn_zombies_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    // Left edge, centred vertically: a full-height (row) container that
    // centres its one child on the cross axis — `align_items: Center` rather
    // than the default stretch, so the panel's background only wraps its text.
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|col| {
            col.spawn((
                EnemiesLeftRoot,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("ENEMIES LEFT"),
                    TextFont {
                        font: font.clone(),
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
                ));
                panel.spawn((
                    EnemiesLeftCount,
                    Text::new(""),
                    TextFont {
                        font: font.clone(),
                        font_size: 40.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
        });

    // The party's panels: bottom left, stacked upward.
    commands.spawn((
        StateScoped(AppState::InGame),
        PartyRoot,
        GlobalZIndex(5),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(24.0),
            bottom: Val::Px(24.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            ..default()
        },
        Visibility::Hidden,
    ));

    // Owned perks' icons: bottom centre, lifted off the edge.
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(PERK_ICON_BOTTOM),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(10.0),
                ..default()
            },
        ))
        .with_children(|row| {
            // One slot per perk there is, filled in purchase order by
            // `update_perk_icons`; unused slots take no room, so the row
            // stays centred however many are owned.
            for index in 0..Perk::ALL.len() {
                row.spawn((
                    PerkIconSlot { index, shown: None },
                    ImageNode::default(),
                    Node {
                        width: Val::Px(PERK_ICON_SIZE),
                        height: Val::Px(PERK_ICON_SIZE),
                        display: Display::None,
                        ..default()
                    },
                ));
            }
        });

    spawn_perk_card(&mut commands, &asset_server, font);
}

fn update_enemies_left(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut root: Single<&mut Visibility, With<EnemiesLeftRoot>>,
    mut count: Single<&mut Text, With<EnemiesLeftCount>>,
) {
    let left = zombies_game(&local, &lobbies)
        .filter(|l| l.round > 0)
        .map(|l| l.enemies_left);
    root.set_if_neq(if left.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    let wanted = left.map(|n| n.to_string()).unwrap_or_default();
    if count.0 != wanted {
        count.0 = wanted;
    }
}

// --- perk machines -----------------------------------------------------

/// A perk machine's stand-in model (a glowing box until there's a real one).
#[derive(Component)]
struct PerkMachine(Perk);

/// Placeholder machine size (m): width, height, depth.
const MACHINE_SIZE: Vec3 = Vec3::new(1.0, 2.1, 0.8);

/// Put each perk's machine on the map while we're in a `Zombies` game (and
/// take it away otherwise).
fn sync_perk_machines(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    machines: Query<(Entity, &PerkMachine)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        for (e, _) in &machines {
            commands.entity(e).despawn();
        }
        return;
    };
    for perk in Perk::ALL {
        if !machines.iter().any(|(_, m)| m.0 == perk) {
            spawn_perk_machine(perk, lobby.map, &mut meshes, &mut materials, &mut commands);
        }
    }
}

/// One perk's placeholder machine: a box in a dark shade of the perk's
/// colour, with a glowing sign and a soft light in its full colour.
fn spawn_perk_machine(
    perk: Perk,
    map: shared::MapId,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
) {
    let color = perk_color(perk);
    let lin = color.to_linear();
    let base = perk.machine_pos(map);
    let body = materials.add(StandardMaterial {
        base_color: Color::LinearRgba(lin * 0.3).with_alpha(1.0),
        perceptual_roughness: 0.5,
        ..default()
    });
    let sign = materials.add(StandardMaterial {
        base_color: color,
        emissive: lin * 4.0,
        ..default()
    });
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PerkMachine(perk),
            Transform::from_translation(base + Vec3::Y * MACHINE_SIZE.y * 0.5),
            Visibility::default(),
            Mesh3d(meshes.add(Cuboid::from_size(MACHINE_SIZE))),
            MeshMaterial3d(body),
        ))
        .with_children(|m| {
            // A glowing sign panel across the top of the front...
            m.spawn((
                Mesh3d(meshes.add(Cuboid::new(MACHINE_SIZE.x * 0.85, 0.4, 0.05))),
                MeshMaterial3d(sign),
                Transform::from_xyz(0.0, MACHINE_SIZE.y * 0.5 - 0.35, MACHINE_SIZE.z * 0.5 + 0.03),
            ));
            // ...and a soft light so it reads from a distance.
            m.spawn((
                PointLight {
                    color,
                    intensity: 60_000.0,
                    range: 6.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, MACHINE_SIZE.y * 0.5 + 0.2, 0.0),
            ));
        });
}

// --- perk card (at a machine) -------------------------------------------

/// The card shown while standing at a perk machine: centred, a little below
/// the crosshair. Built once per game and filled in by `update_perk_card`.
#[derive(Component)]
struct PerkCard;

#[derive(Component)]
struct PerkCardIcon;

/// The strip along the card's bottom saying what the interact key does.
#[derive(Component)]
struct PerkCardAction;

/// Which of the card's texts a `Text` node is.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PerkCardText {
    Name,
    Description,
    Ingredients,
    Cost,
    Points,
    Action,
}

const PERK_CARD_WIDTH: f32 = 440.0;
const PERK_CARD_ICON: f32 = 64.0;
const CARD_RED: Color = Color::srgb(0.9, 0.3, 0.3);
const CARD_GREEN: Color = Color::srgb(0.35, 0.85, 0.45);

fn spawn_perk_card(commands: &mut Commands, asset_server: &AssetServer, font: Handle<Font>) {
    let body = asset_server.load(crate::BODY_FONT);
    let heading = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let plain = |size: f32| TextFont {
        font: body.clone(),
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
                PerkCard,
                Node {
                    width: Val::Px(PERK_CARD_WIDTH),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(Color::WHITE),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                // Icon beside the name and what it does.
                card.spawn(Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(14.0),
                    ..default()
                })
                .with_children(|header| {
                    header.spawn((
                        PerkCardIcon,
                        ImageNode::default(),
                        Node {
                            width: Val::Px(PERK_CARD_ICON),
                            height: Val::Px(PERK_CARD_ICON),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                    header
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            flex_grow: 1.0,
                            flex_shrink: 1.0,
                            row_gap: Val::Px(2.0),
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((PerkCardText::Name, Text::new(""), heading(38.0), TextColor::WHITE));
                            col.spawn((
                                PerkCardText::Description,
                                Text::new(""),
                                plain(14.0),
                                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                            ));
                        });
                });
                card.spawn(divider.clone());
                card.spawn((Text::new("INGREDIENTS"), heading(17.0), faint));
                card.spawn((
                    PerkCardText::Ingredients,
                    Text::new(""),
                    plain(13.0),
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.75)),
                    TextLayout::default().with_linebreak(LineBreak::WordBoundary),
                ));
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
                            col.spawn((PerkCardText::Cost, Text::new(""), heading(30.0), TextColor(MONEY_YELLOW)));
                        });
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::End,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("YOUR POINTS"), heading(16.0), faint));
                            col.spawn((PerkCardText::Points, Text::new(""), heading(30.0), TextColor::WHITE));
                        });
                });
                card.spawn((
                    PerkCardAction,
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((PerkCardText::Action, Text::new(""), heading(24.0), TextColor::WHITE));
            });
        });
}

// --- party panels (bottom left) ------------------------------------------

/// The column holding every member's panel.
#[derive(Component)]
struct PartyRoot;

/// A panel's parts, each for the member `.0`.
#[derive(Component)]
struct PartyMoney(lightyear::prelude::PeerId);
#[derive(Component)]
struct PartyHealthFill(lightyear::prelude::PeerId);
#[derive(Component)]
struct PartyHealthText(lightyear::prelude::PeerId);

/// Health bar width / height (px).
const HEALTH_BAR_W: f32 = 240.0;
const HEALTH_BAR_H: f32 = 7.0;
const MONEY_YELLOW: Color = Color::srgb(1.0, 0.82, 0.1);

/// One member's panel: a yellow `$` and their points; a white health bar over
/// a grey track, the number beside it; their name.
fn spawn_party_panel(
    root: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    font: &Handle<Font>,
    peer: lightyear::prelude::PeerId,
    name: &str,
) {
    let text = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let shadow = TextShadow {
        offset: Vec2::splat(1.5),
        color: Color::srgba(0.0, 0.0, 0.0, 0.75),
    };
    root.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(4.0),
        ..default()
    })
    .with_children(|panel| {
        panel
            .spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((Text::new("$"), text(26.0), TextColor(MONEY_YELLOW), shadow));
                row.spawn((PartyMoney(peer), Text::new("0"), text(26.0), TextColor(Color::WHITE), shadow));
            });
        panel
            .spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(14.0),
                ..default()
            })
            .with_children(|row| {
                // The grey track, with the white fill inside it.
                row.spawn((
                    Node {
                        width: Val::Px(HEALTH_BAR_W),
                        height: Val::Px(HEALTH_BAR_H),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.45, 0.45, 0.45, 0.8)),
                    BorderRadius::all(Val::Px(2.0)),
                ))
                .with_child((
                    PartyHealthFill(peer),
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                    BorderRadius::all(Val::Px(2.0)),
                ));
                row.spawn((PartyHealthText(peer), Text::new("100"), text(28.0), TextColor(Color::WHITE), shadow));
            });
        panel.spawn((Text::new(name.to_string()), text(20.0), TextColor(Color::WHITE), shadow));
    });
}

/// Keep the party's panels matching the lobby — rebuilt only when who's in it
/// (or their names) changes, ours always last so it sits at the bottom — and
/// fill in everyone's points and health every frame.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_party_panels(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    health: Query<(&shared::PlayerId, &shared::PlayerHealth)>,
    // Which column the panels were last built into, and for whom — the column
    // is respawned every game, so a new one always gets rebuilt.
    mut layout: Local<(Option<Entity>, Vec<(lightyear::prelude::PeerId, String)>)>,
    root: Single<(Entity, &mut Visibility), With<PartyRoot>>,
    mut money: Query<(&PartyMoney, &mut Text), Without<PartyHealthText>>,
    mut fills: Query<(&PartyHealthFill, &mut Node)>,
    mut health_text: Query<(&PartyHealthText, &mut Text), Without<PartyMoney>>,
    mut commands: Commands,
) {
    let (root, mut vis) = root.into_inner();
    let me = local.iter().next().map(|l| l.0);
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(if !menu.is_open() && active_killcam.0.is_none() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });

    // Everyone else in lobby order, then us.
    let mut wanted: Vec<(lightyear::prelude::PeerId, String)> = lobby
        .members
        .iter()
        .filter(|m| m.bot.is_none() && Some(m.peer) != me)
        .map(|m| (m.peer, m.name.clone()))
        .collect();
    wanted.extend(
        lobby
            .members
            .iter()
            .filter(|m| Some(m.peer) == me)
            .map(|m| (m.peer, m.name.clone())),
    );
    if layout.0 != Some(root) || layout.1 != wanted {
        let font = asset_server.load(HUD_FONT);
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).with_children(|col| {
            for (peer, name) in &wanted {
                spawn_party_panel(col, &font, *peer, name);
            }
        });
        *layout = (Some(root), wanted);
        return; // filled in from next frame, once they exist
    }

    for (m, mut text) in &mut money {
        let points = lobby.members.iter().find(|x| x.peer == m.0).map_or(0, |x| x.score);
        let s = points.to_string();
        if text.0 != s {
            text.0 = s;
        }
    }
    let hp = |peer| {
        health
            .iter()
            .find(|(id, _)| id.0 == peer)
            .map_or(shared::health::FULL_HEALTH, |(_, h)| h.0)
            .clamp(0.0, shared::health::FULL_HEALTH)
    };
    for (f, mut node) in &mut fills {
        let pct = Val::Percent(hp(f.0) / shared::health::FULL_HEALTH * 100.0);
        if node.width != pct {
            node.width = pct;
        }
    }
    for (h, mut text) in &mut health_text {
        let s = format!("{:.0}", hp(h.0).ceil());
        if text.0 != s {
            text.0 = s;
        }
    }
}

/// Owned-perk icon size and its gap from the bottom of the screen (px).
const PERK_ICON_SIZE: f32 = 64.0;
const PERK_ICON_BOTTOM: f32 = 36.0;

/// One perk's icon in the bottom-centre row, shown while we own it.
/// The `index`th slot in the bottom-centre icon row: the icon of the
/// `index`th perk we bought (`LobbyMember::perks` is in purchase order), so
/// the first bought sits leftmost.
#[derive(Component)]
struct PerkIconSlot {
    index: usize,
    /// Which perk's icon it's showing, so the image is only swapped on change.
    shown: Option<Perk>,
}

/// A perk's HUD icon.
fn perk_icon_path(perk: Perk) -> &'static str {
    match perk {
        Perk::ShroomTea => "textures/shroom_tea_icon.png",
        Perk::NitroBrew => "textures/nitro_brew_icon.png",
    }
}

/// Fill the icon row with the perks we own in a running `Zombies` game, in
/// the order we bought them (hidden behind menus and during a kill cam, like
/// the rest of the HUD).
fn update_perk_icons(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut slots: Query<(&mut PerkIconSlot, &mut ImageNode, &mut Visibility, &mut Node)>,
) {
    let me = local.iter().next().map(|l| l.0);
    let owned: &[Perk] = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or(&[], |m| m.perks.as_slice());
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    for (mut slot, mut image, mut vis, mut node) in &mut slots {
        let perk = owned.get(slot.index).copied();
        if slot.shown != perk {
            slot.shown = perk;
            if let Some(perk) = perk {
                image.image = asset_server.load(perk_icon_path(perk));
            }
        }
        vis.set_if_neq(if hud_up && perk.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let display = if perk.is_some() { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

/// Where we stand with the perk at the machine we're at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PerkStatus {
    Buyable,
    TooPoor,
    Owned,
}

/// The machine we're standing at (if any), how that perk stands for us, and
/// our points. `None` if we're not at a machine or not in the lobby.
fn perk_here(lobby: &Lobby, me: lightyear::prelude::PeerId, feet: Vec3) -> Option<(Perk, PerkStatus, u32)> {
    let member = lobby.members.iter().find(|m| m.peer == me)?;
    let perk = Perk::ALL
        .into_iter()
        .find(|&p| shared::perks::in_range(p, lobby.map, feet, 0.0))?;
    let status = if member.perks.contains(&perk) {
        PerkStatus::Owned
    } else if member.score >= perk.cost() {
        PerkStatus::Buyable
    } else {
        PerkStatus::TooPoor
    };
    Some((perk, status, member.score))
}

/// Fill in and show the perk card while we're at a machine (hidden behind
/// menus and during a kill cam, like the rest of the HUD). It's the same card
/// for every perk: red when we can't afford it, green once we own it.
#[allow(clippy::too_many_arguments)]
fn update_perk_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<PerkCard>>,
    mut icon: Single<&mut ImageNode, With<PerkCardIcon>>,
    mut action_bar: Single<&mut BackgroundColor, With<PerkCardAction>>,
    mut texts: Query<(&PerkCardText, &mut Text, &mut TextColor)>,
    // The perk the card was last filled in for (the icon / ingredients only
    // change with it). The card's respawned each game, so this is reset
    // whenever it's hidden.
    mut shown: Local<Option<Perk>>,
) {
    let (mut vis, mut border) = card.into_inner();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let here = (!menu.is_open() && active_killcam.0.is_none())
        .then(|| {
            let lobby = zombies_game(&local, &lobbies)?;
            perk_here(lobby, local.iter().next()?.0, feet)
        })
        .flatten();
    let Some((perk, status, points)) = here else {
        vis.set_if_neq(Visibility::Hidden);
        *shown = None;
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    if *shown != Some(perk) {
        *shown = Some(perk);
        icon.image = asset_server.load(perk_icon_path(perk));
    }

    let (edge, bar, cost_color, points_color, action, action_color) = match status {
        PerkStatus::Buyable => (
            perk_color(perk),
            perk_color(perk).with_alpha(0.25),
            MONEY_YELLOW,
            Color::WHITE,
            format!("PRESS {} TO BUY", binds.interact.label().to_uppercase()),
            Color::WHITE,
        ),
        PerkStatus::TooPoor => (
            CARD_RED,
            CARD_RED.with_alpha(0.2),
            CARD_RED,
            CARD_RED,
            "NOT ENOUGH POINTS".to_string(),
            CARD_RED,
        ),
        PerkStatus::Owned => (
            CARD_GREEN,
            CARD_GREEN.with_alpha(0.2),
            Color::srgba(1.0, 1.0, 1.0, 0.45),
            Color::WHITE,
            "ALREADY OWNED".to_string(),
            CARD_GREEN,
        ),
    };
    if border.0 != edge {
        border.0 = edge;
    }
    if action_bar.0 != bar {
        action_bar.0 = bar;
    }
    // Greyed out while we can't afford it; full colour otherwise.
    let tint = if status == PerkStatus::TooPoor {
        Color::srgba(0.6, 0.6, 0.6, 0.8)
    } else {
        Color::WHITE
    };
    if icon.color != tint {
        icon.color = tint;
    }

    for (which, mut text, mut color) in &mut texts {
        let (wanted, wanted_color) = match which {
            PerkCardText::Name => (perk.label().to_uppercase(), perk_color(perk)),
            PerkCardText::Description => (perk.description().to_string(), color.0),
            PerkCardText::Ingredients => (
                perk.ingredients()
                    .iter()
                    .map(|i| format!("\u{2022} {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                color.0,
            ),
            PerkCardText::Cost => (format!("${}", perk.cost()), cost_color),
            PerkCardText::Points => (format!("${points}"), points_color),
            PerkCardText::Action => (action.clone(), action_color),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        if color.0 != wanted_color {
            color.0 = wanted_color;
        }
    }
}

/// The interact key at a machine we can afford: ask the server to sell it to
/// us (it re-checks everything and takes the points).
fn buy_perk(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut sender: Query<&mut TriggerSender<shared::BuyPerk>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        return;
    };
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let Some((perk, PerkStatus::Buyable, _)) = perk_here(lobby, me, feet) else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::BuyPerk { perk });
        // (The server logs "bought ..." when it goes through; the card
        // switches to owned once that's replicated back.)
        info!("asked the server to buy {}", perk.label());
    }
}

/// Switch on what each perk does for as long as we own it in a running
/// `Zombies` game — Shroom Tea's shroom effect, Nitro Brew's multipliers —
/// and off again the moment we don't (game over, left, or not in a game at
/// all). A perk newly in our list is our purchase going through, so that's
/// when the buy sound plays and the drinking arms drink it — for us only,
/// since it keys off our own member's perks.
#[allow(clippy::too_many_arguments)]
fn sync_owned_perks(
    state: Res<State<AppState>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    mut shroom: ResMut<ShroomPerk>,
    mut nitro: ResMut<NitroBrew>,
    mut drink: ResMut<crate::PerkDrink>,
    // What we owned last frame (empty out of a game, so a new game starts
    // clean).
    mut prev: Local<Vec<Perk>>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    let owned: Vec<Perk> = if *state.get() == AppState::InGame {
        zombies_game(&local, &lobbies)
            .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
            .map(|m| m.perks.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if *prev == owned {
        return;
    }
    for &perk in owned.iter().filter(|p| !prev.contains(p)) {
        // Stow the weapon and drink it (`weapons::drink_arms`).
        drink.requested = Some(perk);
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.perk_buy.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
    let has_shroom = owned.contains(&Perk::ShroomTea);
    if shroom.0 != has_shroom {
        shroom.0 = has_shroom;
    }
    let has_nitro = owned.contains(&Perk::NitroBrew);
    if nitro.owned != has_nitro {
        nitro.owned = has_nitro;
    }
    *prev = owned;
}
