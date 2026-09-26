//! `Zombies`-only client pieces: the "enemies left" counter (left edge,
//! vertically centred), every member's points / health / name panel (bottom
//! left, ours at the bottom of the stack), the perk machines (solid, with a light) with
//! the card shown while standing at one, and switching on what an owned perk does
//! (Shroom Tea: the shroom screen effect; Nitro Brew: the [`NitroBrew`]
//! speed multipliers; Liquid Courage: the drunk screen effect — its damage
//! cut is server-side).
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
                    (play_perk_jingles, update_perk_jingles).chain(),
                    update_perk_card,
                    update_perk_icons,
                    update_party_panels,
                    buy_perk.run_if(menu::game_active.and(killcam::no_killcam)),
                )
                    .run_if(in_state(AppState::InGame)),
            )
            .init_resource::<NitroBrew>()
            .init_resource::<PerkMachineSettings>()
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
        Perk::LiquidCourage => Color::srgb_u8(0xc8, 0x14, 0x2d),
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
pub(crate) fn my_lobby<'a>(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &'a Query<&Lobby>,
) -> Option<&'a Lobby> {
    let me = local.iter().next()?.0;
    lobbies.iter().find(|l| l.has(me))
}

/// Our lobby while it's a running `Zombies` game.
pub(crate) fn zombies_game<'a>(
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

/// A perk machine's root: stands on its spot, turned to face its way
/// ([`PerkMachineSettings::placement`]). Its parts are children.
#[derive(Component)]
struct PerkMachine(Perk);

/// The pieces of a perk machine, placed relative to its root by
/// [`sync_perk_machines`].
#[derive(Component, Clone, Copy)]
enum MachinePart {
    /// The imported `.glb`.
    Model,
    /// Its solid box — players walk into it and shots / effects stop on it
    /// (the server has the same box, `server::collision::LobbyWorld`).
    Collider,
    /// A soft light in the perk's colour above it.
    Light,
}

/// The machine models' box in their own (Blender) units: half of each
/// `*_perk_machine.glb`'s width, height and depth. Every machine is this box,
/// just with its own material.
const MODEL_HALF_EXTENTS: Vec3 = Vec3::new(1.25, 2.0, 0.6);

/// A machine's nudge away from where `shared` puts it, from the debug panel.
#[derive(Clone, Copy, Default)]
pub(crate) struct MachineNudge {
    /// Metres from [`Perk::machine_pos`].
    pub(crate) offset: Vec3,
    /// Degrees on top of [`Perk::machine_yaw_deg`].
    pub(crate) yaw_deg: f32,
}

/// The light every perk machine has, in the perk's colour.
#[derive(Clone, Copy)]
pub(crate) struct MachineLight {
    /// Where it sits (m) from the ground under the machine's middle, in the
    /// machine's own frame (x across its width, z out through its depth), so
    /// it turns with the machine.
    pub(crate) offset: Vec3,
    /// Lumens.
    pub(crate) intensity: f32,
    /// How far (m) it reaches.
    pub(crate) range: f32,
    /// Size of the glowing source (m) — softens highlights.
    pub(crate) radius: f32,
    pub(crate) shadows: bool,
}

impl Default for MachineLight {
    fn default() -> Self {
        Self {
            offset: Vec3::new(0.0, 1.4, 0.0),
            intensity: 200_000.0,
            range: 27.0,
            radius: 0.0,
            shadows: false,
        }
    }
}

impl MachineLight {
    fn point_light(&self, color: Color) -> PointLight {
        PointLight {
            color,
            intensity: self.intensity,
            range: self.range,
            radius: self.radius,
            shadows_enabled: self.shadows,
            ..default()
        }
    }
}

/// Panel-tunable perk machines ("Zombies perks" → "Perk machines"). The
/// machines' real spots live in `shared::perks` so the server agrees on them
/// (buy range, collision for bots and shots); these nudges move a machine —
/// model, collider, light, jingle and the range for its card — on this
/// client only, for finding a spot to bake into `shared`.
#[derive(Resource, Clone)]
pub(crate) struct PerkMachineSettings {
    /// Model scale for every machine. At the default it matches the
    /// server's `shared::perks::MACHINE_HALF_EXTENTS`.
    pub(crate) scale: f32,
    /// Every machine's light (only its colour is the perk's own).
    pub(crate) light: MachineLight,
    pub(crate) shroom: MachineNudge,
    pub(crate) nitro: MachineNudge,
    pub(crate) courage: MachineNudge,
}

impl Default for PerkMachineSettings {
    fn default() -> Self {
        Self {
            scale: shared::perks::MACHINE_HALF_EXTENTS.y / MODEL_HALF_EXTENTS.y,
            light: default(),
            shroom: default(),
            nitro: default(),
            courage: default(),
        }
    }
}

impl PerkMachineSettings {
    pub(crate) fn nudge_mut(&mut self, perk: Perk) -> &mut MachineNudge {
        match perk {
            Perk::ShroomTea => &mut self.shroom,
            Perk::NitroBrew => &mut self.nitro,
            Perk::LiquidCourage => &mut self.courage,
        }
    }

    fn nudge(&self, perk: Perk) -> &MachineNudge {
        match perk {
            Perk::ShroomTea => &self.shroom,
            Perk::NitroBrew => &self.nitro,
            Perk::LiquidCourage => &self.courage,
        }
    }

    /// Where `perk`'s machine stands on `map` (the ground under its middle)
    /// and which way it faces (degrees), nudges included.
    pub(crate) fn placement(&self, perk: Perk, map: shared::MapId) -> (Vec3, f32) {
        let n = self.nudge(perk);
        (perk.machine_pos(map) + n.offset, perk.machine_yaw_deg(map) + n.yaw_deg)
    }

    /// Half the machine's width, height and depth (m).
    fn half_extents(&self) -> Vec3 {
        MODEL_HALF_EXTENTS * self.scale.max(0.001)
    }

    fn part_transform(&self, part: MachinePart) -> Transform {
        let half = self.half_extents();
        match part {
            MachinePart::Model => {
                Transform::from_xyz(0.0, half.y, 0.0).with_scale(Vec3::splat(self.scale.max(0.001)))
            }
            MachinePart::Collider => Transform::from_xyz(0.0, half.y, 0.0),
            MachinePart::Light => Transform::from_translation(self.light.offset),
        }
    }
}

/// Each perk's machine model.
fn machine_model_path(perk: Perk) -> &'static str {
    match perk {
        Perk::ShroomTea => "models/shroom_tea_perk_machine.glb",
        Perk::NitroBrew => "models/nitro_brew_perk_machine.glb",
        Perk::LiquidCourage => "models/liquid_courage_perk_machine.glb",
    }
}

/// A perk machine's jingle, playing from the machine (see
/// [`play_perk_jingles`]). Its loudness follows the listener's distance every
/// frame ([`update_perk_jingles`]); it despawns when the clip ends, and it's
/// `StateScoped(InGame)` so leaving cuts it off.
#[derive(Component)]
struct PerkJingle;

/// Anyone in our `Zombies` game just bought a perk — a new entry in their
/// replicated `LobbyMember::perks`, which every client sees — so play that
/// perk's jingle from its machine, for the whole lobby at once. The lists are
/// compared frame to frame; outside a `Zombies` game there's nothing to
/// compare against, so a new game starts fresh.
fn play_perk_jingles(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    machines: Res<PerkMachineSettings>,
    mut seen: Local<Option<std::collections::HashMap<lightyear::prelude::PeerId, Vec<Perk>>>>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        *seen = None;
        return;
    };
    let now: std::collections::HashMap<_, _> =
        lobby.members.iter().map(|m| (m.peer, m.perks.clone())).collect();
    if let (Some(prev), Some(sounds)) = (seen.as_ref(), sounds.as_ref()) {
        for (peer, perks) in &now {
            let before = prev.get(peer);
            for &perk in perks.iter().filter(|p| before.is_none_or(|b| !b.contains(p))) {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    PerkJingle,
                    // Volume is ours to set (`update_perk_jingles`), not the
                    // one-shot volume pass's.
                    crate::RemoteSoundEmitter,
                    AudioPlayer::new(sounds.jingle(perk)),
                    // From about the machine's sign.
                    Transform::from_translation(
                        machines.placement(perk, lobby.map).0
                            + Vec3::Y * machines.half_extents().y * 1.6,
                    ),
                    // Starts silent; the real volume is set from the next frame.
                    crate::positional_playback(bevy::audio::Volume::Linear(0.0)),
                ));
            }
        }
    }
    *seen = Some(now);
}

/// Keep each playing jingle's loudness matched to how far the listener is
/// from its machine — the same fade as other players' sounds ("Remote
/// sounds" range) — times the "perk jingle" volume.
fn update_perk_jingles(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    sound_vol: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut jingles: Query<(&GlobalTransform, &mut SpatialAudioSink), With<PerkJingle>>,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();
    for (gt, mut sink) in &mut jingles {
        let loudness =
            sound_vol.perk_jingle * crate::distance_falloff(ear.distance(gt.translation()), &remote);
        sink.set_volume(bevy::audio::Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

/// Put each perk's machine on the map while we're in a `Zombies` game (and
/// take it away otherwise), and keep it where [`PerkMachineSettings`] says.
#[allow(clippy::type_complexity)]
fn sync_perk_machines(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    settings: Res<PerkMachineSettings>,
    asset_server: Res<AssetServer>,
    mut machines: Query<(Entity, &PerkMachine, &mut Transform)>,
    mut parts: Query<(&MachinePart, &mut Transform), Without<PerkMachine>>,
    mut colliders: Query<&mut bevy_rapier3d::prelude::Collider, With<MachinePart>>,
    mut lights: Query<&mut PointLight, With<MachinePart>>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        for (e, ..) in &machines {
            commands.entity(e).despawn();
        }
        return;
    };
    for perk in Perk::ALL {
        if !machines.iter().any(|(_, m, _)| m.0 == perk) {
            spawn_perk_machine(perk, lobby.map, &settings, &asset_server, &mut commands);
        }
    }
    for (_, m, mut t) in &mut machines {
        t.set_if_neq(machine_transform(&settings, m.0, lobby.map));
    }
    if settings.is_changed() {
        for (part, mut t) in &mut parts {
            t.set_if_neq(settings.part_transform(*part));
        }
        let half = settings.half_extents();
        for mut c in &mut colliders {
            *c = bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z);
        }
        for mut l in &mut lights {
            *l = settings.light.point_light(l.color);
        }
    }
}

fn machine_transform(settings: &PerkMachineSettings, perk: Perk, map: shared::MapId) -> Transform {
    let (pos, yaw_deg) = settings.placement(perk, map);
    Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw_deg.to_radians()))
}

/// One perk's machine: its model, solid box and light.
fn spawn_perk_machine(
    perk: Perk,
    map: shared::MapId,
    settings: &PerkMachineSettings,
    asset_server: &AssetServer,
    commands: &mut Commands,
) {
    let half = settings.half_extents();
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PerkMachine(perk),
            machine_transform(settings, perk, map),
            Visibility::default(),
        ))
        .with_children(|m| {
            m.spawn((
                MachinePart::Model,
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(machine_model_path(perk)))),
                settings.part_transform(MachinePart::Model),
            ));
            m.spawn((
                MachinePart::Collider,
                bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                settings.part_transform(MachinePart::Collider),
            ));
            m.spawn((
                MachinePart::Light,
                settings.light.point_light(perk_color(perk)),
                settings.part_transform(MachinePart::Light),
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

pub(crate) const PERK_CARD_WIDTH: f32 = 440.0;
const PERK_CARD_ICON: f32 = 64.0;
pub(crate) const CARD_RED: Color = Color::srgb(0.9, 0.3, 0.3);
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
                            col.spawn((
                                PerkCardText::Name,
                                Text::new(""),
                                heading(38.0),
                                TextColor::WHITE,
                            ));
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
                            col.spawn((
                                PerkCardText::Cost,
                                Text::new(""),
                                heading(30.0),
                                TextColor(MONEY_YELLOW),
                            ));
                        });
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::End,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("YOUR POINTS"), heading(16.0), faint));
                            col.spawn((
                                PerkCardText::Points,
                                Text::new(""),
                                heading(30.0),
                                TextColor::WHITE,
                            ));
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
                .with_child((
                    PerkCardText::Action,
                    Text::new(""),
                    heading(24.0),
                    TextColor::WHITE,
                ));
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
pub(crate) const MONEY_YELLOW: Color = Color::srgb(1.0, 0.82, 0.1);

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
                row.spawn((
                    PartyMoney(peer),
                    Text::new("0"),
                    text(26.0),
                    TextColor(Color::WHITE),
                    shadow,
                ));
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
                row.spawn((
                    PartyHealthText(peer),
                    Text::new("100"),
                    text(28.0),
                    TextColor(Color::WHITE),
                    shadow,
                ));
            });
        panel.spawn((
            Text::new(name.to_string()),
            text(20.0),
            TextColor(Color::WHITE),
            shadow,
        ));
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
        let points = lobby
            .members
            .iter()
            .find(|x| x.peer == m.0)
            .map_or(0, |x| x.score);
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
const PERK_ICON_SIZE: f32 = 100.0;
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

/// A perk's icon — on the machine's card (`update_perk_card`) and in the
/// owned-perk row along the bottom (`update_perk_icons`).
fn perk_icon_path(perk: Perk) -> &'static str {
    match perk {
        Perk::ShroomTea => "textures/icons/perks/shroom_tea.png",
        Perk::NitroBrew => "textures/icons/perks/nitro_brew.png",
        Perk::LiquidCourage => "textures/icons/perks/liquid_courage.png",
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
    mut slots: Query<(
        &mut PerkIconSlot,
        &mut ImageNode,
        &mut Visibility,
        &mut Node,
    )>,
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
        let display = if perk.is_some() {
            Display::Flex
        } else {
            Display::None
        };
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
fn perk_here(
    lobby: &Lobby,
    me: lightyear::prelude::PeerId,
    feet: Vec3,
    machines: &PerkMachineSettings,
) -> Option<(Perk, PerkStatus, u32)> {
    let member = lobby.members.iter().find(|m| m.peer == me)?;
    let perk = Perk::ALL
        .into_iter()
        .find(|&p| shared::perks::in_range_of(machines.placement(p, lobby.map).0, feet, 0.0))?;
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
    machines: Res<PerkMachineSettings>,
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
            perk_here(lobby, local.iter().next()?.0, feet, &machines)
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
    machines: Res<PerkMachineSettings>,
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
    let Some((perk, PerkStatus::Buyable, _)) = perk_here(lobby, me, feet, &machines) else {
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
/// `Zombies` game — Shroom Tea's shroom effect, Nitro Brew's multipliers,
/// Liquid Courage's drunk effect —
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
    mut courage: ResMut<crate::LiquidCouragePerk>,
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
    let has_courage = owned.contains(&Perk::LiquidCourage);
    if courage.0 != has_courage {
        courage.0 = has_courage;
    }
    *prev = owned;
}
