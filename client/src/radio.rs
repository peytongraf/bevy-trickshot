//! `Zombies`' man on the radio, Sergeant Frank Maddox ([`shared::radio`]):
//! a handler talking to the whole squad from somewhere else
//! (`notes/radio-quotes.md`). The server says when ([`shared::RadioSaid`] —
//! the intro as the game starts, a warning before the first dog round), and
//! every member's client plays the call at once, not positional and dry
//! (`audio/radio/`): a burst of static as he keys the mic, the line
//! (`maddox/`), and a click of static as he lets go ([`RadioCall`]).
//!
//! Nobody talks over him: the server lets no operator quote through while
//! he's on air, one already going is cut off as he starts, and our own perk
//! quotes (`zombies_hud::play_perk_quotes`) keep quiet too ([`on_air`]).
//!
//! While he's talking, his portrait and name sit on the right of the screen,
//! halfway between its middle and the ammo HUD in the bottom-right corner
//! ([`RadioPanel`]). The panel, the call and its sounds are
//! `StateScoped(InGame)`, so nothing carries between games.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::time::Real;

use crate::audio_engine::prelude::*;
use lightyear::prelude::MessageReceiver;
use shared::radio::{RadioLine, SQUELCH_END_PATH, SQUELCH_START_PATH, SQUELCH_START_SECS};

use crate::quotes::QuoteAudio;
use crate::{AppState, HUD_FONT};

/// The man on the radio, under his portrait.
const OPERATOR_NAME: &str = "Sergeant Frank Maddox";
/// His portrait's size (px, square).
const PORTRAIT_SIZE: f32 = 150.0;
/// How far (px) above the bottom of the screen the band the panel's
/// centred in ends — the top of the ammo HUD (`hud::ammo_text`), with room.
const AMMO_HUD_TOP: f32 = 120.0;
/// The panel's gap from the right edge (the ammo HUD's).
const PANEL_RIGHT: f32 = 28.0;
const PANEL_BG: Color = Color::srgba(0.0, 0.0, 0.0, 0.55);

pub(crate) struct RadioPlugin;

impl Plugin for RadioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_lines)
            .add_systems(OnEnter(AppState::InGame), spawn_panel)
            .add_systems(
                Update,
                (play_radio, run_calls, show_panel).chain().run_if(in_state(AppState::InGame)),
            );
    }
}

/// His lines, and the static either side of them, loaded at startup.
#[derive(Resource)]
struct RadioLines {
    lines: HashMap<RadioLine, Handle<AudioSource>>,
    squelch_start: Handle<AudioSource>,
    squelch_end: Handle<AudioSource>,
}

fn load_lines(asset_server: Res<AssetServer>, mut commands: Commands) {
    let lines = RadioLine::ALL
        .into_iter()
        .map(|line| (line, asset_server.load(line.path())))
        .collect();
    commands.insert_resource(RadioLines {
        lines,
        squelch_start: asset_server.load(SQUELCH_START_PATH),
        squelch_end: asset_server.load(SQUELCH_END_PATH),
    });
}

/// A call on air: `line`, since `started` (`Time<Real>` seconds) — its
/// static in, then the line once that's done ([`run_calls`]), then its
/// static out; gone once it's all played ([`RadioLine::airtime`]).
#[derive(Component)]
pub(crate) struct RadioCall {
    line: RadioLine,
    started: f32,
    voiced: bool,
    signed_off: bool,
}

/// One of a call's sounds playing (despawns when it's done).
#[derive(Component)]
struct RadioSound;

/// Whether he's on air right now.
pub(crate) fn on_air(calls: &Query<(), With<RadioCall>>) -> bool {
    !calls.is_empty()
}

/// The portrait panel, shown while a [`RadioCall`]'s on air.
#[derive(Component)]
struct RadioPanel;

/// Right side, centred in the band between the screen's middle and the ammo
/// HUD: the portrait over his name, on one background. Hidden until he talks.
fn spawn_panel(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(50.0),
                bottom: Val::Px(AMMO_HUD_TOP),
                right: Val::Px(PANEL_RIGHT),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|band| {
            band.spawn((
                RadioPanel,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|panel| {
                panel.spawn((
                    ImageNode::new(asset_server.load("textures/radio/maddox.png")),
                    Node {
                        width: Val::Px(PORTRAIT_SIZE),
                        height: Val::Px(PORTRAIT_SIZE),
                        ..default()
                    },
                    BorderRadius::all(Val::Px(4.0)),
                ));
                panel.spawn((
                    Text::new(OPERATOR_NAME.to_uppercase()),
                    TextFont {
                        font,
                        font_size: 20.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
        });
}

fn radio_sound(clip: &Handle<AudioSource>) -> impl Bundle {
    (
        Name::new("Radio"),
        RadioSound,
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip.clone()),
        PlaybackSettings::DESPAWN,
    )
}

/// The server says he speaks: key the mic (in place of any call of his
/// still going), and cut off any operator quote under it.
fn play_radio(
    mut receivers: Query<&mut MessageReceiver<shared::RadioSaid>>,
    time: Res<Time<Real>>,
    sounds: Res<RadioLines>,
    playing: Query<Entity, Or<(With<RadioCall>, With<RadioSound>, With<QuoteAudio>)>>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for said in rx.receive() {
            for e in &playing {
                commands.entity(e).despawn();
            }
            commands.spawn((
                Name::new("Radio call"),
                RadioCall {
                    line: said.line,
                    started: time.elapsed_secs(),
                    voiced: false,
                    signed_off: false,
                },
                StateScoped(AppState::InGame),
            ));
            commands.spawn(radio_sound(&sounds.squelch_start));
        }
    }
}

/// Each call on: the line once the static in's done, the static out once
/// the line is, and off air once that's done too.
fn run_calls(
    time: Res<Time<Real>>,
    sounds: Res<RadioLines>,
    mut calls: Query<(Entity, &mut RadioCall)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (e, mut call) in &mut calls {
        let t = now - call.started;
        if !call.voiced && t >= SQUELCH_START_SECS {
            call.voiced = true;
            if let Some(clip) = sounds.lines.get(&call.line) {
                commands.spawn(radio_sound(clip));
            }
        }
        if !call.signed_off && t >= SQUELCH_START_SECS + call.line.secs() {
            call.signed_off = true;
            commands.spawn(radio_sound(&sounds.squelch_end));
        }
        if t >= call.line.airtime() {
            commands.entity(e).despawn();
        }
    }
}

/// The panel's up exactly while he's on air.
fn show_panel(calls: Query<(), With<RadioCall>>, mut panel: Query<&mut Visibility, With<RadioPanel>>) {
    let shown = if !on_air(&calls) { Visibility::Hidden } else { Visibility::Inherited };
    for mut vis in &mut panel {
        vis.set_if_neq(shown);
    }
}
