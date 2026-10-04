//! `Zombies` wall buys (`shared::wall_buy`): a wooden sign
//! (`models/props/wooden_sign.glb`) with a glowing chalk outline of its gun
//! on the front (`textures/wall_buys/<gun>_outline.png`, baked from the
//! gun's HUD icon). Standing at one shows a card — the gun, its cost and
//! whether it can be bought: ready, too dear, or already carried, each
//! looking its own way. The interact key buys it ([`shared::BuyWallWeapon`]):
//! it takes the place of the weapon in hand, which the server drops where we
//! stand (`weapon_drops`), and on its answer ([`shared::WallWeaponBought`])
//! the new gun's drawn, full, with the buy sound.
//!
//! Where each sign stands is the map's layout's, placed in the level editor
//! (which draws them with [`decal`] too). The signs and the card are
//! `StateScoped(InGame)`, and the signs are taken away whenever we're not in
//! a `Zombies` game — nothing carries into the next one.

use bevy::prelude::*;
use lightyear::prelude::{LocalId, MessageReceiver, TriggerSender};
use shared::level::Placement;
use shared::weapon::{SlotWeapon, WeaponId, WALL_BUY_WEAPONS};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::{zombies_game, CARD_RED, MONEY_YELLOW};
use crate::{killcam, menu, AppState, GameSounds, Player, Weapon, EYE_HEIGHT, HUD_FONT};

pub(crate) const SIGN_MODEL: &str = "models/props/wooden_sign.glb";

/// Where the outline sits on the sign, in its own frame (m): just proud of
/// the board's front face (the board spans x -1.02..0.98, y 1.51..2.81, its
/// planks' fronts at z 0.13), and how wide it is.
const DECAL_CENTER: Vec3 = Vec3::new(-0.02, 2.16, 0.14);
const DECAL_WIDTH: f32 = 1.55;
/// How brightly the outline glows (it blooms).
const DECAL_GLOW: f32 = 4.0;

/// The outline's green, for the card's border when it can be bought.
const CHALK_GREEN: Color = Color::srgb(0.6, 1.0, 0.45);
const OWNED_GREY: Color = Color::srgb(0.55, 0.55, 0.58);

/// Least time (s) between buy requests, so a double press before the
/// server's answer can't buy twice.
const BUY_COOLDOWN_SECS: f32 = 0.6;

pub(crate) struct WallBuysPlugin;

impl Plugin for WallBuysPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_wall_buy_assets)
            .add_systems(OnEnter(AppState::InGame), spawn_wall_buy_card)
            .add_systems(
                Update,
                (
                    sync_wall_buy_signs,
                    update_wall_buy_card,
                    buy_wall_weapon.run_if(menu::game_active.and(killcam::no_killcam)),
                    receive_wall_buys,
                )
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Each gun's outline: its quad (the baked texture's shape) and glowing
/// material, made once.
#[derive(Resource)]
pub(crate) struct WallBuyAssets {
    decals: Vec<(WeaponId, Handle<Mesh>, Handle<StandardMaterial>)>,
}

fn outline_texture(gun: WeaponId) -> (&'static str, f32) {
    // (Path, and height / width of the baked image.)
    match gun {
        WeaponId::Ak74 => ("textures/wall_buys/ak_74_outline.png", 313.0 / 1024.0),
        _ => ("textures/wall_buys/sniper_outline.png", 406.0 / 1024.0),
    }
}

fn icon(gun: WeaponId) -> &'static str {
    match gun {
        WeaponId::Ak74 => "textures/icons/weapons/ak_74.png",
        _ => "textures/icons/weapons/sniper.png",
    }
}

fn load_wall_buy_assets(
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let decals = WALL_BUY_WEAPONS
        .into_iter()
        .map(|gun| {
            let (path, aspect) = outline_texture(gun);
            let texture: Handle<Image> = asset_server.load(path);
            let material = materials.add(StandardMaterial {
                base_color_texture: Some(texture.clone()),
                emissive_texture: Some(texture),
                emissive: LinearRgba::rgb(DECAL_GLOW, DECAL_GLOW, DECAL_GLOW),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            });
            (gun, meshes.add(Rectangle::new(DECAL_WIDTH, DECAL_WIDTH * aspect)), material)
        })
        .collect();
    commands.insert_resource(WallBuyAssets { decals });
}

/// `gun`'s outline on a sign, as a child of the sign's root.
pub(crate) fn decal(gun: WeaponId, assets: &WallBuyAssets) -> impl Bundle {
    let (mesh, material) = assets
        .decals
        .iter()
        .find(|(g, ..)| *g == gun)
        .map(|(_, m, mat)| (m.clone(), mat.clone()))
        .unwrap_or_default();
    (
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(DECAL_CENTER),
        bevy::pbr::NotShadowCaster,
    )
}

/// A wall buy's sign.
#[derive(Component)]
struct WallBuySign;

/// Put the signs up while we're in a `Zombies` game (and take them away
/// otherwise), where the map's layout says.
fn sync_wall_buy_signs(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    assets: Option<Res<WallBuyAssets>>,
    asset_server: Res<AssetServer>,
    signs: Query<Entity, With<WallBuySign>>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        for e in &signs {
            commands.entity(e).despawn();
        }
        return;
    };
    let Some(assets) = assets else { return };
    if !signs.is_empty() {
        return;
    }
    for wall_buy in &shared::level::layout(lobby.map).wall_buys {
        let at = wall_buy.at;
        commands
            .spawn((
                StateScoped(AppState::InGame),
                WallBuySign,
                Transform::from_translation(at.pos).with_rotation(at.rotation()),
                Visibility::default(),
            ))
            .with_children(|sign| {
                sign.spawn(SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(SIGN_MODEL))));
                sign.spawn(decal(wall_buy.weapon, &assets));
                // Solid: the post, and the board up top.
                sign.spawn((
                    bevy_rapier3d::prelude::Collider::cuboid(0.08, 0.8, 0.08),
                    Transform::from_xyz(0.0, 0.8, -0.03),
                ));
                sign.spawn((
                    bevy_rapier3d::prelude::Collider::cuboid(1.0, 0.65, 0.09),
                    Transform::from_xyz(-0.02, 2.16, 0.04),
                ));
            });
    }
}

/// Whether we can buy what's on a sign in reach.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Buyable,
    TooPoor,
    Owned,
}

/// The wall buy we're standing at (the nearest, if more than one's in
/// reach) in our `Zombies` game, whether we can buy it, and our points.
fn at_wall_buy(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&Lobby>,
    weapon: &Weapon,
    player: &Transform,
) -> Option<(WeaponId, Status, u32)> {
    let lobby = zombies_game(local, lobbies)?;
    let me = local.iter().next()?.0;
    let points = lobby.members.iter().find(|m| m.peer == me)?.score;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let spot = |at: Placement| shared::wall_buy::use_spot(at).distance_squared(feet);
    let wall_buy = shared::level::layout(lobby.map)
        .wall_buys
        .iter()
        .filter(|w| shared::wall_buy::in_range_of(w.at, feet, 0.0))
        .min_by(|a, b| spot(a.at).total_cmp(&spot(b.at)))?;
    let gun = wall_buy.weapon;
    let status = if weapon.carries(SlotWeapon::Gun(gun)) {
        Status::Owned
    } else if points >= shared::weapon::wall_buy_cost(gun) {
        Status::Buyable
    } else {
        Status::TooPoor
    };
    Some((gun, status, points))
}

/// Whether we're at a wall buy we can buy — the interact key's the sign's,
/// not a dropped weapon's (`knife_pickup`).
pub(crate) fn buyable_in_reach(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&Lobby>,
    weapon: &Weapon,
    player: &Transform,
) -> bool {
    at_wall_buy(local, lobbies, weapon, player).is_some_and(|(_, status, _)| status == Status::Buyable)
}

// --- the card ----------------------------------------------------------------

#[derive(Component)]
struct WallBuyCard;

#[derive(Component)]
struct WallBuyIcon;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum CardText {
    Name,
    Class,
    Cost,
    Status,
}

fn spawn_wall_buy_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let text = |size: f32, color: Color| {
        (
            TextFont {
                font: font.clone(),
                font_size: size,
                ..default()
            },
            TextColor(color),
        )
    };
    let dim = Color::srgba(1.0, 1.0, 1.0, 0.55);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(72.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                WallBuyCard,
                Node {
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.88)),
                BorderColor(CHALK_GREEN),
                BorderRadius::all(Val::Px(4.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                // The gun and its cost.
                card.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                        row_gap: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                ))
                .with_children(|left| {
                    left.spawn((
                        WallBuyIcon,
                        ImageNode::new(asset_server.load(icon(WeaponId::Sniper))),
                        Node {
                            width: Val::Px(150.0),
                            height: Val::Px(75.0),
                            ..default()
                        },
                    ));
                    left.spawn((CardText::Cost, Text::new(""), text(20.0, MONEY_YELLOW)));
                });
                // Its name and class, and what the interact key does.
                card.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    row_gap: Val::Px(2.0),
                    min_width: Val::Px(220.0),
                    ..default()
                })
                .with_children(|right| {
                    right.spawn((CardText::Name, Text::new(""), text(30.0, Color::WHITE)));
                    right.spawn((CardText::Class, Text::new(""), text(16.0, dim)));
                    right.spawn((CardText::Status, Text::new(""), text(20.0, Color::WHITE)));
                });
            });
        });
}

/// Show the card while we're at a wall buy (hidden behind menus, during a
/// kill cam and while dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_wall_buy_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    weapon: Res<Weapon>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<WallBuyCard>>,
    mut icon_node: Single<&mut ImageNode, With<WallBuyIcon>>,
    mut texts: Query<(&CardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let here = (!menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .then(|| at_wall_buy(&local, &lobbies, &weapon, &player))
        .flatten();
    let Some((gun, status, _)) = here else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    let (status_text, status_color, edge) = match status {
        Status::Buyable => (
            format!("PRESS {} TO BUY", binds.interact.label().to_uppercase()),
            Color::WHITE,
            CHALK_GREEN,
        ),
        Status::TooPoor => ("NOT ENOUGH POINTS".to_string(), CARD_RED, CARD_RED),
        Status::Owned => ("OWNED".to_string(), OWNED_GREY, OWNED_GREY),
    };
    border.set_if_neq(BorderColor(edge));
    let image = asset_server.load(icon(gun));
    if icon_node.image != image {
        icon_node.image = image;
    }
    // (Dimmed when there's nothing to buy.)
    icon_node.color = if status == Status::Owned { OWNED_GREY } else { Color::WHITE };
    for (which, mut text, mut color) in &mut texts {
        let (wanted, tint) = match which {
            CardText::Name => (gun.label().to_string(), if status == Status::Owned { OWNED_GREY } else { Color::WHITE }),
            CardText::Class => (gun.class_label().to_string(), Color::srgba(1.0, 1.0, 1.0, 0.55)),
            CardText::Cost => (
                format!("COST  {}", shared::weapon::wall_buy_cost(gun)),
                if status == Status::TooPoor { CARD_RED } else { MONEY_YELLOW },
            ),
            CardText::Status => (status_text.clone(), status_color),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        color.set_if_neq(TextColor(tint));
    }
}

/// The interact key at a wall buy we can buy, hands free: ask the server
/// for it, handing over the weapon in hand's rounds for the drop.
#[allow(clippy::too_many_arguments)]
fn buy_wall_weapon(
    time: Res<Time>,
    mut last_buy: Local<Option<f32>>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<Weapon>,
    hands: crate::weapons::Hands,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut sender: Query<&mut TriggerSender<shared::BuyWallWeapon>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) || !hands.free() {
        return;
    }
    let now = time.elapsed_secs();
    if last_buy.is_some_and(|t| now - t < BUY_COOLDOWN_SECS) {
        return;
    }
    let Some((gun, Status::Buyable, _)) = at_wall_buy(&local, &lobbies, &weapon, &player) else {
        return;
    };
    let (mag, reserve) = weapon.held_ammo();
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::BuyWallWeapon {
            weapon: gun,
            slot: weapon.held as u8,
            mag,
            reserve,
        });
        *last_buy = Some(now);
    }
}

/// The server took our points: the gun's in our hands (full), drawn — and
/// the buy sound, just for us.
fn receive_wall_buys(
    mut receivers: Query<&mut MessageReceiver<shared::WallWeaponBought>>,
    mut weapon: ResMut<Weapon>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for bought in rx.receive() {
            weapon.take_into_slot(bought.slot as usize, SlotWeapon::Gun(bought.weapon), None);
            commands.spawn((AudioPlayer::new(sounds.buy_ammo.clone()), PlaybackSettings::DESPAWN));
        }
    }
}
