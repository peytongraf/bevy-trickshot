//! The top-left frames-per-second + ping readout, over the minimap — Modern
//! Warfare III's: a small dark box each, a dim label ("FPS:", "LATENCY:")
//! and the value in white.

use bevy::prelude::*;
use lightyear::prelude::{Connected, PingManager};

use crate::net::GameClient;
use crate::{menu, HUD_FONT};

/// How tall (logical px) the readout's boxes are, and how far down from the
/// top of the screen they sit — the minimap goes under them.
pub(crate) const FPS_ROW_TOP: f32 = 6.0;
pub(crate) const FPS_ROW_HEIGHT: f32 = 26.0;

/// The FPS value.
#[derive(Component)]
pub(crate) struct FpsText;

/// The ping value, in the box beside [`FpsText`]'s.
#[derive(Component)]
pub(crate) struct PingText;

/// Top-left frames-per-second + ping readout.
pub(crate) fn setup_fps_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = TextFont {
        font: asset_server.load(HUD_FONT),
        font_size: 20.0,
        ..default()
    };
    let stat = |row: &mut ChildSpawnerCommands, label: &str, value: Entity| {
        row.spawn((
            Node {
                height: Val::Px(FPS_ROW_HEIGHT),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
                align_items: AlignItems::Center,
                column_gap: Val::Px(5.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        ))
        .with_child((Text::new(label), font.clone(), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55))))
        .add_child(value);
    };
    let fps = commands.spawn((FpsText, Text::new(""), font.clone(), TextColor(Color::WHITE))).id();
    let ping = commands.spawn((PingText, Text::new(""), font.clone(), TextColor(Color::WHITE))).id();
    commands
        .spawn((
            menu::HudElement,
            GlobalZIndex(4),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(crate::minimap::MINIMAP_LEFT),
                top: Val::Px(FPS_ROW_TOP),
                column_gap: Val::Px(3.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            stat(row, "FPS:", fps);
            stat(row, "LATENCY:", ping);
        });
}

/// Accumulator for the FPS/ping readout: frames, seconds, and time-weighted
/// round-trip time (only over connected frames) since the last update.
#[derive(Default)]
pub(crate) struct FpsAccum {
    elapsed: f32,
    frames: u32,
    ping_secs: f32,
    ping_weighted_ms: f32,
}

/// Refresh the top-left readout twice a second with the average FPS and the
/// average ping (lightyear's RTT estimate, weighted by frame time) over each
/// 500 ms window. Ping shows `-- ms` while not connected / no pong yet.
pub(crate) fn update_fps_ui(
    time: Res<Time>,
    mut acc: Local<FpsAccum>,
    ping: Query<&PingManager, (With<GameClient>, With<Connected>)>,
    mut fps_text: Single<&mut Text, (With<FpsText>, Without<PingText>)>,
    mut ping_text: Single<&mut Text, (With<PingText>, Without<FpsText>)>,
) {
    let dt = time.delta_secs();
    acc.elapsed += dt;
    acc.frames += 1;
    if let Ok(pm) = ping.single() {
        // `pongs_recv == 0` means the estimator has no sample yet (rtt is 0).
        if pm.pongs_recv > 0 {
            acc.ping_secs += dt;
            acc.ping_weighted_ms += pm.rtt().as_secs_f32() * 1000.0 * dt;
        }
    }

    if acc.elapsed >= 0.5 {
        let fps = acc.frames as f32 / acc.elapsed;
        let wanted = format!("{fps:.0}");
        if fps_text.0 != wanted {
            fps_text.0 = wanted;
        }
        let wanted = if acc.ping_secs > 0.0 {
            format!("{:.0} ms", acc.ping_weighted_ms / acc.ping_secs)
        } else {
            "-- ms".to_string()
        };
        if ping_text.0 != wanted {
            ping_text.0 = wanted;
        }
        *acc = FpsAccum::default();
    }
}
