//! The top-left frames-per-second + ping readout.

use bevy::prelude::*;
use lightyear::prelude::{Connected, PingManager};

use crate::net::GameClient;
use crate::{menu, HUD_FONT};

/// The top-left FPS readout.
#[derive(Component)]
pub(crate) struct FpsText;

/// The ping readout, beside [`FpsText`] in the same pill.
#[derive(Component)]
pub(crate) struct PingText;

/// Top-left frames-per-second + ping readout.
pub(crate) fn setup_fps_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = TextFont {
        font: asset_server.load(HUD_FONT),
        font_size: 22.0,
        ..default()
    };
    commands
        .spawn((
            menu::HudElement,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(18.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                column_gap: Val::Px(16.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_children(|pill| {
            pill.spawn((FpsText, Text::new(""), font.clone(), TextColor(Color::WHITE)));
            pill.spawn((PingText, Text::new(""), font, TextColor(Color::WHITE)));
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
        let wanted = format!("{fps:.0} fps");
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
