//! `Zombies` last stand and reviving, as the client shows it — the rules and
//! the clocks are the server's (`server::revive`, `shared::revive`), which
//! replicates each downed player's state as [`Downed`] (only on events —
//! [`DownClock`] runs the clocks on locally in between).
//!
//! * **Down** (us): forced prone, crawling ([`DOWNED_CRAWL_SPEED`]), no
//!   weapon — `weapon_system` is off and the view model's hidden. The world
//!   drains to black and white and the blood overlay (`health`) fades in as
//!   the bleed-out runs down. The bleed-out bar sits low in the middle with
//!   the perks we went down with spaced along under it, each at the point
//!   it's lost ([`shared::revive::perk_mark`]); above it, while someone's
//!   reviving us, who and how far along.
//! * **Bled out** (us): fully black and white until the next round brings us
//!   back (a `PlayerRespawn`).
//! * **A teammate down**: the revive icon over them, through walls, white
//!   going red as they bleed out; in reach, "hold [interact] to revive", and
//!   holding it ([`PlayerInput::revive`]) stops us, lowers the weapon and
//!   shows our progress.
//!
//! Everything's derived from the replicated state each frame (and the UI is
//! `StateScoped(InGame)`), so nothing carries into the next game; the view
//! effects are put back by [`apply_down_view`], which runs outside a game too.

use bevy::prelude::*;
use bevy::render::view::{ColorGrading, RenderLayers};
use bevy::transform::helper::TransformHelper;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use lightyear::prelude::input::client::InputSet;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::{Interpolated, LocalId, MessageReceiver, PeerId};
use shared::perks::Perk;
use shared::revive::{
    Downed, PlayerRevived, PlayerWentDown, BLEED_OUT_SECS, DOWNED_CRAWL_SPEED, REVIVE_RANGE,
};
use shared::{Lobby, PlayerId, PlayerInput, PlayerPose};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::player::{Slide, Stance, ViewModelCamera, WorldModelCamera};
use crate::zombies_hud::{perk_icon_path, zombies_game};
use crate::{
    killcam, menu, AppState, GameSounds, Player, EYE_HEIGHT, HUD_FONT, VIEW_MODEL_RENDER_LAYER,
};

/// The revive icon (over a downed teammate, and beside our bleed-out bar).
const REVIVE_ICON: &str = "textures/icons/revive.webp";

/// How fast (m/s) a downed player crawls — `move_player` uses it.
pub(crate) const CRAWL_SPEED: f32 = DOWNED_CRAWL_SPEED;

const BLEED_BAR_W: f32 = 420.0;
const BAR_H: f32 = 10.0;
const REVIVE_BAR_W: f32 = 360.0;
const PERK_SIZE: f32 = 44.0;
const MARKER_SIZE: f32 = 44.0;
/// How high (m) above a downed teammate's feet their revive icon sits.
const MARKER_HEIGHT: f32 = 0.9;
const BLEED_RED: Color = Color::srgb(0.85, 0.12, 0.1);

pub(crate) struct RevivePlugin;

impl Plugin for RevivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalRevive>()
            .add_systems(OnEnter(AppState::InGame), (reset_local, spawn_revive_hud))
            // Before anything's run conditions read it this frame.
            .add_systems(
                PreUpdate,
                (tick_down_clocks, track_local)
                    .chain()
                    .after(bevy::input::InputSystem),
            )
            .add_systems(
                FixedPreUpdate,
                write_revive_input
                    .in_set(InputSet::WriteClientInputs)
                    .after(crate::net::write_input),
            )
            .add_systems(
                Update,
                (
                    update_down_panel,
                    update_revive_panel,
                    update_revive_markers,
                    play_revive_sounds,
                )
                    .run_if(in_state(AppState::InGame)),
            )
            // Not gated on `InGame`: it's what puts the view back once the
            // game's left.
            .add_systems(Update, apply_down_view);
    }
}

/// Where we stand: down, out, or reviving someone — read by the movement,
/// stance, weapon and ADS systems (and the HUD).
#[derive(Resource, Default, Clone)]
pub(crate) struct LocalRevive {
    /// We're down (bleeding out, or getting ourselves up with solo Quick
    /// Revive).
    pub(crate) downed: bool,
    /// We've bled out (or fallen out of the world): dead until next round.
    pub(crate) bled_out: bool,
    /// How far our bleed-out's gone (`0.0` just down … `1.0` out).
    pub(crate) progress: f32,
    /// The teammate we're holding the key to revive.
    pub(crate) reviving: Option<PeerId>,
    /// The downed teammate in reach we'd revive (the prompt).
    prompt: Option<PeerId>,
}

impl LocalRevive {
    /// No shooting, aiming, stabbing or throwing: down, out, or busy
    /// reviving.
    pub(crate) fn blocks_weapon(&self) -> bool {
        self.downed || self.bled_out || self.reviving.is_some()
    }
}

/// Run condition: we're not down or out (`Zombies`).
pub(crate) fn up(local: Res<LocalRevive>) -> bool {
    !local.downed && !local.bled_out
}

fn reset_local(mut local: ResMut<LocalRevive>) {
    *local = LocalRevive::default();
}

/// [`Downed`]'s clocks run on locally from its last update: the bleed-out
/// down while nobody's reviving, the revive up while somebody is (both held
/// while the game's paused).
#[derive(Component, Clone, Copy)]
pub(crate) struct DownClock {
    bleed_left: f32,
    revive_done: f32,
}

impl DownClock {
    fn from(downed: &Downed) -> Self {
        Self {
            bleed_left: downed.bleed_left,
            revive_done: 0.0,
        }
    }

    /// How far the bleed-out's gone, `0.0` … `1.0`.
    fn progress(&self, downed: &Downed) -> f32 {
        if downed.bled_out {
            1.0
        } else {
            (1.0 - self.bleed_left / BLEED_OUT_SECS).clamp(0.0, 1.0)
        }
    }

    /// How far the current revive is, `0.0` … `1.0`.
    fn revive_frac(&self, downed: &Downed) -> f32 {
        if downed.revive_secs <= 0.0 {
            0.0
        } else {
            (self.revive_done / downed.revive_secs).clamp(0.0, 1.0)
        }
    }
}

fn tick_down_clocks(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut downs: Query<(Entity, Ref<Downed>, Option<&mut DownClock>)>,
    mut commands: Commands,
) {
    let paused = zombies_game(&local, &lobbies).is_some_and(|l| l.paused);
    let dt = time.delta_secs();
    for (entity, downed, clock) in &mut downs {
        let Some(mut clock) = clock else {
            commands.entity(entity).insert(DownClock::from(&downed));
            continue;
        };
        if downed.is_changed() {
            *clock = DownClock::from(&downed);
        } else if !paused {
            if downed.reviver.is_some() {
                clock.revive_done += dt;
            } else {
                clock.bleed_left = (clock.bleed_left - dt).max(0.0);
            }
        }
    }
}

/// The server's say-so: a lobby member's gone down (everyone hears it), or a
/// revive we were part of — revived or reviving — just finished. Played flat,
/// not from where it happened.
fn play_revive_sounds(
    mut downs: Query<&mut MessageReceiver<PlayerWentDown>>,
    mut revives: Query<&mut MessageReceiver<PlayerRevived>>,
    sounds: Option<Res<GameSounds>>,
    mut commands: Commands,
) {
    let mut play = |clip: &Handle<AudioSource>| {
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(clip.clone()),
            PlaybackSettings::DESPAWN,
        ));
    };
    for mut rx in &mut downs {
        for _ in rx.receive() {
            if let Some(sounds) = &sounds {
                play(&sounds.player_down);
            }
        }
    }
    for mut rx in &mut revives {
        for _ in rx.receive() {
            if let Some(sounds) = &sounds {
                play(&sounds.revived);
            }
        }
    }
}

/// Work out [`LocalRevive`]: our own state from our [`Downed`], and — while
/// we're up — which downed teammate's in reach and whether we're holding
/// the interact key on them (sticking with whoever we started on).
#[allow(clippy::too_many_arguments)]
fn track_local(
    state: Res<State<AppState>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    menu: Res<menu::Menu>,
    window: Query<&Window, With<PrimaryWindow>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    downs: Query<(&PlayerId, &Downed, Option<&DownClock>, Option<&PlayerPose>)>,
    player: Query<&Transform, With<Player>>,
    mut out: ResMut<LocalRevive>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = zombies_game(&local, &lobbies).filter(|_| *state.get() == AppState::InGame);
    let (Some(me), Some(lobby)) = (me, lobby) else {
        if out.downed || out.bled_out || out.reviving.is_some() || out.prompt.is_some() {
            *out = LocalRevive::default();
        }
        return;
    };
    let mine = downs.iter().find(|(id, ..)| id.0 == me);
    let mut next = LocalRevive {
        downed: mine.is_some_and(|(_, d, ..)| !d.bled_out),
        bled_out: mine.is_some_and(|(_, d, ..)| d.bled_out),
        progress: mine.map_or(0.0, |(_, d, c, _)| c.map_or(0.0, |c| c.progress(d))),
        ..default()
    };
    if mine.is_none() && !lobby.paused {
        if let Ok(player) = player.single() {
            let feet = player.translation - Vec3::Y * EYE_HEIGHT;
            // (A touch inside the server's reach, so it never disagrees.)
            let in_reach = |pose: &PlayerPose| {
                let at = pose.translation - Vec3::Y * EYE_HEIGHT;
                Vec2::new(at.x - feet.x, at.z - feet.z).length() <= REVIVE_RANGE - 0.2
                    && (at.y - feet.y).abs() <= 1.4
            };
            // Downed teammates someone else isn't already reviving (and
            // not getting themselves up).
            let candidates: Vec<(PeerId, f32)> = downs.iter().filter_map(|(id, d, _, pose)| {
                let pose = pose?;
                (id.0 != me
                    && lobby.has(id.0)
                    && !d.bled_out
                    && d.reviver.is_none_or(|r| r == me)
                    && in_reach(pose))
                .then_some((id.0, pose.translation.distance_squared(player.translation)))
            }).collect();
            let prev = out.reviving;
            let nearest = candidates
                .iter()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(peer, _)| *peer);
            let target = prev
                .filter(|p| candidates.iter().any(|(c, _)| c == p))
                .or(nearest);
            let locked = window
                .single()
                .is_ok_and(|w| w.cursor_options.grab_mode != CursorGrabMode::None);
            let holding = locked && !menu.is_open() && binds.interact.pressed(&keys, &mouse);
            next.prompt = target;
            next.reviving = target.filter(|_| holding);
        }
    }
    if next.downed != out.downed
        || next.bled_out != out.bled_out
        || next.progress != out.progress
        || next.reviving != out.reviving
        || next.prompt != out.prompt
    {
        *out = next;
    }
}

/// Tell the server we're holding the key on someone (it checks the rest).
fn write_revive_input(
    local: Res<LocalRevive>,
    mut q: Query<&mut ActionState<PlayerInput>, With<InputMarker<PlayerInput>>>,
) {
    if let Ok(mut action) = q.single_mut() {
        action.revive = local.reviving.is_some();
    }
}

/// Going down: flat on the floor. Getting back up: on our feet. (Called by
/// `crouch_slide`, which owns the stance.) Returns whether we're down, so it
/// can ignore the stance keys.
pub(crate) fn hold_downed_stance(local: &LocalRevive, slide: &mut Slide, was_down: &mut bool) -> bool {
    let down = local.downed;
    if down {
        slide.stance = Stance::Prone;
        slide.velocity = Vec3::ZERO;
    } else if *was_down {
        slide.stance = Stance::Standing;
    }
    *was_down = down;
    down
}

/// The world drains to black and white as our bleed-out runs (all the way
/// once we're out), and the view model's hidden while we're down, out or
/// reviving someone (an empty render layer on its camera, so the screen
/// passes it carries — Shroom Tea, Liquid Courage — still run). Back to
/// normal at once otherwise, including outside a game.
fn apply_down_view(
    time: Res<Time>,
    local: Res<LocalRevive>,
    mut saturation: Local<Option<f32>>,
    mut grading: Query<&mut ColorGrading, Or<(With<WorldModelCamera>, With<ViewModelCamera>)>>,
    mut vm_layers: Query<&mut RenderLayers, With<ViewModelCamera>>,
) {
    let target = if local.bled_out {
        0.0
    } else if local.downed {
        1.0 - local.progress
    } else {
        1.0
    };
    let current = saturation.unwrap_or(1.0);
    // Eased, so the revive's colour comes back over a moment.
    let next = if (target - current).abs() < 0.002 {
        target
    } else {
        current + (target - current) * (time.delta_secs() * 4.0).min(1.0)
    };
    if *saturation != Some(next) {
        *saturation = Some(next);
        for mut g in &mut grading {
            g.global.post_saturation = next;
        }
    }
    let want = if local.blocks_weapon() {
        RenderLayers::none()
    } else {
        RenderLayers::layer(VIEW_MODEL_RENDER_LAYER)
    };
    for mut layers in &mut vm_layers {
        if *layers != want {
            *layers = want.clone();
        }
    }
}

// --- HUD ---------------------------------------------------------------

/// Our own last-stand panel (low centre), up while we're down or out.
#[derive(Component)]
struct DownPanel;
#[derive(Component)]
struct DownReviveText;
/// The panel's parts [`update_down_panel`] shows, hides and fills.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum DownPart {
    /// "BEING REVIVED BY ..." and its bar, while someone's reviving us.
    ReviveRow,
    ReviveFill,
    /// The bleed-out bar, its icon and the perks under it.
    BleedBlock,
    BleedFill,
    /// "BLED OUT" once we're out.
    OutText,
}
/// The `index`th perk we went down with, at its mark under the bar.
#[derive(Component)]
struct DownPerkSlot {
    index: usize,
    shown: Option<Perk>,
}

/// Reviving a teammate: the prompt, or who and how far along.
#[derive(Component)]
struct RevivePanel;
#[derive(Component)]
struct RevivePanelText;
#[derive(Component)]
struct RevivePanelTrack;
#[derive(Component)]
struct RevivePanelFill;

/// The revive icon over a downed teammate.
#[derive(Component)]
struct ReviveMarker(PeerId);

fn text(font: &Handle<Font>, size: f32, color: Color) -> (TextFont, TextColor, TextShadow) {
    (
        TextFont {
            font: font.clone(),
            font_size: size,
            ..default()
        },
        TextColor(color),
        TextShadow {
            offset: Vec2::splat(2.0),
            color: Color::srgba(0.0, 0.0, 0.0, 0.8),
        },
    )
}

/// A bar's dark track with its fill (`fill` marks the fill node).
fn bar(parent: &mut ChildSpawnerCommands, width: f32, color: Color, fill: impl Bundle, track: impl Bundle) {
    parent
        .spawn((
            track,
            Node {
                width: Val::Px(width),
                height: Val::Px(BAR_H),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            BorderColor(Color::srgba(1.0, 1.0, 1.0, 0.35)),
        ))
        .with_child((
            fill,
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(color),
        ));
}

fn spawn_revive_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let icon = asset_server.load(REVIVE_ICON);

    // Our own last stand: above the blood overlay (`health`, 9) and the
    // death overlay (10).
    commands
        .spawn((
            DownPanel,
            StateScoped(AppState::InGame),
            GlobalZIndex(11),
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Percent(24.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(18.0),
                ..default()
            },
        ))
        .with_children(|col| {
            col.spawn((
                DownPart::ReviveRow,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(6.0),
                    display: Display::None,
                    ..default()
                },
            ))
            .with_children(|row| {
                row.spawn((DownReviveText, Text::new(""), text(&font, 26.0, Color::WHITE)));
                bar(row, REVIVE_BAR_W, Color::WHITE, DownPart::ReviveFill, ());
            });
            col.spawn((
                DownPart::BleedBlock,
                Node {
                    width: Val::Px(BLEED_BAR_W),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ))
            .with_children(|block| {
                // The icon to the bar's left, centred on it.
                block.spawn((
                    ImageNode::new(icon.clone()).with_color(BLEED_RED),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(-(MARKER_SIZE + 12.0)),
                        top: Val::Px((BAR_H - MARKER_SIZE) / 2.0),
                        width: Val::Px(MARKER_SIZE),
                        height: Val::Px(MARKER_SIZE),
                        ..default()
                    },
                ));
                bar(block, BLEED_BAR_W, BLEED_RED, DownPart::BleedFill, ());
                // The perks, each at its mark (`update_down_panel`).
                block
                    .spawn(Node {
                        width: Val::Px(BLEED_BAR_W),
                        height: Val::Px(PERK_SIZE),
                        margin: UiRect::top(Val::Px(10.0)),
                        ..default()
                    })
                    .with_children(|strip| {
                        for index in 0..Perk::MAX_PER_SET {
                            strip.spawn((
                                DownPerkSlot { index, shown: None },
                                ImageNode::default(),
                                Node {
                                    position_type: PositionType::Absolute,
                                    width: Val::Px(PERK_SIZE),
                                    height: Val::Px(PERK_SIZE),
                                    display: Display::None,
                                    ..default()
                                },
                            ));
                        }
                    });
            });
            col.spawn((
                DownPart::OutText,
                Text::new("BLED OUT · BACK NEXT ROUND"),
                text(&font, 34.0, Color::srgb(0.85, 0.85, 0.85)),
                Node {
                    display: Display::None,
                    ..default()
                },
            ));
        });

    // Reviving a teammate: a little above where our own bars would be.
    commands
        .spawn((
            RevivePanel,
            StateScoped(AppState::InGame),
            GlobalZIndex(11),
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Percent(30.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            },
        ))
        .with_children(|col| {
            col.spawn((RevivePanelText, Text::new(""), text(&font, 26.0, Color::WHITE)));
            bar(col, REVIVE_BAR_W, Color::WHITE, RevivePanelFill, RevivePanelTrack);
        });
}

fn name_of(lobby: &Lobby, peer: PeerId) -> String {
    lobby
        .members
        .iter()
        .find(|m| m.peer == peer)
        .map_or_else(|| "TEAMMATE".into(), |m| m.name.to_uppercase())
}

/// Fill in our own last-stand panel: the bleed-out bar draining, the perks
/// we went down with at their marks (gone once lost), and who's reviving us.
#[allow(clippy::too_many_arguments)]
fn update_down_panel(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    downs: Query<(&PlayerId, &Downed, &DownClock)>,
    mut panel: Single<&mut Visibility, With<DownPanel>>,
    mut parts: Query<(&DownPart, &mut Node), Without<DownPerkSlot>>,
    mut slots: Query<(&mut DownPerkSlot, &mut ImageNode, &mut Node), Without<DownPart>>,
    mut revive_text: Single<&mut Text, With<DownReviveText>>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = zombies_game(&local, &lobbies);
    let mine = me.and_then(|me| downs.iter().find(|(id, ..)| id.0 == me));
    let (Some(lobby), Some((_, downed, clock)), Some(me)) = (lobby, mine, me) else {
        panel.set_if_neq(Visibility::Hidden);
        return;
    };
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    panel.set_if_neq(if hud_up { Visibility::Inherited } else { Visibility::Hidden });
    let display = |on: bool| if on { Display::Flex } else { Display::None };

    let out = downed.bled_out;
    let reviver = downed.reviver.filter(|_| !out);
    if let Some(r) = reviver {
        let line = if r == me {
            "REVIVING YOURSELF".to_string()
        } else {
            format!("BEING REVIVED BY {}", name_of(lobby, r))
        };
        if revive_text.0 != line {
            revive_text.0 = line;
        }
    }
    for (part, mut node) in &mut parts {
        let (shown, fill) = match part {
            DownPart::ReviveRow => (reviver.is_some(), None),
            DownPart::ReviveFill => (true, Some(clock.revive_frac(downed))),
            DownPart::BleedBlock => (!out, None),
            DownPart::BleedFill => (true, Some(1.0 - clock.progress(downed))),
            DownPart::OutText => (out, None),
        };
        if node.display != display(shown) {
            node.display = display(shown);
        }
        if let Some(frac) = fill {
            let w = Val::Percent(frac.clamp(0.0, 1.0) * 100.0);
            if node.width != w {
                node.width = w;
            }
        }
    }

    // What we've still got of what we went down with.
    let have: &[Perk] = lobby
        .members
        .iter()
        .find(|m| m.peer == me)
        .map_or(&[], |m| m.perks.as_slice());
    let count = downed.perks.len();
    for (mut slot, mut image, mut node) in &mut slots {
        let perk = downed.perks.get(slot.index).copied().filter(|p| !out && have.contains(p));
        if slot.shown != perk {
            slot.shown = perk;
            if let Some(perk) = perk {
                image.image = asset_server.load(perk_icon_path(perk));
            }
        }
        let left = Val::Px(shared::revive::perk_mark(slot.index, count) * BLEED_BAR_W - PERK_SIZE / 2.0);
        if node.left != left {
            node.left = left;
        }
        if node.display != display(perk.is_some()) {
            node.display = display(perk.is_some());
        }
    }
}

/// Reviving a teammate: "hold [interact] to revive" while one's in reach,
/// then — holding it — who and how far along (the server's word on it, run
/// on locally).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_revive_panel(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    revive: Res<LocalRevive>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    downs: Query<(&PlayerId, &Downed, &DownClock)>,
    mut panel: Single<&mut Visibility, With<RevivePanel>>,
    mut text: Single<&mut Text, With<RevivePanelText>>,
    mut track: Single<&mut Node, (With<RevivePanelTrack>, Without<RevivePanelFill>)>,
    mut fill: Single<&mut Node, (With<RevivePanelFill>, Without<RevivePanelTrack>)>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = zombies_game(&local, &lobbies);
    let target = revive.reviving.or(revive.prompt);
    let (Some(lobby), Some(target), Some(me)) = (lobby, target, me) else {
        panel.set_if_neq(Visibility::Hidden);
        return;
    };
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    panel.set_if_neq(if hud_up { Visibility::Inherited } else { Visibility::Hidden });
    let name = name_of(lobby, target);
    let line = if revive.reviving.is_some() {
        format!("REVIVING {name}")
    } else {
        format!("HOLD {} TO REVIVE {name}", binds.interact.label().to_uppercase())
    };
    if text.0 != line {
        text.0 = line;
    }
    let display = if revive.reviving.is_some() { Display::Flex } else { Display::None };
    if track.display != display {
        track.display = display;
    }
    // (Zero until the server says we've started.)
    let frac = downs
        .iter()
        .find(|(id, d, _)| id.0 == target && d.reviver == Some(me))
        .map_or(0.0, |(_, d, c)| c.revive_frac(d));
    let w = Val::Percent(frac * 100.0);
    if fill.width != w {
        fill.width = w;
    }
}

/// The revive icon over every downed teammate (not us, not the bled out),
/// through walls, pinned to the screen edge when they're off it — white
/// going red as they bleed out.
#[allow(clippy::too_many_arguments)]
fn update_revive_markers(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    downs: Query<(&PlayerId, &Downed, &DownClock, Option<&PlayerPose>)>,
    smooth: Query<(&PlayerId, &PlayerPose), With<Interpolated>>,
    camera: Single<(Entity, &Camera), With<WorldModelCamera>>,
    transforms: TransformHelper,
    window: Single<&Window, With<PrimaryWindow>>,
    mut markers: Query<(Entity, &ReviveMarker, &mut Node, &mut ImageNode, &mut Visibility)>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = zombies_game(&local, &lobbies);
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    let (cam_entity, cam) = camera.into_inner();
    let cam_gt = transforms.compute_global_transform(cam_entity).ok();
    let size = window.size();

    let wanted: Vec<(PeerId, Vec3, Color)> = match (lobby, me) {
        (Some(lobby), Some(me)) => downs
            .iter()
            .filter(|(id, d, ..)| id.0 != me && !d.bled_out && lobby.has(id.0))
            .filter_map(|(id, d, clock, pose)| {
                let pose = smooth
                    .iter()
                    .find(|(sid, _)| sid.0 == id.0)
                    .map(|(_, p)| p)
                    .or(pose)?;
                let anchor = pose.translation - Vec3::Y * (EYE_HEIGHT - MARKER_HEIGHT);
                let t = clock.progress(d);
                let color = Color::WHITE.mix(&BLEED_RED, t);
                Some((id.0, anchor, color))
            })
            .collect(),
        _ => Vec::new(),
    };

    for (entity, marker, ..) in &markers {
        if !wanted.iter().any(|(p, ..)| *p == marker.0) {
            commands.entity(entity).try_despawn();
        }
    }
    for (peer, anchor, color) in wanted {
        let Some((_, _, mut node, mut image, mut vis)) = markers.iter_mut().find(|(_, m, ..)| m.0 == peer) else {
            commands.spawn((
                ReviveMarker(peer),
                StateScoped(AppState::InGame),
                GlobalZIndex(6),
                ImageNode::new(asset_server.load(REVIVE_ICON)).with_color(color),
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(MARKER_SIZE),
                    height: Val::Px(MARKER_SIZE),
                    ..default()
                },
                Visibility::Hidden,
            ));
            continue;
        };
        // On screen where they are; behind us or off the side, along the
        // nearest edge.
        let screen = cam_gt.and_then(|gt| {
            let ahead = (anchor - gt.translation()).dot(*gt.forward()) > 0.0;
            match cam.world_to_viewport(&gt, anchor) {
                Ok(p) if ahead => Some(p),
                _ => {
                    let local = gt.compute_matrix().inverse().transform_point3(anchor);
                    let dir = Vec2::new(local.x, -local.y).normalize_or(Vec2::Y);
                    Some(size / 2.0 + dir * size.length())
                }
            }
        });
        let Some(screen) = screen else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        let half = MARKER_SIZE / 2.0;
        let x = screen.x.clamp(half + 8.0, size.x - half - 8.0);
        let y = screen.y.clamp(half + 8.0, size.y - half - 8.0);
        let (left, top) = (Val::Px(x - half), Val::Px(y - half));
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        if image.color != color {
            image.color = color;
        }
        vis.set_if_neq(if hud_up { Visibility::Inherited } else { Visibility::Hidden });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_starts_from_the_snapshot() {
        let downed = Downed {
            perks: vec![],
            bleed_left: BLEED_OUT_SECS / 2.0,
            reviver: None,
            revive_secs: 3.0,
            bled_out: false,
        };
        let c = DownClock::from(&downed);
        assert!((c.progress(&downed) - 0.5).abs() < 1e-5);
        assert_eq!(c.revive_frac(&downed), 0.0);
        assert_eq!(c.progress(&Downed { bled_out: true, ..downed }), 1.0);
    }
}
