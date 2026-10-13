//! `Zombies` doors (`shared::doors`): each of the map's doorways' door,
//! drawn with `models/props/door.glb` fitted to its size, solid while it's
//! shut, and sliding out of the way once someone's bought it open
//! (`Lobby::doors_open`) — for everyone. Near a shut one, a card (the power
//! card's style) shows its cost and what the interact key does; the key asks
//! the server to open it ([`shared::BuyDoor`]), which checks it all again.
//!
//! Each slides open with a sound (`GameSounds::door_open`) from the door,
//! heard by the whole lobby, fading with distance like other players'
//! sounds — not for one already open when we got here. Whoever bought it
//! also hears the purchase ching (`GameSounds::money_ching`, not positional):
//! we remember the door we asked for ([`DoorBuy`]) and ching when it opens.
//! The map's main door (`Door::main`) also plays its ambience
//! (`GameSounds::main_door_ambience`) from it, for everyone, heard much
//! further off ([`MAIN_DOOR_FULL_M`]..[`MAIN_DOOR_SILENT_M`]).
//!
//! The doors, their sounds and the card are `StateScoped(InGame)`, and the doors are taken
//! away whenever we're not in a `Zombies` game — nothing carries into the
//! next one. In the other modes every doorway's open.

use bevy::audio::{SpatialAudioSink, Volume};
use bevy::prelude::*;
use bevy_rapier3d::prelude::Collider;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::doors::{Door, Locked, MODEL_SIZE, OPEN_SECS};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::{zombies_game, CARD_RED, MONEY_YELLOW, PERK_CARD_WIDTH};
use crate::{killcam, menu, AppState, Player, EYE_HEIGHT, HUD_FONT};

pub(crate) const DOOR_MODEL: &str = "models/props/door.glb";

/// A door's colour on its card: old stone.
const DOOR_TAN: Color = Color::srgb(0.85, 0.72, 0.5);

pub(crate) struct DoorsPlugin;

impl Plugin for DoorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DoorBuy>()
            .add_systems(OnEnter(AppState::InGame), spawn_door_card)
            .add_systems(OnExit(AppState::InGame), |mut buy: ResMut<DoorBuy>| buy.0 = None)
            .add_systems(
                Update,
                (
                    sync_doors,
                    slide_doors,
                    fade_door_sounds,
                    update_door_card,
                    buy_door.run_if(menu::game_active.and(killcam::no_killcam)),
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            // (Ungated: what takes them away once the game's left.)
            .add_systems(Update, clear_doors.run_if(not(in_state(AppState::InGame))));
    }
}

/// `models/props/door.glb` fitted to a door `size` metres `(width, height,
/// depth)`: turned so its width runs across the doorway (it's modelled along
/// its own z) and stretched to fit — its base at the door's floor.
pub(crate) fn model_transform(size: Vec3) -> Transform {
    Transform {
        translation: Vec3::ZERO,
        rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        // (In the model's own axes: depth along x, height y, width z.)
        scale: Vec3::new(size.z / MODEL_SIZE.z, size.y / MODEL_SIZE.y, size.x / MODEL_SIZE.x),
    }
}

/// A door's root, where it stands in its doorway.
#[derive(Component)]
struct DoorRoot {
    index: usize,
    /// When it started sliding open (`Time::elapsed_secs`) — far in the past
    /// for one already open when we got here.
    opened_at: Option<f32>,
    /// Whether it's been seen once (so one open from the start doesn't slide).
    seen: bool,
}

/// What slides: the model and, while it's shut, the collider.
#[derive(Component)]
struct DoorPanel;

/// A shut door's collider.
#[derive(Component)]
struct DoorCollider;

/// The door we last asked to buy, and when — it's ours if it opens within
/// [`BUY_CONFIRM_SECS`].
#[derive(Resource, Default)]
struct DoorBuy(Option<(usize, f32)>);

/// How long (s) after asking a door opening still counts as our buy.
const BUY_CONFIRM_SECS: f32 = 2.0;

/// The main door's ambience: full volume within this far (m) of it, fading
/// to silence here — a good way across the map, not all of it.
const MAIN_DOOR_FULL_M: f32 = 15.0;
const MAIN_DOOR_SILENT_M: f32 = 60.0;

/// A door's sound as it slides open — and whether it's the main door's
/// ambience (its own long range) rather than the slide.
#[derive(Component)]
struct DoorOpenSound {
    ambience: bool,
}

/// Put the map's doors in while we're in a `Zombies` game (and take them
/// away otherwise).
fn sync_doors(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    doors: Query<Entity, With<DoorRoot>>,
    mut shown: Local<Option<shared::MapId>>,
    mut commands: Commands,
) {
    let map = zombies_game(&local, &lobbies).map(|l| l.map);
    if *shown == map && (map.is_none() || !doors.is_empty() || shared::doors::doors(map.unwrap()).is_empty()) {
        return;
    }
    for e in &doors {
        commands.entity(e).despawn();
    }
    *shown = map;
    let Some(map) = map else { return };
    for (index, door) in shared::doors::doors(map).iter().enumerate() {
        spawn_door(&mut commands, &asset_server, index, door);
    }
}

fn spawn_door(commands: &mut Commands, asset_server: &AssetServer, index: usize, door: &Door) {
    let half = door.size * 0.5;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            DoorRoot {
                index,
                opened_at: None,
                seen: false,
            },
            Transform::from_translation(door.at.pos).with_rotation(door.rotation()),
            Visibility::default(),
        ))
        .with_children(|root| {
            root.spawn((DoorPanel, Transform::default(), Visibility::default()))
                .with_children(|panel| {
                    panel.spawn((
                        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(DOOR_MODEL))),
                        model_transform(door.size),
                    ));
                    panel.spawn((
                        DoorCollider,
                        crate::vfx::NoBulletHoles,
                        Collider::cuboid(half.x, half.y, half.z),
                        Transform::from_xyz(0.0, half.y, 0.0),
                    ));
                });
        });
}

/// Take the doors away out of a game.
fn clear_doors(doors: Query<Entity, With<DoorRoot>>, mut commands: Commands) {
    for e in &doors {
        commands.entity(e).despawn();
    }
}

/// Slide each door open once it's bought — its collider gone the moment it
/// starts — and hide it once it's out of the way.
#[allow(clippy::type_complexity)]
fn slide_doors(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut doors: Query<(&mut DoorRoot, &Children)>,
    mut panels: Query<(&mut Transform, &mut Visibility, &Children), With<DoorPanel>>,
    colliders: Query<(), With<DoorCollider>>,
    sounds: Option<Res<crate::GameSounds>>,
    mut buy: ResMut<DoorBuy>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else { return };
    let now = time.elapsed_secs();
    let layout = shared::doors::doors(lobby.map);
    for (mut root, children) in &mut doors {
        let Some(door) = layout.get(root.index) else { continue };
        let open = shared::doors::is_open(lobby.doors_open, root.index);
        if open && root.opened_at.is_none() {
            // (Already open when we got here: just gone, silently.)
            root.opened_at = Some(if root.seen { now } else { now - OPEN_SECS });
            if let (true, Some(sounds)) = (root.seen, sounds.as_ref()) {
                let (center, ..) = door.solid_box();
                let clips = std::iter::once((sounds.door_open.clone(), false))
                    .chain(door.main.then(|| (sounds.main_door_ambience.clone(), true)));
                for (clip, ambience) in clips {
                    commands.spawn((
                        StateScoped(AppState::InGame),
                        DoorOpenSound { ambience },
                        // Volume is ours to set (`fade_door_sounds`), not the
                        // one-shot volume pass's.
                        crate::RemoteSoundEmitter,
                        AudioPlayer::new(clip),
                        Transform::from_translation(center),
                        // Starts silent; the real volume is set from the next
                        // frame.
                        crate::positional_playback(Volume::Linear(0.0)),
                    ));
                }
            }
            // Ours: the ching (the one-shot volume pass sets its loudness).
            if let (Some((asked, at)), Some(sounds)) = (buy.0, sounds.as_ref()) {
                if asked == root.index && now - at <= BUY_CONFIRM_SECS {
                    buy.0 = None;
                    commands.spawn((
                        StateScoped(AppState::InGame),
                        AudioPlayer::new(sounds.money_ching.clone()),
                        PlaybackSettings::DESPAWN,
                    ));
                }
            }
        }
        root.seen = true;
        let Some(opened_at) = root.opened_at else { continue };
        let t = ((now - opened_at) / OPEN_SECS).clamp(0.0, 1.0);
        // Eased: heavy to get going, settling at the end.
        let eased = t * t * (3.0 - 2.0 * t);
        for &child in children {
            let Ok((mut tf, mut vis, parts)) = panels.get_mut(child) else { continue };
            tf.translation = door.slide_offset(eased);
            vis.set_if_neq(if t >= 1.0 { Visibility::Hidden } else { Visibility::Inherited });
            for &part in parts {
                if colliders.contains(part) {
                    commands.entity(part).despawn();
                }
            }
        }
    }
}

/// Keep each door's opening sound at the loudness for how far the listener
/// is — the same fade as other players' sounds ("Remote sounds" range) —
/// times the "door open" volume; the main door's ambience over its own,
/// longer range.
fn fade_door_sounds(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    vols: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut sounds: Query<(&GlobalTransform, &DoorOpenSound, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();
    for (gt, sound, mut sink) in &mut sounds {
        let distance = ear.distance(gt.translation());
        let loudness = if sound.ambience {
            let t = ((distance - MAIN_DOOR_FULL_M) / (MAIN_DOOR_SILENT_M - MAIN_DOOR_FULL_M)).clamp(0.0, 1.0);
            vols.main_door_ambience * (1.0 - t) * (1.0 - t)
        } else {
            vols.door_open * crate::distance_falloff(distance, &remote)
        };
        sink.set_volume(Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

/// The shut door we're at, if any, and why it can't be bought if it can't.
fn at_door(lobby: &Lobby, feet: Vec3) -> Option<(usize, &'static Door, Option<Locked>)> {
    let index = shared::doors::door_at(lobby.map, feet, lobby.doors_open, 0.0)?;
    let door = shared::doors::doors(lobby.map).get(index)?;
    let locked = shared::doors::can_open(lobby.map, index, lobby.doors_open, lobby.power_on).err();
    Some((index, door, locked))
}

/// The interact key at a door we can open and afford: ask the server to
/// open it (it checks again, and sets its bit in `Lobby::doors_open`).
#[allow(clippy::too_many_arguments)]
fn buy_door(
    time: Res<Time>,
    mut buy: ResMut<DoorBuy>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut sender: Query<&mut TriggerSender<shared::BuyDoor>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let (Some(me), Some(lobby)) = (local.iter().next().map(|l| l.0), zombies_game(&local, &lobbies)) else {
        return;
    };
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let Some((index, door, None)) = at_door(lobby, feet) else { return };
    let points = lobby.members.iter().find(|m| m.peer == me).map_or(0, |m| m.score);
    if points < door.cost {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::BuyDoor(index as u8));
        buy.0 = Some((index, time.elapsed_secs()));
    }
}

// --- the card (at a shut door) ----------------------------------------------

#[derive(Component)]
struct DoorCard;

#[derive(Component)]
struct DoorCardAction;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum DoorCardText {
    Blurb,
    Cost,
    Points,
    Action,
}

fn spawn_door_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let body = asset_server.load(crate::BODY_FONT);
    let heading = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let faint = TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55));
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
                DoorCard,
                Node {
                    width: Val::Px(PERK_CARD_WIDTH),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(DOOR_TAN),
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
                    col.spawn((Text::new("DOOR"), heading(38.0), TextColor(DOOR_TAN)));
                    col.spawn((
                        DoorCardText::Blurb,
                        Text::new(""),
                        TextFont {
                            font: body.clone(),
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    ));
                });
                card.spawn((
                    Node {
                        height: Val::Px(1.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.15)),
                ));
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
                            col.spawn((DoorCardText::Cost, Text::new(""), heading(30.0), TextColor(MONEY_YELLOW)));
                        });
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::End,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("YOUR POINTS"), heading(16.0), faint));
                            col.spawn((DoorCardText::Points, Text::new(""), heading(30.0), TextColor::WHITE));
                        });
                });
                card.spawn((
                    DoorCardAction,
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((DoorCardText::Action, Text::new(""), heading(24.0), TextColor::WHITE));
            });
        });
}

/// Show the card while we're at a shut door (hidden behind menus and during
/// a kill cam, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_door_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<DoorCard>>,
    mut action_bar: Single<&mut BackgroundColor, With<DoorCardAction>>,
    mut texts: Query<(&DoorCardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let shown = (!menu.is_open() && active_killcam.0.is_none())
        .then(|| {
            let lobby = zombies_game(&local, &lobbies)?;
            let me = local.iter().next()?.0;
            let points = lobby.members.iter().find(|m| m.peer == me)?.score;
            Some((at_door(lobby, feet)?, points))
        })
        .flatten();
    let Some(((_, door, locked), points)) = shown else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);

    let needs_power = locked == Some(Locked::NeedsPower);
    let affordable = points >= door.cost;
    let (edge, bar, cost_color, points_color, action, action_color) = if needs_power {
        (
            CARD_RED,
            CARD_RED.with_alpha(0.2),
            MONEY_YELLOW,
            Color::WHITE,
            "TURN ON THE POWER FIRST".to_string(),
            CARD_RED,
        )
    } else if affordable {
        (
            DOOR_TAN,
            DOOR_TAN.with_alpha(0.25),
            MONEY_YELLOW,
            Color::WHITE,
            format!("PRESS {} TO OPEN", binds.interact.label().to_uppercase()),
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
    let blurb = if door.requires_power {
        "Sealed until the power's on. Open it to go deeper."
    } else {
        "Open it to reach more of the map."
    };
    if border.0 != edge {
        border.0 = edge;
    }
    if action_bar.0 != bar {
        action_bar.0 = bar;
    }
    for (which, mut text, mut color) in &mut texts {
        let (wanted, wanted_color) = match which {
            DoorCardText::Blurb => (blurb.to_string(), Color::srgba(1.0, 1.0, 1.0, 0.85)),
            DoorCardText::Cost => (format!("${}", crate::util::grouped(door.cost)), cost_color),
            DoorCardText::Points => (format!("${}", crate::util::grouped(points)), points_color),
            DoorCardText::Action => (action.clone(), action_color),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        if color.0 != wanted_color {
            color.0 = wanted_color;
        }
    }
}
