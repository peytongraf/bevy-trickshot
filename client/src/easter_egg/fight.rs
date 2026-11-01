//! The easter egg's boss fight on this client ([`EggStep::BossFight`] — the
//! server's `easter_egg::fight` runs him):
//!
//! * **Him** — for now the boss's model and animations (`crate::boss`, his
//!   pose's `Boss*` states), until his own are made; his aura's a sickly
//!   spirit-green rather than a boss's orange ([`crate::boss::BossTint`]),
//!   so he's never mistaken for one.
//! * **His health bar** — top centre, under the compass: his name over a
//!   long bar of what's left of the health he came in with
//!   (`EasterEgg::hierophant`), a paler chunk trailing behind each hit.
//!   Up while he's alive in the fight, hidden behind menus and during a
//!   kill cam like the rest of the HUD.
//! * **His taunts** — [`shared::HierophantTaunt`]: the line, from where he
//!   stands (following him), heard far off (`mix.ron`'s "Hierophant
//!   voice").
//!
//! The bar's `StateScoped(InGame)`, hidden (and its trail reset) whenever
//! there's no fight; a taunt playing goes with his avatar (and the game).
//! Tuned in the debug panel's "Easter egg (Der Tempel)" → "The fight".

use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::{Interpolated, LocalId, MessageReceiver};
use shared::easter_egg::{EggStep, Hierophant};
use shared::{Lobby, PlayerHealth, PlayerId};

use crate::audio_engine::prelude::*;
use crate::boss::{BossTint, BossVisual};
use crate::net::{GameClient, RemoteAvatar};
use crate::{killcam, menu, AppState, HUD_FONT};

use super::egg_game;

pub(super) struct FightPlugin;

impl Plugin for FightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggFightSettings>()
            .add_systems(Startup, load_sounds)
            .add_systems(OnEnter(AppState::InGame), spawn_bar)
            .add_systems(
                Update,
                (tint_him, taunts, update_bar).chain().run_if(in_state(AppState::InGame)),
            );
    }
}

/// Panel-tunable fight ("The fight").
#[derive(Resource, Clone, Debug)]
pub(crate) struct EggFightSettings {
    /// His taunts' volume (linear, before the mix).
    pub(crate) taunt_volume: f32,
    /// How high (m) over his feet they come from.
    pub(crate) taunt_height: f32,
    /// His aura's colour (linear) — a boss's is orange.
    pub(crate) aura_color: [f32; 3],
    /// The health bar's width (px).
    pub(crate) bar_width: f32,
    /// How fast (of the bar a second) the paler chunk behind a hit drains,
    /// after holding this long (s).
    pub(crate) trail_speed: f32,
    pub(crate) trail_hold_secs: f32,
}

impl Default for EggFightSettings {
    fn default() -> Self {
        Self {
            taunt_volume: 2.0,
            taunt_height: 2.3,
            aura_color: [0.35, 1.0, 0.32],
            bar_width: 560.0,
            trail_speed: 0.35,
            trail_hold_secs: 0.5,
        }
    }
}

#[derive(Resource)]
struct FightSounds {
    taunts: Vec<Handle<AudioSource>>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(FightSounds {
        taunts: (1..=shared::easter_egg::TAUNTS)
            .map(|i| asset_server.load(format!("audio/zombies/easter_egg/hierophant/taunt_{i}.mp3")))
            .collect(),
    });
}

/// Him, while he's in the fight: who he is.
fn in_fight(lobby: Option<&Lobby>) -> Option<Hierophant> {
    lobby
        .filter(|l| l.easter_egg.step == EggStep::BossFight)
        .and_then(|l| l.easter_egg.hierophant)
}

/// His avatar, if it's here.
fn his_avatar(
    him: Hierophant,
    avatars: &Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    ids: &Query<&PlayerId>,
    interp: &Query<&Interpolated>,
) -> Option<Entity> {
    avatars
        .iter()
        .find(|(_, a)| crate::boss::peer_of(a.src, ids, interp) == Some(him.peer))
        .map(|(e, _)| e)
}

/// His avatar gets his colour (whoever he was — a corpse keeps it).
fn tint_him(
    settings: Res<EggFightSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    mut tints: Query<&mut BossTint>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    let Some(him) = egg_game(&local, &lobbies).and_then(|l| l.easter_egg.hierophant) else {
        return;
    };
    let Some(avatar) = his_avatar(him, &avatars, &ids, &interp) else {
        return;
    };
    match tints.get_mut(avatar) {
        Ok(mut tint) => {
            if tint.0 != settings.aura_color {
                tint.0 = settings.aura_color;
            }
        }
        Err(_) => {
            commands.entity(avatar).insert(BossTint(settings.aura_color));
        }
    }
}

/// One of his taunts playing.
#[derive(Component)]
struct Taunt;

/// His taunts, from where he stands.
#[allow(clippy::too_many_arguments)]
fn taunts(
    settings: Res<EggFightSettings>,
    sounds: Res<FightSounds>,
    mut receivers: Query<&mut MessageReceiver<shared::HierophantTaunt>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    scales: Query<&Transform, With<BossVisual>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    let him = in_fight(egg_game(&local, &lobbies));
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let (Some(him), Some(clip)) = (him, sounds.taunts.get(msg.line as usize)) else {
                continue;
            };
            let Some(avatar) = his_avatar(him, &avatars, &ids, &interp) else {
                continue;
            };
            // (Under his avatar, which is scaled.)
            let scale = scales.get(avatar).map_or(1.0, |t| t.scale.y.max(0.001));
            commands.entity(avatar).with_child((
                Taunt,
                AudioPlayer::new(clip.clone()),
                Transform::from_xyz(0.0, settings.taunt_height / scale, 0.0),
                crate::positional_playback(Volume::Linear(settings.taunt_volume)),
            ));
        }
    }
}

// --- his health bar ---------------------------------------------------------------------

#[derive(Component)]
struct HealthBar;

#[derive(Component)]
struct BarFill;

#[derive(Component)]
struct BarTrail;

#[derive(Component)]
struct BarFrame;

/// The paler chunk behind a hit: where it's at (0..1), and when it starts
/// draining (`Time::elapsed_secs`).
#[derive(Component, Default)]
struct Trail {
    at: Option<f32>,
    drain_from: f32,
}

const BAR_HEIGHT: f32 = 14.0;
const FILL: Color = Color::srgb(0.62, 0.05, 0.08);
const TRAIL: Color = Color::srgb(0.95, 0.78, 0.55);
const NAME: Color = Color::srgb(0.85, 1.0, 0.82);

fn spawn_bar(mut commands: Commands, asset_server: Res<AssetServer>, settings: Res<EggFightSettings>) {
    let font = asset_server.load(HUD_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            HealthBar,
            Trail::default(),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                // (Under the compass.)
                top: Val::Px(crate::hud::COMPASS_HEIGHT + 10.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn((
                Text::new("THE HIEROPHANT"),
                TextFont {
                    font,
                    font_size: 26.0,
                    ..default()
                },
                TextColor(NAME),
                TextShadow {
                    offset: Vec2::splat(2.0),
                    color: Color::BLACK.with_alpha(0.85),
                },
            ));
            col.spawn((
                BarFrame,
                Node {
                    width: Val::Px(settings.bar_width),
                    height: Val::Px(BAR_HEIGHT),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
                BorderColor(Color::srgba(0.45, 0.9, 0.42, 0.75)),
            ))
            .with_children(|frame| {
                frame.spawn((
                    BarTrail,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(TRAIL),
                ));
                frame.spawn((
                    BarFill,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(FILL),
                ));
            });
        });
}

/// Up while he's alive in the fight (not behind a menu or during a kill
/// cam): what's left of his health, the chunk behind a hit trailing after.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_bar(
    time: Res<Time>,
    settings: Res<EggFightSettings>,
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    healths: Query<(&PlayerId, &PlayerHealth)>,
    bar: Single<(&mut Visibility, &mut Trail), With<HealthBar>>,
    mut frame: Single<&mut Node, (With<BarFrame>, Without<BarFill>, Without<BarTrail>)>,
    mut fill: Single<&mut Node, (With<BarFill>, Without<BarFrame>, Without<BarTrail>)>,
    mut trail_node: Single<&mut Node, (With<BarTrail>, Without<BarFrame>, Without<BarFill>)>,
) {
    let (mut visibility, mut trail) = bar.into_inner();
    let him = in_fight(egg_game(&local, &lobbies));
    let left = him.and_then(|h| {
        let health = healths.iter().find(|(id, _)| id.0 == h.peer).map(|(_, hp)| hp.0)?;
        Some((health / h.max_health.max(1.0)).clamp(0.0, 1.0))
    });
    let Some(left) = left.filter(|l| *l > 0.0) else {
        visibility.set_if_neq(Visibility::Hidden);
        trail.at = None;
        return;
    };
    let shown = !menu.is_open() && active_killcam.0.is_none();
    visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    let now = time.elapsed_secs();
    let draining = now >= trail.drain_from;
    let at = trail.at.get_or_insert(left);
    if left > *at {
        // (Healed, or a fresh one: no trail.)
        *at = left;
    } else if left < *at && draining {
        *at = (*at - settings.trail_speed * time.delta_secs()).max(left);
    }
    let at = *at;
    // A hit just landed: the trail holds a moment before draining.
    if fill.width != Val::Percent(left * 100.0) {
        if let Val::Percent(was) = fill.width {
            if left * 100.0 < was - 0.01 {
                trail.drain_from = now + settings.trail_hold_secs;
            }
        }
        fill.width = Val::Percent(left * 100.0);
    }
    if trail_node.width != Val::Percent(at * 100.0) {
        trail_node.width = Val::Percent(at * 100.0);
    }
    if frame.width != Val::Px(settings.bar_width) {
        frame.width = Val::Px(settings.bar_width);
    }
}

// --- the debug panel --------------------------------------------------------------------

/// The debug panel's controls for it.
pub(crate) fn fight_section(ui: &mut egui::Ui, s: &mut EggFightSettings, lobby: Option<&Lobby>) {
    match lobby.and_then(|l| l.easter_egg.hierophant).filter(|_| lobby.is_some_and(|l| l.easter_egg.step == EggStep::BossFight)) {
        Some(h) => ui.label(format!("now: he's in ({:.0} health to start) — he's the boss model for now", h.max_health)),
        None => ui.label("now: no fight — jump to \"Boss fight\" to bring him in"),
    };
    ui.add(egui::Slider::new(&mut s.taunt_volume, 0.0f32..=3.0).text("taunt volume"));
    ui.add(egui::Slider::new(&mut s.taunt_height, 0.0f32..=4.0).text("taunts from (m up)"));
    ui.horizontal(|ui| {
        ui.label("aura colour");
        ui.color_edit_button_rgb(&mut s.aura_color);
    });
    ui.add(egui::Slider::new(&mut s.bar_width, 120.0f32..=1200.0).text("health bar width (px)"));
    ui.add(egui::Slider::new(&mut s.trail_hold_secs, 0.0f32..=3.0).text("hit trail holds (s)"));
    ui.add(egui::Slider::new(&mut s.trail_speed, 0.01f32..=3.0).text("hit trail drains (of the bar / s)"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = EggFightSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
