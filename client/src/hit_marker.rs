//! Hit markers: when the server reports that one of *your* shots damaged a bot
//! or player without killing them (`shared::HitMarker`, sent to the shooter
//! only), play `hit-marker-sound.mp3` and flash a small white X at the
//! crosshair. A kill has its own feedback and sends none.

use std::f32::consts::FRAC_PI_4;

use bevy::prelude::*;
use lightyear::prelude::*;

use crate::killcam::ActiveKillCam;
use crate::{AppState, GameSounds};

/// How long the X takes to fade out (s).
const FLASH_SECS: f32 = 0.3;
/// Size of the marker's box (px), and of each bar of the X.
const BOX_PX: f32 = 28.0;
const BAR_LEN_PX: f32 = 20.0;
const BAR_THICK_PX: f32 = 3.0;

/// Time left on the current flash; `0` when none is showing.
#[derive(Resource, Default)]
struct HitMarkerFlash {
    remaining: f32,
}

/// One of the X's two bars.
#[derive(Component)]
struct HitMarkerBar;

pub struct HitMarkerPlugin;

impl Plugin for HitMarkerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitMarkerFlash>()
            .add_systems(OnEnter(AppState::InGame), spawn_hit_marker)
            .add_systems(
                Update,
                (receive_hit_markers, update_hit_marker)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn spawn_hit_marker(mut commands: Commands, mut flash: ResMut<HitMarkerFlash>) {
    flash.remaining = 0.0;
    let bar = |angle: f32| {
        (
            HitMarkerBar,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px((BOX_PX - BAR_LEN_PX) * 0.5),
                top: Val::Px((BOX_PX - BAR_THICK_PX) * 0.5),
                width: Val::Px(BAR_LEN_PX),
                height: Val::Px(BAR_THICK_PX),
                ..default()
            },
            Transform::from_rotation(Quat::from_rotation_z(angle)),
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.0)),
        )
    };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(6),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Px(BOX_PX),
                height: Val::Px(BOX_PX),
                ..default()
            })
            .with_children(|x| {
                x.spawn(bar(FRAC_PI_4));
                x.spawn(bar(-FRAC_PI_4));
            });
        });
}

/// A `HitMarker` from the server: sound + start the flash. Ignored during a
/// kill cam (it's the live world's shot).
fn receive_hit_markers(
    mut receivers: Query<&mut MessageReceiver<shared::HitMarker>>,
    killcam: Res<ActiveKillCam>,
    sounds: Res<GameSounds>,
    mut flash: ResMut<HitMarkerFlash>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            if killcam.0.is_some() {
                continue;
            }
            flash.remaining = FLASH_SECS;
            commands.spawn((
                AudioPlayer::new(sounds.hit_marker.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}

fn update_hit_marker(
    time: Res<Time>,
    mut flash: ResMut<HitMarkerFlash>,
    mut bars: Query<&mut BackgroundColor, With<HitMarkerBar>>,
) {
    if flash.remaining <= 0.0 {
        return;
    }
    flash.remaining = (flash.remaining - time.delta_secs()).max(0.0);
    let alpha = flash.remaining / FLASH_SECS;
    for mut bg in &mut bars {
        bg.0 = Color::srgba(1.0, 1.0, 1.0, alpha);
    }
}
