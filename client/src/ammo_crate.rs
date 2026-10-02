//! The `Zombies` ammo crate (`models/props/ammo_crate.glb`): stand at it with the
//! sniper out and not full, and a card (the perk card's style, pared down)
//! shows the price and our points; the interact key buys a refill
//! ([`shared::BuyAmmo`] → [`shared::AmmoBought`]), which fills the sniper's
//! mag and reserve (`Weapon::fill_mag_and_reserve`) and plays the buy sound
//! for us alone.
//!
//! Its placement is panel-tunable for now ("Zombies perks" → "Ammo crate"),
//! on this client only — no collision, and the server takes the client's word
//! that it's at the crate (see `shared::ammo`).
//!
//! The crate and card are `StateScoped(InGame)`; the crate is also taken
//! away whenever we're not in a `Zombies` game — nothing carries into the
//! next one.

use bevy::prelude::*;
use lightyear::prelude::{LocalId, MessageReceiver, TriggerSender};
use shared::{GameMode, Lobby};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::{zombies_game, CARD_RED, MONEY_YELLOW, PERK_CARD_WIDTH};
use crate::{
    killcam, menu, AppState, GameSounds, Player, Weapon, WeaponSlot, EYE_HEIGHT, HUD_FONT,
};

const AMMO_CRATE_MODEL: &str = "models/props/ammo_crate.glb";
/// How close (m, across the ground) our feet must be to the crate's spot.
const USE_RADIUS: f32 = 2.0;
/// How far (m) above / below the crate's spot our feet may be.
const USE_HEIGHT: f32 = 2.5;
/// Least time (s) between buy requests, so a double press before the
/// server's answer lands doesn't buy twice.
const BUY_COOLDOWN_SECS: f32 = 0.5;
/// The card's colour: an olive ammo-box green.
const AMMO_GREEN: Color = Color::srgb(0.55, 0.8, 0.35);

pub(crate) struct AmmoCratePlugin;

impl Plugin for AmmoCratePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AmmoCrateSettings>()
            .add_systems(OnEnter(AppState::InGame), spawn_ammo_card)
            .add_systems(
                Update,
                (
                    sync_ammo_crate,
                    update_ammo_card,
                    buy_ammo.run_if(menu::game_active.and(killcam::no_killcam)),
                    receive_ammo,
                )
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Panel-tunable ammo crate placement.
#[derive(Resource, Clone)]
pub(crate) struct AmmoCrateSettings {
    /// World position (m) of the model's origin.
    pub(crate) pos: Vec3,
    /// Degrees about x (pitch), y (turn) and z (roll).
    pub(crate) rotation_deg: Vec3,
    /// 1 = the model as it comes.
    pub(crate) scale: f32,
    /// Set by the panel's "move to me" button: `sync_ammo_crate` moves `pos`
    /// to our feet, then clears it.
    pub(crate) snap_to_player: bool,
}

impl Default for AmmoCrateSettings {
    fn default() -> Self {
        Self {
            pos: Vec3::new(9.6, 0.4, -35.0),
            rotation_deg: Vec3::new(0.0, -90.0, 0.0),
            scale: 1.0,
            snap_to_player: false,
        }
    }
}

impl AmmoCrateSettings {
    fn transform(&self) -> Transform {
        let r = self.rotation_deg;
        Transform::from_translation(self.pos)
            .with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                r.y.to_radians(),
                r.x.to_radians(),
                r.z.to_radians(),
            ))
            .with_scale(Vec3::splat(self.scale.max(1e-4)))
    }

    /// Whether feet at `feet` are close enough to use the crate.
    fn in_reach(&self, feet: Vec3) -> bool {
        let d = feet - self.pos;
        Vec2::new(d.x, d.z).length() <= USE_RADIUS && d.y.abs() <= USE_HEIGHT
    }
}

#[derive(Component)]
struct AmmoCrate;

/// Put the crate on the map while we're in a `Zombies` game (and take it
/// away otherwise), where the panel says.
fn sync_ammo_crate(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut settings: ResMut<AmmoCrateSettings>,
    player: Single<&Transform, (With<Player>, Without<AmmoCrate>)>,
    asset_server: Res<AssetServer>,
    mut crates: Query<(Entity, &mut Transform), With<AmmoCrate>>,
    mut commands: Commands,
) {
    if settings.snap_to_player {
        settings.snap_to_player = false;
        settings.pos = player.translation - Vec3::Y * EYE_HEIGHT;
    }
    // (Only placed on Break Point so far.)
    if !zombies_game(&local, &lobbies).is_some_and(|l| l.map.is_break_point()) {
        for (e, _) in &crates {
            commands.entity(e).despawn();
        }
        return;
    }
    if crates.is_empty() {
        commands.spawn((
            StateScoped(AppState::InGame),
            AmmoCrate,
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(AMMO_CRATE_MODEL))),
            settings.transform(),
        ));
        return;
    }
    for (_, mut t) in &mut crates {
        t.set_if_neq(settings.transform());
    }
}

/// Our points, if we're at the crate in a running `Zombies` game (on Break
/// Point, the only map it's placed on so far) with the
/// sniper out and short of ammo.
fn at_crate(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&Lobby>,
    settings: &AmmoCrateSettings,
    weapon: &Weapon,
    player: &Transform,
) -> Option<u32> {
    let lobby = zombies_game(local, lobbies).filter(|l| l.map.is_break_point())?;
    let me = local.iter().next()?.0;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if weapon.slot != WeaponSlot::Primary
        || weapon.ammo_full(GameMode::Zombies)
        || !settings.in_reach(feet)
    {
        return None;
    }
    lobby.members.iter().find(|m| m.peer == me).map(|m| m.score)
}

// --- the card ----------------------------------------------------------------

#[derive(Component)]
struct AmmoCard;

/// The strip along the card's bottom saying what the interact key does.
#[derive(Component)]
struct AmmoCardAction;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum AmmoCardText {
    Cost,
    Points,
    Action,
}

fn spawn_ammo_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
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
                AmmoCard,
                Node {
                    width: Val::Px(PERK_CARD_WIDTH * 0.75),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(AMMO_GREEN),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((Text::new("AMMO"), heading(38.0), TextColor(AMMO_GREEN)));
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
                            col.spawn((AmmoCardText::Cost, Text::new(""), heading(30.0), TextColor(MONEY_YELLOW)));
                        });
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::End,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("YOUR POINTS"), heading(16.0), faint));
                            col.spawn((AmmoCardText::Points, Text::new(""), heading(30.0), TextColor::WHITE));
                        });
                });
                card.spawn((
                    AmmoCardAction,
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((AmmoCardText::Action, Text::new(""), heading(24.0), TextColor::WHITE));
            });
        });
}

/// Show the card while we're at the crate with the sniper out and short of
/// ammo (hidden behind menus, during a kill cam and while dead).
#[allow(clippy::too_many_arguments)]
fn update_ammo_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    settings: Res<AmmoCrateSettings>,
    weapon: Res<Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<AmmoCard>>,
    mut action_bar: Single<&mut BackgroundColor, With<AmmoCardAction>>,
    mut texts: Query<(&AmmoCardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let points = (!menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .then(|| at_crate(&local, &lobbies, &settings, &weapon, &player))
        .flatten();
    let Some(points) = points else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);

    let cost = shared::ammo::AMMO_COST;
    let (edge, bar, cost_color, points_color, action, action_color) = if points >= cost {
        (
            AMMO_GREEN,
            AMMO_GREEN.with_alpha(0.25),
            MONEY_YELLOW,
            Color::WHITE,
            format!("PRESS {} TO BUY AMMO", binds.interact.label().to_uppercase()),
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
            AmmoCardText::Cost => (format!("${cost}"), cost_color),
            AmmoCardText::Points => (format!("${points}"), points_color),
            AmmoCardText::Action => (action.clone(), action_color),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        if color.0 != wanted_color {
            color.0 = wanted_color;
        }
    }
}

/// The interact key at the crate, if we can afford it: ask the server (it
/// checks the points again and takes them).
#[allow(clippy::too_many_arguments)]
fn buy_ammo(
    time: Res<Time>,
    mut last_buy: Local<Option<f32>>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    settings: Res<AmmoCrateSettings>,
    weapon: Res<Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut sender: Query<&mut TriggerSender<shared::BuyAmmo>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let now = time.elapsed_secs();
    if last_buy.is_some_and(|t| now - t < BUY_COOLDOWN_SECS) {
        return;
    }
    let affordable = at_crate(&local, &lobbies, &settings, &weapon, &player)
        .is_some_and(|points| points >= shared::ammo::AMMO_COST);
    if !affordable {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::BuyAmmo);
        *last_buy = Some(now);
    }
}

/// The server took our points: fill the sniper — a full mag too, so an
/// empty one doesn't need a manual reload — and play the buy sound —
/// just for us, not positional.
fn receive_ammo(
    mut receivers: Query<&mut MessageReceiver<shared::AmmoBought>>,
    mut weapon: ResMut<Weapon>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.fill_mag_and_reserve(GameMode::Zombies);
            commands.spawn((AudioPlayer::new(sounds.buy_ammo.clone()), PlaybackSettings::DESPAWN));
        }
    }
}
