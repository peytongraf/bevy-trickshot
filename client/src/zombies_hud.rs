//! `Zombies`-only client pieces: the "enemies left" counter (left edge,
//! vertically centred), every member's points / health / name panel (bottom
//! left, ours at the bottom of the stack), the perk machines (placeholder boxes for now) with
//! their "press F to buy" prompt, and switching on what an owned perk does
//! (Shroom Tea: the shroom screen effect).
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
                    update_perk_prompt,
                    update_perk_icons,
                    update_party_panels,
                    buy_perk.run_if(menu::game_active.and(killcam::no_killcam)),
                )
                    .run_if(in_state(AppState::InGame)),
            )
            // Not gated on `InGame`: it's what switches the effect back *off*
            // once the game is left.
            .add_systems(Update, sync_shroom_perk);
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

/// The "enemies left" counter and the (empty until needed) perk prompt.
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
        .with_child((
            PerkIcon(Perk::ShroomTea),
            ImageNode::new(asset_server.load("textures/shroom_tea_icon.png")),
            Node {
                width: Val::Px(PERK_ICON_SIZE),
                height: Val::Px(PERK_ICON_SIZE),
                ..default()
            },
            Visibility::Hidden,
        ));

    // The perk prompt: centred, a little below the crosshair.
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(60.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_child((
            PerkPrompt,
            Text::new(""),
            TextFont {
                font,
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::WHITE),
            TextShadow {
                offset: Vec2::splat(1.5),
                color: Color::srgba(0.0, 0.0, 0.0, 0.8),
            },
        ));
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
    let perk = Perk::ShroomTea;
    if machines.iter().any(|(_, m)| m.0 == perk) {
        return;
    }
    let base = perk.machine_pos(lobby.map);
    let body = materials.add(StandardMaterial {
        base_color: Color::srgb(0.32, 0.12, 0.42),
        perceptual_roughness: 0.5,
        ..default()
    });
    let sign = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.5, 1.0),
        emissive: LinearRgba::rgb(3.0, 1.2, 4.0),
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
                    color: Color::srgb(0.85, 0.5, 1.0),
                    intensity: 60_000.0,
                    range: 6.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, MACHINE_SIZE.y * 0.5 + 0.2, 0.0),
            ));
        });
}

#[derive(Component)]
struct PerkPrompt;

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
#[derive(Component)]
struct PerkIcon(Perk);

/// Show each perk's icon while we own it in a running `Zombies` game (hidden
/// behind menus and during a kill cam, like the rest of the HUD).
fn update_perk_icons(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut icons: Query<(&PerkIcon, &mut Visibility)>,
) {
    let me = local.iter().next().map(|l| l.0);
    let owned: &[Perk] = zombies_game(&local, &lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or(&[], |m| m.perks.as_slice());
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    for (icon, mut vis) in &mut icons {
        vis.set_if_neq(if hud_up && owned.contains(&icon.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// What (if anything) we can buy where we're standing: the perk and whether
/// we can afford it. `None` if we're not at a machine, already own its perk,
/// or aren't in a `Zombies` game.
fn perk_here(lobby: &Lobby, me: lightyear::prelude::PeerId, feet: Vec3) -> Option<(Perk, bool)> {
    let member = lobby.members.iter().find(|m| m.peer == me)?;
    let perk = Perk::ShroomTea;
    if member.perks.contains(&perk) || !shared::perks::in_range(perk, lobby.map, feet, 0.0) {
        return None;
    }
    Some((perk, member.score >= perk.cost()))
}

fn update_perk_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut prompt: Single<(&mut Text, &mut TextColor), With<PerkPrompt>>,
) {
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let here = (!menu.is_open() && active_killcam.0.is_none())
        .then(|| {
            let lobby = zombies_game(&local, &lobbies)?;
            perk_here(lobby, local.iter().next()?.0, feet)
        })
        .flatten();
    let (wanted, color) = match here {
        Some((perk, true)) => (
            format!(
                "Press {} to buy {} [Cost: {}]",
                binds.interact.label(),
                perk.label(),
                perk.cost()
            ),
            Color::WHITE,
        ),
        Some((perk, false)) => (
            format!("{} costs {} — not enough points", perk.label(), perk.cost()),
            Color::srgb(0.85, 0.35, 0.35),
        ),
        None => (String::new(), Color::WHITE),
    };
    let (text, text_color) = &mut *prompt;
    if text.0 != wanted {
        text.0 = wanted;
    }
    if text_color.0 != color {
        text_color.0 = color;
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
    let Some((perk, true)) = perk_here(lobby, me, feet) else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::BuyPerk { perk });
        // (The server logs "bought ..." when it goes through; the prompt
        // disappears once that's replicated back.)
        info!("asked the server to buy {}", perk.label());
    }
}

/// Shroom Tea's effect for now: the shroom screen effect, on for as long as we
/// own the perk in a running `Zombies` game — and off again the moment we
/// don't (game over, left, or not in a game at all). The moment it switches
/// on is our purchase going through, so that's when the buy sound and the
/// drinking arms play — for us only, since it keys off our own member's perks.
fn sync_shroom_perk(
    state: Res<State<AppState>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    mut perk: ResMut<ShroomPerk>,
    mut drink: ResMut<crate::PerkDrink>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    let owned = *state.get() == AppState::InGame
        && zombies_game(&local, &lobbies).is_some_and(|l| {
            l.members
                .iter()
                .any(|m| Some(m.peer) == me && m.perks.contains(&Perk::ShroomTea))
        });
    if perk.0 != owned {
        perk.0 = owned;
        if owned {
            // Stow the weapon and drink it (`weapons::drink_arms`).
            drink.requested = true;
        }
        if let (true, Some(sounds)) = (owned, sounds) {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.shroom_tea_buy.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}
