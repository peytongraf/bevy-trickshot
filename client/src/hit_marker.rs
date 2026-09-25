//! Hit markers: the server tells the attacker when their shot, stab or thrown
//! knife hurt a bot or player (`shared::HitMarker { kill: false }`) or killed
//! one (`kill: true`). Either way an X flashes on the crosshair — its four
//! arms fly outward from the centre and it fades fast. A hit is a white X with
//! `audio/combat/hit_marker.mp3`; a kill is a bigger red X (the kill sound
//! comes from the kill's own feedback).
//!
//! The X is a "+" of four bars in a box turned 45°, so each bar just slides
//! straight out along its own axis. Spawned per game (`StateScoped`) and
//! cleared on respawn — nothing carries between games or lives.

use std::f32::consts::FRAC_PI_4;

use bevy::prelude::*;
use lightyear::prelude::*;

use crate::killcam::ActiveKillCam;
use crate::{AppState, GameSounds};

/// The box the X is drawn in (px) — big enough for a kill X at full spread.
const BOX_PX: f32 = 96.0;
const BAR_THICK_PX: f32 = 3.0;

/// One marker's look: colour, how long it lasts, and how its arms move.
struct MarkerStyle {
    color: Color,
    secs: f32,
    /// Arm length (px).
    len: f32,
    /// Gap from the centre to each arm's inner end (px), at the start and
    /// the end of the flash.
    gap_from: f32,
    gap_to: f32,
}

const HIT: MarkerStyle = MarkerStyle {
    color: Color::WHITE,
    secs: 0.25,
    len: 9.0,
    gap_from: 3.0,
    gap_to: 14.0,
};

const KILL: MarkerStyle = MarkerStyle {
    color: Color::srgb(0.95, 0.12, 0.12),
    secs: 0.35,
    len: 13.0,
    gap_from: 4.0,
    gap_to: 22.0,
};

/// The flash showing now, if any.
#[derive(Resource, Default)]
struct HitMarkerFlash {
    /// `Some(kill)` while one is up.
    kill: Option<bool>,
    /// Seconds since it started.
    age: f32,
}

/// One arm of the X, pointing out along `dir` (unit, box space before the
/// 45° turn).
#[derive(Component)]
struct HitMarkerArm {
    dir: Vec2,
}

pub struct HitMarkerPlugin;

impl Plugin for HitMarkerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitMarkerFlash>()
            .add_systems(OnEnter(AppState::InGame), spawn_hit_marker)
            .add_systems(
                Update,
                (reset_on_respawn, receive_hit_markers, update_hit_marker)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn spawn_hit_marker(mut commands: Commands, mut flash: ResMut<HitMarkerFlash>) {
    *flash = HitMarkerFlash::default();
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
            root.spawn((
                Node {
                    width: Val::Px(BOX_PX),
                    height: Val::Px(BOX_PX),
                    ..default()
                },
                // A "+" turned into an X.
                Transform::from_rotation(Quat::from_rotation_z(FRAC_PI_4)),
            ))
            .with_children(|x| {
                for dir in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
                    x.spawn((
                        HitMarkerArm { dir },
                        Node {
                            position_type: PositionType::Absolute,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ));
                }
            });
        });
}

/// A new life: no marker left flashing from the last one.
fn reset_on_respawn(
    mut respawned: EventReader<crate::net::LocalPlayerRespawned>,
    mut flash: ResMut<HitMarkerFlash>,
) {
    if respawned.read().count() > 0 {
        flash.kill = None;
    }
}

/// `HitMarker`s from the server: start a flash (a kill beats a hit landing
/// in the same frame, e.g. a collateral that killed one and hurt another),
/// with the hit sound for a hit. Ignored during a kill cam (it's the live
/// world's shot).
fn receive_hit_markers(
    mut receivers: Query<&mut MessageReceiver<shared::HitMarker>>,
    killcam: Res<ActiveKillCam>,
    sounds: Res<GameSounds>,
    mut flash: ResMut<HitMarkerFlash>,
    mut commands: Commands,
) {
    let mut got: Option<bool> = None;
    for mut rx in &mut receivers {
        for marker in rx.receive() {
            if killcam.0.is_none() {
                got = Some(got.unwrap_or(false) || marker.kill);
            }
        }
    }
    let Some(kill) = got else {
        return;
    };
    *flash = HitMarkerFlash {
        kill: Some(kill),
        age: 0.0,
    };
    if !kill {
        commands.spawn((
            AudioPlayer::new(sounds.hit_marker.clone()),
            PlaybackSettings::DESPAWN,
        ));
    }
}

/// Slide the arms out and fade them: quick ease-out spread, the fade
/// weighted to the end so the X reads at full strength first.
fn update_hit_marker(
    time: Res<Time>,
    mut flash: ResMut<HitMarkerFlash>,
    mut arms: Query<(&HitMarkerArm, &mut Node, &mut BackgroundColor)>,
) {
    let Some(kill) = flash.kill else {
        for (_, _, mut bg) in &mut arms {
            if bg.0 != Color::NONE {
                bg.0 = Color::NONE;
            }
        }
        return;
    };
    let style = if kill { &KILL } else { &HIT };
    flash.age += time.delta_secs();
    let t = (flash.age / style.secs).min(1.0);
    if t >= 1.0 {
        flash.kill = None;
    }
    let spread = 1.0 - (1.0 - t).powi(3);
    let gap = style.gap_from + (style.gap_to - style.gap_from) * spread;
    let alpha = 1.0 - t * t;

    let centre = BOX_PX * 0.5;
    for (arm, mut node, mut bg) in &mut arms {
        // The arm's middle, out along its axis.
        let mid = arm.dir * (gap + style.len * 0.5);
        let (w, h) = if arm.dir.x != 0.0 {
            (style.len, BAR_THICK_PX)
        } else {
            (BAR_THICK_PX, style.len)
        };
        node.left = Val::Px(centre + mid.x - w * 0.5);
        node.top = Val::Px(centre + mid.y - h * 0.5);
        node.width = Val::Px(w);
        node.height = Val::Px(h);
        bg.0 = style.color.with_alpha(alpha);
    }
}
