//! The top-left telemetry readout, over the minimap — Modern Warfare III's: a
//! small dark box per readout, a dim label ("FPS:", "LATENCY:") and the
//! value in white. Which readouts show is the player's call (settings →
//! INTERFACE → TELEMETRY, [`Telemetry`]): frames per second, 1% low, frame
//! time, server latency, and the game's CPU and memory use. With debug mode
//! on (settings → DEBUG), two more can show after them, switched on and off
//! there: DRAW CALLS (counted in the render world each frame,
//! [`DrawCallsPlugin`]) and ENTITIES (in the main world).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore, SystemInformationDiagnosticsPlugin};
use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d, Transmissive3d, Transparent3d};
use bevy::core_pipeline::prepass::{AlphaMask3dPrepass, Opaque3dPrepass};
use bevy::ecs::entity::Entities;
use bevy::pbr::Shadow;
use bevy::prelude::*;
use bevy::render::render_phase::{
    BinnedPhaseItem, SortedPhaseItem, ViewBinnedRenderPhases, ViewSortedRenderPhases,
};
use bevy::render::{Render, RenderApp, RenderSet};
use bevy::ui::TransparentUi;
use lightyear::prelude::{Connected, PingManager};

use crate::net::GameClient;
use crate::settings::{Settings, TelemetryStat};
use crate::{menu, HUD_FONT};

/// How tall (logical px) the readout's boxes are, and how far down from the
/// top of the screen they sit — the minimap goes under them.
pub(crate) const FPS_ROW_TOP: f32 = 6.0;
pub(crate) const FPS_ROW_HEIGHT: f32 = 26.0;
/// How often (s) the values refresh.
const REFRESH_SECS: f32 = 0.5;
/// How far back (s) the 1% low looks.
const LOW_WINDOW_SECS: f32 = 10.0;

/// One readout on the row: one the player picks, or a debug-mode one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Readout {
    Stat(TelemetryStat),
    DrawCalls,
    Entities,
}

impl Readout {
    fn label(self) -> &'static str {
        match self {
            Readout::Stat(stat) => stat.label(),
            Readout::DrawCalls => "DRAW CALLS:",
            Readout::Entities => "ENTITIES:",
        }
    }

    fn shown(self, settings: &Settings) -> bool {
        match self {
            Readout::Stat(stat) => settings.telemetry.shows(stat),
            Readout::DrawCalls => settings.debug_mode && settings.telemetry.draw_calls,
            Readout::Entities => settings.debug_mode && settings.telemetry.entities,
        }
    }

    fn all() -> impl Iterator<Item = Readout> {
        TelemetryStat::ALL
            .into_iter()
            .map(Readout::Stat)
            .chain([Readout::DrawCalls, Readout::Entities])
    }
}

/// One readout's box (hidden while it's turned off).
#[derive(Component)]
pub(crate) struct TelemetryBox(Readout);

/// One readout's value.
#[derive(Component)]
pub(crate) struct TelemetryValue(Readout);

/// The top-left telemetry row: a box for every readout, the ones turned off
/// hidden.
pub(crate) fn setup_fps_ui(mut commands: Commands, asset_server: Res<AssetServer>, settings: Res<Settings>) {
    let font = TextFont {
        font: asset_server.load(HUD_FONT),
        font_size: 20.0,
        ..default()
    };
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
            for readout in Readout::all() {
                row.spawn((
                    TelemetryBox(readout),
                    Node {
                        display: box_display(&settings, readout),
                        height: Val::Px(FPS_ROW_HEIGHT),
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(5.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                ))
                .with_children(|b| {
                    b.spawn((Text::new(readout.label()), font.clone(), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55))));
                    b.spawn((TelemetryValue(readout), Text::new("--"), font.clone(), TextColor(Color::WHITE)));
                });
            }
        });
}

fn box_display(settings: &Settings, readout: Readout) -> Display {
    if readout.shown(settings) {
        Display::Flex
    } else {
        Display::None
    }
}

/// What the readouts are worked out from: frame times (the last
/// [`LOW_WINDOW_SECS`] of them, for the 1% low) and time-weighted round-trip
/// time (only over connected frames) since the last refresh.
#[derive(Default)]
pub(crate) struct FpsAccum {
    elapsed: f32,
    frames: u32,
    ping_secs: f32,
    ping_weighted_ms: f32,
    /// `(Time::elapsed_secs, frame time)` per frame.
    recent: VecDeque<(f32, f32)>,
}

/// The average frame rate of the slowest 1% of `frame_times` (at least one).
fn one_percent_low(frame_times: impl Iterator<Item = f32>) -> Option<f32> {
    let mut times: Vec<f32> = frame_times.filter(|t| *t > 0.0).collect();
    if times.is_empty() {
        return None;
    }
    times.sort_by(|a, b| b.total_cmp(a));
    let worst = &times[..times.len().div_ceil(100)];
    Some(worst.len() as f32 / worst.iter().sum::<f32>())
}

/// Show the readouts turned on, and refresh their values every
/// [`REFRESH_SECS`]: the average FPS and frame time and the average ping
/// (lightyear's RTT estimate, weighted by frame time) over the window, the 1%
/// low over the last [`LOW_WINDOW_SECS`], and the game's CPU and memory use.
/// Ping shows `-- MS` while not connected / no pong yet.
pub(crate) fn update_fps_ui(
    time: Res<Time>,
    settings: Res<Settings>,
    diagnostics: Res<DiagnosticsStore>,
    mut acc: Local<FpsAccum>,
    ping: Query<&PingManager, (With<GameClient>, With<Connected>)>,
    draw_calls: Res<DrawCalls>,
    entities: &Entities,
    mut boxes: Query<(&TelemetryBox, &mut Node)>,
    mut values: Query<(&TelemetryValue, &mut Text)>,
) {
    if settings.is_changed() {
        for (b, mut node) in &mut boxes {
            let display = box_display(&settings, b.0);
            if node.display != display {
                node.display = display;
            }
        }
    }

    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    acc.elapsed += dt;
    acc.frames += 1;
    acc.recent.push_back((now, dt));
    while acc.recent.front().is_some_and(|&(at, _)| now - at > LOW_WINDOW_SECS) {
        acc.recent.pop_front();
    }
    if let Ok(pm) = ping.single() {
        // `pongs_recv == 0` means the estimator has no sample yet (rtt is 0).
        if pm.pongs_recv > 0 {
            acc.ping_secs += dt;
            acc.ping_weighted_ms += pm.rtt().as_secs_f32() * 1000.0 * dt;
        }
    }
    if acc.elapsed < REFRESH_SECS {
        return;
    }

    let fps = acc.frames as f32 / acc.elapsed;
    let process = |path: &DiagnosticPath| diagnostics.get(path).and_then(|d| d.smoothed());
    for (value, mut text) in &mut values {
        if !value.0.shown(&settings) {
            continue;
        }
        let Readout::Stat(stat) = value.0 else {
            let wanted = match value.0 {
                Readout::DrawCalls => draw_calls.0.load(Ordering::Relaxed).to_string(),
                _ => entities.len().to_string(),
            };
            if text.0 != wanted {
                text.0 = wanted;
            }
            continue;
        };
        let wanted = match stat {
            TelemetryStat::Fps => format!("{fps:.0}"),
            TelemetryStat::OnePercentLow => {
                one_percent_low(acc.recent.iter().map(|&(_, t)| t)).map_or("--".into(), |low| format!("{low:.0}"))
            }
            TelemetryStat::FrameTime => format!("{:.1} MS", 1000.0 / fps.max(0.001)),
            TelemetryStat::Latency if acc.ping_secs > 0.0 => {
                format!("{:.0} MS", acc.ping_weighted_ms / acc.ping_secs)
            }
            TelemetryStat::Latency => "-- MS".into(),
            TelemetryStat::Cpu => process(&SystemInformationDiagnosticsPlugin::PROCESS_CPU_USAGE)
                .map_or("--".into(), |cpu| format!("{cpu:.0}%")),
            TelemetryStat::Ram => process(&SystemInformationDiagnosticsPlugin::PROCESS_MEM_USAGE)
                .map_or("--".into(), |gib| format!("{gib:.1} GB")),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
    let recent = std::mem::take(&mut acc.recent);
    *acc = FpsAccum { recent, ..default() };
}

// --- draw calls ------------------------------------------------------------------

/// How many draw calls the last rendered frame made — counted in the render
/// world ([`count_draw_calls`]), read on the HUD. Shared by both worlds.
#[derive(Resource, Clone, Default)]
pub(crate) struct DrawCalls(Arc<AtomicUsize>);

/// Counts the draw calls each frame makes, for debug mode's DRAW CALLS.
pub(crate) struct DrawCallsPlugin;

impl Plugin for DrawCallsPlugin {
    fn build(&self, app: &mut App) {
        let count = DrawCalls::default();
        app.insert_resource(count.clone());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(count)
                .add_systems(Render, count_draw_calls.in_set(RenderSet::Render));
        }
    }
}

/// The draws a binned pass (opaque, alpha-masked, shadows, prepass) makes:
/// one per batch set of multidrawn meshes, one per bin of batched ones, one
/// per unbatchable mesh and per non-mesh item.
fn binned<B: BinnedPhaseItem>(phases: Option<Res<ViewBinnedRenderPhases<B>>>) -> usize {
    phases.map_or(0, |phases| {
        phases
            .0
            .values()
            .map(|phase| {
                phase.multidrawable_meshes.len()
                    + phase.batchable_meshes.len()
                    + phase.unbatchable_meshes.values().map(|u| u.entities.len()).sum::<usize>()
                    + phase.non_mesh_items.values().map(|n| n.entities.len()).sum::<usize>()
            })
            .sum()
    })
}

/// The draws a sorted pass (transparent, the UI) makes: one per item left
/// with something to draw once batched.
fn sorted<S: SortedPhaseItem>(phases: Option<Res<ViewSortedRenderPhases<S>>>) -> usize {
    phases.map_or(0, |phases| {
        phases
            .0
            .values()
            .map(|phase| phase.items.iter().filter(|item| !item.batch_range().is_empty()).count())
            .sum()
    })
}

/// Add up this frame's draws, every view and pass. (egui's own pass isn't
/// in it.)
#[allow(clippy::too_many_arguments)]
fn count_draw_calls(
    count: Res<DrawCalls>,
    opaque: Option<Res<ViewBinnedRenderPhases<Opaque3d>>>,
    alpha_mask: Option<Res<ViewBinnedRenderPhases<AlphaMask3d>>>,
    opaque_prepass: Option<Res<ViewBinnedRenderPhases<Opaque3dPrepass>>>,
    alpha_mask_prepass: Option<Res<ViewBinnedRenderPhases<AlphaMask3dPrepass>>>,
    shadows: Option<Res<ViewBinnedRenderPhases<Shadow>>>,
    transmissive: Option<Res<ViewSortedRenderPhases<Transmissive3d>>>,
    transparent: Option<Res<ViewSortedRenderPhases<Transparent3d>>>,
    ui: Option<Res<ViewSortedRenderPhases<TransparentUi>>>,
) {
    let total = binned(opaque)
        + binned(alpha_mask)
        + binned(opaque_prepass)
        + binned(alpha_mask_prepass)
        + binned(shadows)
        + sorted(transmissive)
        + sorted(transparent)
        + sorted(ui);
    count.0.store(total, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_percent_low_averages_the_slowest_frames() {
        // 198 frames at 100 FPS and two at 20 FPS: the slowest 1% is those two.
        let times = std::iter::repeat_n(0.01, 198).chain([0.05, 0.05]);
        assert!((one_percent_low(times).unwrap() - 20.0).abs() < 1e-3);
        assert_eq!(one_percent_low(std::iter::empty()), None);
    }
}
