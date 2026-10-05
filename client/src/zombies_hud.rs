//! `Zombies`-only client pieces: the "enemies left" counter with the
//! "active enemies" one right under it (left edge, vertically centred), every member's points / health / name panel (bottom
//! left, ours at the bottom of the stack), the perk machines (solid, with a light) with
//! the card shown while standing at one, and switching on what an owned perk does
//! (Shroom Tea: the shroom screen effect; Nitro Brew: the [`NitroBrew`]
//! speed multipliers; Liquid Courage: the drunk screen effect — its damage
//! cut is server-side; the classic Call of Duty perks: [`ClassicPerks`]).
//! The machines are the lobby's perk set's (`Lobby::perk_set`). Going prone at a machine claims its one-off
//! [`shared::perks::PRONE_BONUS_POINTS`] ([`prone_at_perk`]).
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
                    update_active_enemies,
                    update_pregame_countdown,
                    sync_perk_machines,
                    (play_perk_jingles, play_perk_quotes, update_perk_jingles).chain(),
                    update_perk_card,
                    update_no_power_prompt,
                    update_perk_icons,
                    update_party_panels,
                    buy_perk.run_if(menu::game_active.and(killcam::no_killcam)),
                    phd_slam.run_if(menu::game_active.and(killcam::no_killcam)),
                    prone_at_perk,
                    receive_prone_bonus,
                )
                    .run_if(in_state(AppState::InGame)),
            )
            .init_resource::<NitroBrew>()
            .init_resource::<ClassicPerks>()
            .init_resource::<Kangabrew>()
            .init_resource::<PerkMachineSettings>()
            // Not gated on `InGame`: it's what switches the effects back *off*
            // once the game is left.
            .add_systems(Update, sync_owned_perks);
    }
}

/// Each perk's colour — its machine's glow, its name on the card and its
/// bottle in the drinking arms. The classic ones are Call of Duty's own.
pub(crate) fn perk_color(perk: Perk) -> Color {
    match perk {
        Perk::ShroomTea => Color::srgb_u8(0x6a, 0x1f, 0xbf),
        Perk::NitroBrew => Color::srgb_u8(0xff, 0xd4, 0x00),
        Perk::LiquidCourage => Color::srgb_u8(0xc8, 0x14, 0x2d),
        Perk::BombShot => Color::srgb_u8(0xff, 0x6a, 0x00),
        Perk::Kangabrew => Color::srgb_u8(0x2e, 0xd1, 0x4a),
        Perk::Juggernog => Color::srgb_u8(0xe0, 0x20, 0x2a),
        Perk::QuickRevive => Color::srgb_u8(0x5f, 0xd6, 0xff),
        Perk::SpeedCola => Color::srgb_u8(0x3c, 0xff, 0x3c),
        Perk::StaminUp => Color::srgb_u8(0xff, 0xb2, 0x3a),
        Perk::DoubleTap => Color::srgb_u8(0xff, 0x6a, 0x1e),
        Perk::DeadshotDaiquiri => Color::srgb_u8(0x8d, 0xb0, 0x6a),
        Perk::PhdFlopper => Color::srgb_u8(0xa8, 0x3c, 0xff),
        Perk::DeathPerception => Color::srgb_u8(0xff, 0x5a, 0x14),
    }
}

/// The classic (Call of Duty) perks we own right now, and what the
/// client-side ones do (the debug panel's "Classic perks" section). Kept in
/// step with our perks by `sync_owned_perks`, so it's all off again the
/// moment a game ends or is left. Juggernog and Quick Revive are the
/// server's (health), as is PhD Flopper's explosion damage.
#[derive(Resource)]
pub(crate) struct ClassicPerks {
    pub(crate) owned: Vec<Perk>,
    /// Debug: act as if every classic perk is owned. Never saved.
    pub(crate) debug_force: bool,
    /// Speed Cola: reload speed (×).
    pub(crate) speed_cola_reload: f32,
    /// Stamin-Up: movement speed (×).
    pub(crate) stamin_up_move: f32,
    /// Double Tap: rate of fire (×) — the AK's rounds, the sniper's bolt.
    pub(crate) double_tap_fire_rate: f32,
    /// PhD Flopper: how much faster a slide launches (×), and how much
    /// longer it carries on for (× its length, on top of the speed).
    pub(crate) phd_slide_speed: f32,
    pub(crate) phd_slide: f32,
    /// Death Perception's outline (`vfx::shroom_xray`): sRGB colour, glow,
    /// how tight to the silhouette it hugs (higher = thinner), and how far
    /// (m) it's puffed out past the body.
    pub(crate) death_perception_color: [f32; 3],
    pub(crate) death_perception_brightness: f32,
    pub(crate) death_perception_sharpness: f32,
    pub(crate) death_perception_inflate: f32,
}

impl Default for ClassicPerks {
    fn default() -> Self {
        Self {
            owned: Vec::new(),
            debug_force: false,
            speed_cola_reload: 1.5,
            stamin_up_move: 1.1,
            double_tap_fire_rate: 1.25,
            phd_slide_speed: 1.3,
            phd_slide: 1.2,
            death_perception_color: [1.0, 0.45, 0.05],
            death_perception_brightness: 4.0,
            death_perception_sharpness: 2.5,
            death_perception_inflate: 0.02,
        }
    }
}

impl ClassicPerks {
    pub(crate) fn has(&self, perk: Perk) -> bool {
        self.debug_force && perk.set() == shared::perks::PerkSet::Classic || self.owned.contains(&perk)
    }

    fn pick(&self, perk: Perk, mult: f32) -> f32 {
        if self.has(perk) {
            mult.max(0.01)
        } else {
            1.0
        }
    }

    /// Movement speed multiplier (Stamin-Up).
    pub(crate) fn movement(&self) -> f32 {
        self.pick(Perk::StaminUp, self.stamin_up_move)
    }

    /// Reload speed multiplier (Speed Cola).
    pub(crate) fn reload(&self) -> f32 {
        self.pick(Perk::SpeedCola, self.speed_cola_reload)
    }

    /// Rate of fire multiplier (Double Tap).
    pub(crate) fn fire_rate(&self) -> f32 {
        self.pick(Perk::DoubleTap, self.double_tap_fire_rate)
    }

    /// Slide length multiplier (PhD Flopper).
    pub(crate) fn slide(&self) -> f32 {
        self.pick(Perk::PhdFlopper, self.phd_slide)
    }

    /// Slide launch speed multiplier (PhD Flopper).
    pub(crate) fn slide_speed(&self) -> f32 {
        self.pick(Perk::PhdFlopper, self.phd_slide_speed)
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

/// Kangabrew (the debug panel's "Kangabrew" section): higher jumps and
/// Black Ops 7 style wall jumps — see `player::movement::jump`. `owned` is
/// kept in step with our perks by `sync_owned_perks`, so it's off again the
/// moment a game ends or is left.
#[derive(Resource)]
pub(crate) struct Kangabrew {
    pub(crate) owned: bool,
    /// Debug: act as if owned, to tune without buying it. Never saved.
    pub(crate) debug_force: bool,
    /// How high a jump goes, as a multiple of a normal one (2 = twice as
    /// high).
    pub(crate) jump_height_mult: f32,
    pub(crate) wall_jumps: bool,
    /// How high a wall jump goes from where it's made, as a multiple of a
    /// normal (perk-less) jump's height.
    pub(crate) wall_jump_height_mult: f32,
    /// Speed (m/s) a wall jump pushes you straight out from the wall.
    pub(crate) wall_push: f32,
    /// Extra speed (m/s) toward where you're looking (never back into the
    /// wall), so you can steer off it.
    pub(crate) wall_steer: f32,
    /// How close (m) the edge of your body must be to a wall.
    pub(crate) wall_reach: f32,
    /// Wall jumps allowed before touching the ground again.
    pub(crate) max_wall_jumps: u32,
    /// Seconds between wall jumps.
    pub(crate) wall_cooldown: f32,
}

impl Default for Kangabrew {
    fn default() -> Self {
        Self {
            owned: false,
            debug_force: false,
            jump_height_mult: 3.0,
            wall_jumps: true,
            wall_jump_height_mult: 2.0,
            wall_push: 5.0,
            wall_steer: 3.0,
            wall_reach: 0.5,
            max_wall_jumps: 3,
            wall_cooldown: 0.25,
        }
    }
}

impl Kangabrew {
    pub(crate) fn active(&self) -> bool {
        self.owned || self.debug_force
    }

    /// Launch speed for a ground jump: `base` scaled so the jump reaches
    /// `jump_height_mult` × the height (height goes with speed squared).
    pub(crate) fn jump_speed(&self, base: f32) -> f32 {
        if self.active() {
            base * self.jump_height_mult.max(0.01).sqrt()
        } else {
            base
        }
    }

    /// Launch speed for a wall jump (see [`Self::wall_jump_height_mult`]).
    pub(crate) fn wall_jump_speed(&self, base: f32) -> f32 {
        base * self.wall_jump_height_mult.max(0.01).sqrt()
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

/// The "active enemies" panel under it: how many are spawned in right now.
#[derive(Component)]
struct ActiveEnemiesRoot;

#[derive(Component)]
struct ActiveEnemiesCount;

/// One left-edge counter panel: a small `label` over a big number (filled in
/// by the `count` entity's `Text`), hidden until a round is on.
fn spawn_counter_panel(
    col: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    label: &str,
    root: impl Component,
    count: impl Component,
) {
    col.spawn((
        root,
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
            Text::new(label),
            TextFont {
                font: font.clone(),
                font_size: 15.0,
                ..default()
            },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
        ));
        panel.spawn((
            count,
            Text::new(""),
            TextFont {
                font: font.clone(),
                font_size: 40.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    });
}

/// The "enemies left" counter, party panels, owned-perk icons and the
/// (hidden until needed) perk card.
fn spawn_zombies_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    // Left edge, centred vertically: a full-height column that centres the
    // stacked panels in it, "enemies left" over "active enemies" (stretched
    // to one width; each panel's background only wraps the pair).
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|col| {
            spawn_counter_panel(col, &font, "ENEMIES LEFT", EnemiesLeftRoot, EnemiesLeftCount);
            spawn_counter_panel(col, &font, "ACTIVE ENEMIES", ActiveEnemiesRoot, ActiveEnemiesCount);
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
            for index in 0..Perk::MAX_PER_SET {
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

    // The pre-game countdown: top centre, while it runs.
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PregameCountdown,
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                // (Under the compass.)
                top: Val::Px(crate::hud::COMPASS_HEIGHT + 8.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            let shadow = TextShadow {
                offset: Vec2::splat(2.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.8),
            };
            col.spawn((
                Text::new("GAME STARTS IN"),
                TextFont {
                    font: font.clone(),
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                shadow,
            ));
            col.spawn((
                PregameCountdownText,
                Text::new(""),
                TextFont {
                    font: font.clone(),
                    font_size: 64.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                shadow,
            ));
        });

    spawn_perk_card(&mut commands, &asset_server, font);
}

/// The pre-game countdown's panel (top centre) and its m:ss.
#[derive(Component)]
struct PregameCountdown;

#[derive(Component)]
struct PregameCountdownText;

/// Show the pre-game countdown while it runs (`Lobby::countdown_left`) —
/// hidden behind menus and the kill cam like the rest of the HUD. The last
/// ten seconds go red.
fn update_pregame_countdown(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut root: Single<&mut Visibility, With<PregameCountdown>>,
    text: Single<(&mut Text, &mut TextColor), With<PregameCountdownText>>,
) {
    let left = zombies_game(&local, &lobbies)
        .map(|l| l.countdown_left)
        .filter(|&s| s > 0 && !menu.is_open() && active_killcam.0.is_none());
    root.set_if_neq(if left.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    let Some(secs) = left else { return };
    let (mut text, mut color) = text.into_inner();
    let wanted = format!("{}:{:02}", secs / 60, secs % 60);
    if text.0 != wanted {
        text.0 = wanted;
    }
    let c = if secs <= 10 { CARD_RED } else { Color::WHITE };
    color.set_if_neq(TextColor(c));
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

fn update_active_enemies(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut root: Single<&mut Visibility, With<ActiveEnemiesRoot>>,
    mut count: Single<&mut Text, With<ActiveEnemiesCount>>,
) {
    let active = zombies_game(&local, &lobbies)
        .filter(|l| l.round > 0)
        .map(|l| l.enemies_active);
    root.set_if_neq(if active.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    let wanted = active.map(|n| n.to_string()).unwrap_or_default();
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
    /// The imported `.glb` of this perk's machine.
    Model(Perk),
    /// Its solid box — players walk into it and shots / effects stop on it
    /// (the server has the same box, `server::collision::LobbyWorld`).
    Collider,
    /// A soft light in the perk's colour above it.
    Light,
}

/// The custom machine models' box in their own (Blender) units: half of each
/// `custom/*_perk_machine.glb`'s width, height and depth. Every custom
/// machine is this box, centred on its origin, just with its own material.
const MODEL_HALF_EXTENTS: Vec3 = Vec3::new(1.25, 2.0, 0.6);

/// A classic machine model's size and placement in its own units — they're
/// from all over, each at its own scale and some with their origin at the
/// base, some in the middle — so each is fitted to the machine's box:
/// scaled to its height, stood on the ground and centred.
pub(crate) struct ModelFit {
    /// Its height, its lowest point, and the middle of its footprint
    /// (x, z).
    height: f32,
    bottom: f32,
    center: Vec2,
}

impl ModelFit {
    const fn new(height: f32, bottom: f32, x: f32, z: f32) -> Self {
        Self {
            height,
            bottom,
            center: Vec2::new(x, z),
        }
    }
}

/// Der Wunderfizz's model (`crate::wunderfizz`) and its fit.
pub(crate) const WUNDERFIZZ_MODEL: &str = "models/props/perk_machines/classic/der_wunderfizz.glb";
const WUNDERFIZZ_FIT: ModelFit = ModelFit::new(1.779, -0.002, 0.078, -0.114);

/// A model fitted by `fit` to a box `height` tall: base on the ground,
/// footprint centred under the box, turned `yaw_deg` about its own middle.
fn fit_transform(fit: &ModelFit, height: f32, yaw_deg: f32) -> Transform {
    let s = height / fit.height.max(1e-6);
    let turn = Quat::from_rotation_y(yaw_deg.to_radians());
    Transform {
        translation: turn * Vec3::new(-fit.center.x * s, -fit.bottom * s, -fit.center.y * s),
        rotation: turn,
        scale: Vec3::splat(s),
    }
}

/// Each perk's machine model, and for a classic one how to fit it (measured
/// from the `.glb`s' bounds). Death Perception and PhD Flopper have none
/// (only Der Wunderfizz sells them); they'd get its model.
pub(crate) fn machine_model(perk: Perk) -> (&'static str, Option<ModelFit>) {
    const WUNDERFIZZ: (&str, Option<ModelFit>) = (WUNDERFIZZ_MODEL, Some(WUNDERFIZZ_FIT));
    match perk {
        Perk::ShroomTea => ("models/props/perk_machines/custom/shroom_tea_perk_machine.glb", None),
        Perk::NitroBrew => ("models/props/perk_machines/custom/nitro_brew_perk_machine.glb", None),
        Perk::LiquidCourage => ("models/props/perk_machines/custom/liquid_courage_perk_machine.glb", None),
        Perk::BombShot => ("models/props/perk_machines/custom/bomb_shot_perk_machine.glb", None),
        Perk::Kangabrew => ("models/props/perk_machines/custom/kangabrew_perk_machine.glb", None),
        Perk::Juggernog => (
            "models/props/perk_machines/classic/juggernog_perk_machine.glb",
            Some(ModelFit::new(0.089, 0.002, -0.0005, -0.013)),
        ),
        Perk::QuickRevive => (
            "models/props/perk_machines/classic/quick_revive_perk_machine.glb",
            Some(ModelFit::new(75.5, -0.016, 0.0, 1.4)),
        ),
        Perk::SpeedCola => (
            "models/props/perk_machines/classic/speed_cola_perk_machine.glb",
            Some(ModelFit::new(95.834, 0.0, -3.51, 1.0)),
        ),
        Perk::StaminUp => (
            "models/props/perk_machines/classic/stamin_up_perk_machine.glb",
            Some(ModelFit::new(42.0, 0.0, -0.5, 0.0)),
        ),
        Perk::DoubleTap => (
            "models/props/perk_machines/classic/double_tap_perk_machine.glb",
            Some(ModelFit::new(1.968, 0.007, -0.002, 0.036)),
        ),
        Perk::DeadshotDaiquiri => (
            "models/props/perk_machines/classic/deadshot_daiquiri_perk_machine.glb",
            Some(ModelFit::new(2.0, -1.0, 0.0, 0.0)),
        ),
        Perk::PhdFlopper | Perk::DeathPerception => WUNDERFIZZ,
    }
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
    pub(crate) fn point_light(&self, color: Color) -> PointLight {
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

/// Panel-tunable perk machines ("Zombies perks" → "Perk machines"): their
/// look. Where they stand is the map's layout's (`shared::level`, placed in
/// the level editor), so the server agrees on it (buy range, collision for
/// bots and shots).
#[derive(Resource, Clone)]
pub(crate) struct PerkMachineSettings {
    /// Model scale for every machine. At the default it matches the
    /// server's `shared::perks::MACHINE_HALF_EXTENTS`.
    pub(crate) scale: f32,
    /// Every machine's light (only its colour is the perk's own).
    pub(crate) light: MachineLight,
    /// The fog every machine vents once the power's on
    /// (`vfx::machine_fog`).
    pub(crate) fog: crate::vfx::MachineFog,
    /// Each classic machine model's turn (degrees) inside its box — the
    /// imported models don't all face the same way. The machine's own turn
    /// (its layout's) is what the fog follows, so this lines
    /// the model's front up with that and nothing else.
    pub(crate) model_yaw_deg: std::collections::HashMap<Perk, f32>,
    /// Der Wunderfizz's model's turn inside its box (`crate::wunderfizz`).
    pub(crate) wunderfizz_model_yaw_deg: f32,
}

impl Default for PerkMachineSettings {
    fn default() -> Self {
        Self {
            scale: shared::perks::MACHINE_HALF_EXTENTS.y / MODEL_HALF_EXTENTS.y,
            light: default(),
            fog: default(),
            // Turned so each model's front — its dispenser slot — faces the
            // machine's front, the way its fog vents (`vfx::machine_fog`);
            // the rest already do. Degrees counter-clockwise seen from above.
            model_yaw_deg: [(Perk::StaminUp, 90.0), (Perk::QuickRevive, -90.0), (Perk::Juggernog, 180.0)]
                .into_iter()
                .collect(),
            wunderfizz_model_yaw_deg: 0.0,
        }
    }
}

impl PerkMachineSettings {
    /// Where `perk`'s machine stands on `map` (the ground under its middle)
    /// and which way it faces (degrees).
    pub(crate) fn placement(&self, perk: Perk, map: shared::MapId) -> (Vec3, f32) {
        (perk.machine_pos(map), perk.machine_yaw_deg(map))
    }

    /// Where Der Wunderfizz stands on `map` and which way it faces
    /// (degrees).
    pub(crate) fn wunderfizz_placement(&self, map: shared::MapId) -> (Vec3, f32) {
        (shared::wunderfizz::machine_pos(map), shared::wunderfizz::machine_yaw_deg(map))
    }

    /// `perk`'s machine model, from the machine's root.
    pub(crate) fn model_transform(&self, perk: Perk) -> Transform {
        self.part_transform(MachinePart::Model(perk))
    }

    /// Der Wunderfizz's model, fitted to the machine box like the classic
    /// perk machines' ([`ModelFit`]).
    pub(crate) fn wunderfizz_model_transform(&self) -> Transform {
        fit_transform(&WUNDERFIZZ_FIT, self.half_extents().y * 2.0, self.wunderfizz_model_yaw_deg)
    }

    /// Half the machine's width, height and depth (m).
    pub(crate) fn half_extents(&self) -> Vec3 {
        MODEL_HALF_EXTENTS * self.scale.max(0.001)
    }

    fn part_transform(&self, part: MachinePart) -> Transform {
        let half = self.half_extents();
        match part {
            MachinePart::Model(perk) => match machine_model(perk).1 {
                None => Transform::from_xyz(0.0, half.y, 0.0).with_scale(Vec3::splat(self.scale.max(0.001))),
                Some(fit) => fit_transform(
                    &fit,
                    half.y * 2.0,
                    self.model_yaw_deg.get(&perk).copied().unwrap_or(0.0),
                ),
            },
            MachinePart::Collider => Transform::from_xyz(0.0, half.y, 0.0),
            MachinePart::Light => Transform::from_translation(self.light.offset),
        }
    }
}

/// A perk machine's jingle, playing from the machine (see
/// [`play_perk_jingles`]). Its loudness follows the listener's distance every
/// frame ([`update_perk_jingles`]). Played with `PlaybackMode::Remove`, so
/// once the clip ends the entity stays (minus its `AudioPlayer`) for
/// [`play_perk_quotes`] to notice, play `quote` and despawn it. It's
/// `StateScoped(InGame)` so leaving cuts it (and the quote to come) off.
#[derive(Component)]
struct PerkJingle {
    /// The buyer's operator's line, to play once the jingle ends.
    quote: Option<PendingQuote>,
}

struct PendingQuote {
    clip: Handle<AudioSource>,
    /// The clip's own "Sound volumes" multiplier.
    volume: f32,
    /// We're the buyer — our own voice, so it plays in our head rather than
    /// from the machine.
    own: bool,
}

/// Another player's perk quote, playing from the machine they bought at —
/// faded by distance like the jingle ([`update_perk_jingles`]). Holds the
/// clip's own volume multiplier.
#[derive(Component)]
struct PerkQuote(f32);

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
    let me = local.iter().next().map(|l| l.0);
    let now: std::collections::HashMap<_, _> =
        lobby.members.iter().map(|m| (m.peer, m.perks.clone())).collect();
    if let (Some(prev), Some(sounds)) = (seen.as_ref(), sounds.as_ref()) {
        for (peer, perks) in &now {
            let before = prev.get(peer);
            for &perk in perks.iter().filter(|p| before.is_none_or(|b| !b.contains(p))) {
                let Some(jingle) = sounds.jingle(perk) else {
                    continue;
                };
                // Picked from what every client sees, so the whole lobby
                // hears the same line.
                let seed = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    (peer, perk, lobby.round, perks.len()).hash(&mut h);
                    h.finish()
                };
                let operator = lobby.members.iter().find(|m| m.peer == *peer).map(|m| m.operator);
                let quote = operator
                    .and_then(|op| sounds.perk_quote(op, perk, seed))
                    .map(|clip| PendingQuote {
                        clip: clip.handle.clone(),
                        volume: clip.volume,
                        own: Some(*peer) == me,
                    });
                commands.spawn((
                    StateScoped(AppState::InGame),
                    PerkJingle { quote },
                    // Volume is ours to set (`update_perk_jingles`), not the
                    // one-shot volume pass's.
                    crate::RemoteSoundEmitter,
                    AudioPlayer::new(jingle),
                    // From about the machine's sign.
                    Transform::from_translation(
                        machines.placement(perk, lobby.map).0
                            + Vec3::Y * machines.half_extents().y * 1.6,
                    ),
                    // Starts silent; the real volume is set from the next
                    // frame. Kept once it ends, for `play_perk_quotes`.
                    PlaybackSettings {
                        mode: bevy::audio::PlaybackMode::Remove,
                        ..crate::positional_playback(bevy::audio::Volume::Linear(0.0))
                    },
                ));
            }
        }
    }
    *seen = Some(now);
}

/// A perk jingle just ended (its `AudioPlayer` is gone): the buyer's
/// operator says their line — in our head if it was us, otherwise from the
/// machine — and the jingle's entity goes.
fn play_perk_quotes(
    finished: Query<(Entity, &PerkJingle, &Transform), Without<AudioPlayer>>,
    sound_vol: Res<crate::SoundVolumes>,
    mut commands: Commands,
) {
    for (entity, jingle, transform) in &finished {
        commands.entity(entity).despawn();
        let Some(quote) = &jingle.quote else {
            continue;
        };
        if quote.own {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(quote.clip.clone()),
                // (`GlobalVolume` is multiplied in at spawn.)
                PlaybackSettings::DESPAWN
                    .with_volume(bevy::audio::Volume::Linear(sound_vol.perk_quote * quote.volume)),
            ));
        } else {
            commands.spawn((
                StateScoped(AppState::InGame),
                PerkQuote(quote.volume),
                crate::RemoteSoundEmitter,
                AudioPlayer::new(quote.clip.clone()),
                *transform,
                crate::positional_playback(bevy::audio::Volume::Linear(0.0)),
            ));
        }
    }
}

/// Keep each playing jingle's (and other players' perk quotes') loudness
/// matched to how far the listener is from its machine — the same fade as
/// other players' sounds ("Remote sounds" range) — times the "perk jingle"
/// (or "perk quote") volume.
#[allow(clippy::type_complexity)]
fn update_perk_jingles(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    sound_vol: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut jingles: Query<
        (&GlobalTransform, &mut SpatialAudioSink, Option<&PerkQuote>),
        Or<(With<PerkJingle>, With<PerkQuote>)>,
    >,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();
    for (gt, mut sink, quote) in &mut jingles {
        let volume = quote.map_or(sound_vol.perk_jingle, |q| sound_vol.perk_quote * q.0);
        let loudness = volume * crate::distance_falloff(ear.distance(gt.translation()), &remote);
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
    time: Res<Time>,
    map_lights: Res<crate::power::MapLightSettings>,
    // How far the machines' lights are faded in (0..=1) — with the power, at
    // the map lights' pace. Back to 0 whenever there's no game.
    mut power: Local<f32>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        for (e, ..) in &machines {
            commands.entity(e).despawn();
        }
        *power = 0.0;
        return;
    };
    let before = *power;
    let target = if shared::power::has_power(lobby.map, lobby.power_on) || map_lights.force_on {
        1.0
    } else {
        0.0
    };
    *power = if shared::power::switch_pos(lobby.map).is_none() || map_lights.fade_secs <= 0.0 {
        // (No switch on this map: just on.)
        target
    } else {
        let step = time.delta_secs() / map_lights.fade_secs;
        *power + (target - *power).clamp(-step, step)
    };
    // The lobby's set's machines — and not the other set's, nor the perks
    // only Der Wunderfizz sells (`crate::wunderfizz`).
    let set: Vec<Perk> = lobby.perk_set.machine_perks().collect();
    for &perk in &set {
        if !machines.iter().any(|(_, m, _)| m.0 == perk) {
            spawn_perk_machine(perk, lobby.map, &settings, &asset_server, &mut commands);
        }
    }
    for (e, m, mut t) in &mut machines {
        if set.contains(&m.0) {
            t.set_if_neq(machine_transform(&settings, m.0, lobby.map));
        } else {
            commands.entity(e).despawn();
        }
    }
    if settings.is_changed() {
        for (part, mut t) in &mut parts {
            t.set_if_neq(settings.part_transform(*part));
        }
        let half = settings.half_extents();
        for mut c in &mut colliders {
            *c = bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z);
        }
    }
    // (Every light when the look or the fade moves; otherwise just any that
    // were only now spawned, dark.)
    let all = settings.is_changed() || *power != before;
    let fade = *power * *power * (3.0 - 2.0 * *power);
    for mut l in &mut lights {
        if all || l.is_added() {
            let mut lit = settings.light.point_light(l.color);
            lit.intensity *= fade;
            lit.shadows_enabled &= fade > 0.0;
            *l = lit;
        }
    }
}

fn machine_transform(_settings: &PerkMachineSettings, perk: Perk, map: shared::MapId) -> Transform {
    crate::util::placed(shared::level::layout(map).perk(perk))
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
            // (From about its middle — the root's on the ground.)
            crate::power::PoweredHum(Vec3::Y * 1.2),
            crate::vfx::MachineFogEmitter::default(),
            machine_transform(settings, perk, map),
            Visibility::default(),
        ))
        .with_children(|m| {
            m.spawn((
                MachinePart::Model(perk),
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(machine_model(perk).0))),
                settings.part_transform(MachinePart::Model(perk)),
            ));
            m.spawn((
                MachinePart::Collider,
                bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                settings.part_transform(MachinePart::Collider),
            ));
            m.spawn((
                MachinePart::Light,
                // (Dark until `sync_perk_machines` fades it in with the power.)
                PointLight {
                    intensity: 0.0,
                    shadows_enabled: false,
                    ..settings.light.point_light(perk_color(perk))
                },
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

/// The card's INGREDIENTS block (its divider, heading and list) — hidden for
/// a perk without any (the classic ones).
#[derive(Component)]
struct PerkCardIngredients;

/// Shown instead of the perk card while the power's off.
#[derive(Component)]
struct NoPowerPrompt;

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
            // (Only one of the two shows at a time.)
            row.spawn((
                NoPowerPrompt,
                Node {
                    padding: UiRect::axes(Val::Px(22.0), Val::Px(12.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.25)),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_child((Text::new("THE POWER MUST BE ACTIVATED FIRST"), heading(26.0), TextColor(Color::WHITE)));
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
                card.spawn((PerkCardIngredients, divider.clone()));
                card.spawn((PerkCardIngredients, Text::new("INGREDIENTS"), heading(17.0), faint));
                card.spawn((
                    PerkCardIngredients,
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
/// One of the member's armor plates' bars (`.1`: 0 = the bottom plate),
/// and its blue fill.
#[derive(Component)]
struct PartyArmorBar(lightyear::prelude::PeerId, u8);
#[derive(Component)]
struct PartyArmorFill(lightyear::prelude::PeerId, u8);

/// Armor bar height (px), and the gap between them.
const ARMOR_BAR_H: f32 = 6.0;
const ARMOR_BAR_GAP: f32 = 4.0;

/// Health bar width / height (px).
const HEALTH_BAR_W: f32 = 240.0;
const HEALTH_BAR_H: f32 = 7.0;
pub(crate) const MONEY_YELLOW: Color = Color::srgb(1.0, 0.82, 0.1);

/// One member's panel: a yellow `$` and their points; a bar for each armor
/// plate they own (`shared::armor` — blue over half-clear black, three
/// across the health bar's width, only the owned ones shown, from the
/// left); a white health bar over a grey track, the number beside it; their
/// name.
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
        // The armor plates' bars, over the health bar.
        panel
            .spawn(Node {
                width: Val::Px(HEALTH_BAR_W),
                height: Val::Px(ARMOR_BAR_H),
                column_gap: Val::Px(ARMOR_BAR_GAP),
                ..default()
            })
            .with_children(|row| {
                let plates = shared::armor::MAX_LEVEL;
                let width = (HEALTH_BAR_W - ARMOR_BAR_GAP * (plates - 1) as f32) / plates as f32;
                for plate in 0..plates {
                    row.spawn((
                        PartyArmorBar(peer, plate),
                        Node {
                            width: Val::Px(width),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                        BorderRadius::all(Val::Px(2.0)),
                        Visibility::Hidden,
                    ))
                    .with_child((
                        PartyArmorFill(peer, plate),
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(crate::armor::ARMOR_BLUE),
                        BorderRadius::all(Val::Px(2.0)),
                    ));
                }
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
    mut fills: Query<(&PartyHealthFill, &mut Node), Without<PartyArmorFill>>,
    mut armor_bars: Query<(&PartyArmorBar, &mut Visibility), Without<PartyRoot>>,
    mut armor_fills: Query<(&PartyArmorFill, &mut Node), Without<PartyHealthFill>>,
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
    // (Juggernog raises a member's bar.)
    let max_hp = |peer| {
        lobby
            .members
            .iter()
            .find(|m| m.peer == peer)
            .map_or(shared::health::FULL_HEALTH, |m| shared::perks::max_health(&m.perks))
    };
    let hp = |peer| {
        health
            .iter()
            .find(|(id, _)| id.0 == peer)
            .map_or(max_hp(peer), |(_, h)| h.0)
            .clamp(0.0, max_hp(peer))
    };
    for (f, mut node) in &mut fills {
        let pct = Val::Percent(hp(f.0) / max_hp(f.0) * 100.0);
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
    // Armor: a bar for each plate owned, emptying toward the left.
    let armor = |peer| {
        lobby
            .members
            .iter()
            .find(|m| m.peer == peer)
            .map_or_else(shared::armor::Armor::default, |m| m.armor)
    };
    for (bar, mut v) in &mut armor_bars {
        v.set_if_neq(if bar.1 < armor(bar.0).level { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (fill, mut node) in &mut armor_fills {
        let pct = Val::Percent(armor(fill.0).plate_fill(fill.1) * 100.0);
        if node.width != pct {
            node.width = pct;
        }
    }
}

/// Owned-perk icon size and its gap from the bottom of the screen (px).
pub(crate) const PERK_ICON_SIZE: f32 = 100.0;
pub(crate) const PERK_ICON_BOTTOM: f32 = 36.0;

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
/// owned-perk row along the bottom (`update_perk_icons`). The classic ones
/// are Call of Duty's own.
pub(crate) fn perk_icon_path(perk: Perk) -> &'static str {
    match perk {
        Perk::ShroomTea => "textures/icons/perks/custom/shroom_tea.png",
        Perk::NitroBrew => "textures/icons/perks/custom/nitro_brew.png",
        Perk::LiquidCourage => "textures/icons/perks/custom/liquid_courage.png",
        Perk::BombShot => "textures/icons/perks/custom/bomb_shot.png",
        Perk::Kangabrew => "textures/icons/perks/custom/kangabrew.png",
        Perk::Juggernog => "textures/icons/perks/classic/juggernog.png",
        Perk::QuickRevive => "textures/icons/perks/classic/quick_revive.png",
        Perk::SpeedCola => "textures/icons/perks/classic/speed_cola.png",
        Perk::StaminUp => "textures/icons/perks/classic/stamin_up.png",
        Perk::DoubleTap => "textures/icons/perks/classic/double_tap.png",
        Perk::DeadshotDaiquiri => "textures/icons/perks/classic/deadshot_daiquiri.png",
        Perk::PhdFlopper => "textures/icons/perks/classic/phd_flopper.png",
        Perk::DeathPerception => "textures/icons/perks/classic/death_perception.png",
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
    revive: Res<crate::revive::LocalRevive>,
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
    // (While we're down they're under the bleed-out bar instead — `revive`.)
    let hud_up = !menu.is_open() && active_killcam.0.is_none() && !revive.downed && !revive.bled_out;
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
    /// The power's off — nothing sells yet (`shared::power::has_power`).
    NoPower,
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
    let perk = lobby
        .perk_set
        .machine_perks()
        .find(|&p| shared::perks::in_range_of(machines.placement(p, lobby.map).0, feet, 0.0))?;
    let status = if !shared::power::has_power(lobby.map, lobby.power_on) {
        PerkStatus::NoPower
    } else if member.perks.contains(&perk) {
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
    mut ingredients: Query<&mut Node, With<PerkCardIngredients>>,
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
    // (With the power off it's `update_no_power_prompt`'s line instead.)
    let Some((perk, status, points)) = here.filter(|(_, s, _)| *s != PerkStatus::NoPower) else {
        vis.set_if_neq(Visibility::Hidden);
        *shown = None;
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    if *shown != Some(perk) {
        *shown = Some(perk);
        icon.image = asset_server.load(perk_icon_path(perk));
        let display = if perk.ingredients().is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        for mut node in &mut ingredients {
            node.display = display;
        }
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
        PerkStatus::NoPower => unreachable!("filtered out above"),
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

/// At a perk machine with the power still off: Call of Duty's line in place
/// of the perk card (hidden behind menus and the kill cam, like the card).
fn update_no_power_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    machines: Res<PerkMachineSettings>,
    mut prompt: Single<&mut Visibility, With<NoPowerPrompt>>,
) {
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let no_power = !menu.is_open()
        && active_killcam.0.is_none()
        && zombies_game(&local, &lobbies)
            .zip(local.iter().next())
            .and_then(|(lobby, me)| perk_here(lobby, me.0, feet, &machines))
            .is_some_and(|(_, status, _)| status == PerkStatus::NoPower);
    prompt.set_if_neq(if no_power { Visibility::Inherited } else { Visibility::Hidden });
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
        s.trigger::<shared::LobbyChannel>(shared::BuyPerk { perk, wunderfizz: false });
        // (The server logs "bought ..." when it goes through; the card
        // switches to owned once that's replicated back.)
        info!("asked the server to buy {}", perk.label());
    }
}

/// PhD Flopper: sliding into a zombie stops the slide dead and asks the
/// server to set off the explosion — once a slide (the server checks the
/// perk, its cooldown and that a zombie really is that close).
fn phd_slam(
    mut slide: ResMut<crate::player::Slide>,
    classic: Res<ClassicPerks>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    poses: Query<&shared::PlayerPose>,
    mut sent: Local<bool>,
    mut sender: Query<&mut TriggerSender<shared::PhdSlam>, With<GameClient>>,
) {
    if slide.stance != crate::player::Stance::Sliding {
        *sent = false;
        return;
    }
    if *sent || !classic.owned.contains(&Perk::PhdFlopper) || zombies_game(&local, &lobbies).is_none() {
        return;
    }
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let hit = poses.iter().any(|p| {
        p.zombie.is_zombie()
            && p.alive
            && (p.translation - Vec3::Y * EYE_HEIGHT).distance(feet) <= shared::perks::PHD_SLAM_RADIUS
    });
    if !hit {
        return;
    }
    // The first zombie hit ends the slide on the spot.
    slide.stance = crate::player::Stance::Standing;
    slide.velocity = Vec3::ZERO;
    *sent = true;
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::PhdSlam);
    }
}

/// The moment we go prone at a perk machine (not Pack-a-Punch — it's no
/// perk), ask the server for its bonus. It pays only the first in the game
/// at each machine, so we just ask every time and let it decide.
fn prone_at_perk(
    slide: Res<crate::player::Slide>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    machines: Res<PerkMachineSettings>,
    mut was_prone: Local<bool>,
    mut sender: Query<&mut TriggerSender<shared::ProneAtPerk>, With<GameClient>>,
) {
    let prone = slide.stance == crate::player::Stance::Prone;
    let went_prone = prone && !*was_prone;
    *was_prone = prone;
    if !went_prone {
        return;
    }
    let Some(lobby) = zombies_game(&local, &lobbies) else {
        return;
    };
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let Some(perk) = lobby
        .perk_set
        .machine_perks()
        .find(|&p| shared::perks::in_range_of(machines.placement(p, lobby.map).0, feet, 0.0))
    else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::ProneAtPerk { perk });
    }
}

/// The server paid out our prone bonus: a bare `+100` and a ching, just
/// for us.
fn receive_prone_bonus(
    mut receivers: Query<&mut lightyear::prelude::MessageReceiver<shared::ProneBonus>>,
    sounds: Res<GameSounds>,
    mut scored: EventWriter<crate::TrickScoredEvent>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            scored.write(crate::TrickScoredEvent {
                total: msg.points,
                lines: Vec::new(),
                sound: Some(sounds.money_ching.clone()),
                color: crate::hud::SCORE_WHITE,
            });
        }
    }
}

/// Switch on what each perk does for as long as we own it in a running
/// `Zombies` game — Shroom Tea's shroom effect, Nitro Brew's multipliers,
/// Liquid Courage's drunk effect, the classic perks' [`ClassicPerks`] —
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
    (mut nitro, mut kanga, mut classic): (ResMut<NitroBrew>, ResMut<Kangabrew>, ResMut<ClassicPerks>),
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
    let has_kanga = owned.contains(&Perk::Kangabrew);
    if kanga.owned != has_kanga {
        kanga.owned = has_kanga;
    }
    let has_courage = owned.contains(&Perk::LiquidCourage);
    if courage.0 != has_courage {
        courage.0 = has_courage;
    }
    let classic_owned: Vec<Perk> = owned
        .iter()
        .copied()
        .filter(|p| p.set() == shared::perks::PerkSet::Classic)
        .collect();
    if classic.owned != classic_owned {
        classic.owned = classic_owned;
    }
    *prev = owned;
}
