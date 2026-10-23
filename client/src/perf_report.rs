//! Debug mode's performance recorder: every frame of a game, start to
//! finish, so a test run can be judged as a whole instead of by glancing at
//! the live FPS readout — and judged against the last run like it.
//!
//! While debug mode's on, each frame's real duration (`Time<Real>` — the
//! virtual clock caps a frame at 250 ms, which would hide the worst hitches)
//! is kept along with the round, how many enemies were on the map and
//! whether an exfil was on; once a second the timeline also notes entities,
//! draw calls, memory, CPU and ping. Not while the party's loading in (nor
//! for [`WARMUP_SECS`] after), nor while the leader has the game paused.
//!
//! When the results screen opens the run is summed up — the whole game, the
//! exfil on its own, by how many enemies were on the map (so two runs that
//! drew different hordes still compare fairly) and the 1% low of every round
//! (does it sink as the game goes on?) — and saved to
//! `<config dir>/bevy-trickshot/perf/` as `<unix time>_<map>_<mode>.json`
//! (the report) and `.csv` (the per-second timeline). The results screen's
//! PERFORMANCE button (debug mode only) shows it, each number beside how it
//! moved since the last saved run on the same map and mode from the same
//! starting round. A game left early is saved too, just not shown.
//!
//! Nothing's recorded with debug mode off — playing normally never collects
//! frames or writes files. Every game starts with a fresh recorder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore, SystemInformationDiagnosticsPlugin};
use bevy::ecs::entity::Entities;
use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use lightyear::prelude::{Connected, LocalId, PingManager};
use serde::{Deserialize, Serialize};

use crate::hud::DrawCalls;
use crate::menu::{Menu, Screen};
use crate::net::{GameClient, RemoteAvatar};
use crate::settings::Settings;
use crate::ui::TEXT;
use crate::{AppState, BODY_FONT, HUD_FONT};

/// Seconds after the party's loaded in before frames count — the first
/// frames of a game stall on pipelines and uploads that aren't the game's
/// steady state.
const WARMUP_SECS: f32 = 2.0;
/// Most frames kept (~48 MB) — hours of play at a high frame rate.
const MAX_FRAMES: usize = 4_000_000;
/// "Under 60 FPS" is a frame slower than this (s) — a hair of slack, so a
/// game held at exactly 60 doesn't count every frame of jitter.
const UNDER_60: f32 = 1.0 / 59.0;
const UNDER_100: f32 = 1.0 / 99.0;
/// A hitch: a frame slower than this (s), under 30 FPS.
const HITCH: f32 = 1.0 / 30.0;
/// Enemy-count buckets: `0..10`, `10..20`, `20..30`, `30+`.
const BUCKET: u16 = 10;
const BUCKETS: u16 = 4;
/// A bucket needs this many seconds in it to be worth showing.
const MIN_BUCKET_SECS: f32 = 5.0;
/// A change under this (%) is shown as no change.
const NOISE_PCT: f32 = 2.0;

pub(crate) struct PerfReportPlugin;

impl Plugin for PerfReportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PerfRecorder>()
            .init_resource::<PerfPanel>()
            .add_systems(OnEnter(AppState::InGame), reset)
            .add_systems(OnExit(AppState::InGame), (save_unfinished, reset).chain())
            .add_systems(
                Update,
                (record_frame, finish_on_results, close_panel, sync_panel, hover_close)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

// --- recording ---------------------------------------------------------------

/// One recorded frame.
#[derive(Clone, Copy)]
struct FrameRec {
    /// Its real duration (s).
    dt: f32,
    /// The `Zombies` round (0 in other modes).
    round: u16,
    /// Enemies on the map.
    enemies: u16,
    exfil: bool,
}

/// One second of the timeline.
struct Sample {
    /// Seconds recorded so far.
    at: f32,
    fps: f32,
    worst_ms: f32,
    /// The most enemies on the map during it.
    enemies: u16,
    entities: u32,
    draw_calls: u32,
    mem_gb: Option<f32>,
    cpu_pct: Option<f32>,
    ping_ms: Option<f32>,
    round: u16,
    exfil: bool,
}

/// What a game is, for matching it against earlier runs.
#[derive(Clone)]
struct Meta {
    map: String,
    map_label: String,
    mode: String,
    start_round: u32,
    full_power: bool,
}

/// The second being added up for the timeline.
#[derive(Default)]
struct SecondAcc {
    secs: f32,
    frames: u32,
    worst: f32,
    enemies: u16,
}

/// This game's recording. Reset on entering and leaving a game.
#[derive(Resource, Default)]
pub(crate) struct PerfRecorder {
    meta: Option<Meta>,
    /// Seconds since the party loaded in, for the warm-up.
    since_loaded: f32,
    frames: Vec<FrameRec>,
    timeline: Vec<Sample>,
    second: SecondAcc,
    /// `(round, entities, memory GB)` the moment each round started.
    round_starts: Vec<(u16, u32, Option<f32>)>,
    last_round: Option<u16>,
    recorded_secs: f32,
    /// Summed up and saved: nothing more is recorded this game.
    done: bool,
}

impl PerfRecorder {
    /// Seconds recorded so far, while a game's being recorded.
    pub(crate) fn recording(&self) -> Option<f32> {
        (self.meta.is_some() && !self.done).then_some(self.recorded_secs)
    }
}

fn reset(mut rec: ResMut<PerfRecorder>, mut panel: ResMut<PerfPanel>) {
    *rec = PerfRecorder::default();
    *panel = PerfPanel::default();
}

fn diagnostic(diagnostics: &DiagnosticsStore, path: &DiagnosticPath) -> Option<f32> {
    diagnostics.get(path).and_then(|d| d.smoothed()).map(|v| v as f32)
}

/// Note this frame (debug mode only), and every second a timeline sample.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn record_frame(
    time: Res<Time<Real>>,
    settings: Res<Settings>,
    menu: Res<Menu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    enemies: Query<
        (),
        (
            With<RemoteAvatar>,
            Or<(With<crate::ZombieVisual>, With<crate::dogs::DogVisual>, With<crate::boss::BossVisual>)>,
        ),
    >,
    avatars: Query<(), (With<RemoteAvatar>, Without<crate::killcam::KillCamPlayerGhost>)>,
    (entities, draw_calls, diagnostics): (&Entities, Res<DrawCalls>, Res<DiagnosticsStore>),
    ping: Query<&PingManager, (With<GameClient>, With<Connected>)>,
    mut rec: ResMut<PerfRecorder>,
) {
    if !settings.debug_mode || rec.done || matches!(menu.screen, Screen::LoadingGame | Screen::MatchResults) {
        return;
    }
    let me = local.iter().next().map(|l| l.0);
    let Some(lobby) = me.and_then(|me| lobbies.iter().find(|l| l.has(me) && l.started)) else {
        return;
    };
    let dt = time.delta_secs();
    if lobby.paused || dt <= 0.0 {
        return;
    }
    if rec.since_loaded < WARMUP_SECS {
        rec.since_loaded += dt;
        return;
    }
    let zombies = lobby.mode == shared::GameMode::Zombies;
    if rec.meta.is_none() {
        rec.meta = Some(Meta {
            map: format!("{:?}", lobby.map),
            map_label: lobby.map.label().to_string(),
            mode: lobby.mode.label().to_string(),
            start_round: if zombies { lobby.start_round.max(1) } else { 0 },
            full_power: zombies && lobby.full_power,
        });
    }
    let round = if zombies { lobby.round.min(u16::MAX as u32) as u16 } else { 0 };
    // (Other players don't count in `Zombies`; everyone else is the enemy.)
    let on_map = if zombies { enemies.iter().count() } else { avatars.iter().count() };
    let on_map = on_map.min(u16::MAX as usize) as u16;
    let exfil = zombies && lobby.exfil.running();
    let mem = diagnostic(&diagnostics, &SystemInformationDiagnosticsPlugin::PROCESS_MEM_USAGE);

    if rec.last_round != Some(round) {
        rec.last_round = Some(round);
        if zombies {
            rec.round_starts.push((round, entities.len(), mem));
        }
    }
    if rec.frames.len() < MAX_FRAMES {
        rec.frames.push(FrameRec { dt, round, enemies: on_map, exfil });
    }
    rec.recorded_secs += dt;
    let s = &mut rec.second;
    s.secs += dt;
    s.frames += 1;
    s.worst = s.worst.max(dt);
    s.enemies = s.enemies.max(on_map);
    if s.secs < 1.0 {
        return;
    }
    let s = std::mem::take(&mut rec.second);
    let ping_ms = ping
        .single()
        .ok()
        .filter(|pm| pm.pongs_recv > 0)
        .map(|pm| pm.rtt().as_secs_f32() * 1000.0);
    let sample = Sample {
        at: rec.recorded_secs,
        fps: s.frames as f32 / s.secs,
        worst_ms: s.worst * 1000.0,
        enemies: s.enemies,
        entities: entities.len(),
        draw_calls: draw_calls.get().min(u32::MAX as usize) as u32,
        mem_gb: mem,
        cpu_pct: diagnostic(&diagnostics, &SystemInformationDiagnosticsPlugin::PROCESS_CPU_USAGE),
        ping_ms,
        round,
        exfil,
    };
    rec.timeline.push(sample);
}

// --- the report ----------------------------------------------------------------

/// How a stretch of frames went.
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(default)]
pub(crate) struct Stats {
    secs: f32,
    frames: u32,
    avg_fps: f32,
    /// Average FPS of the slowest 1% / 0.1% of frames.
    low_1: f32,
    low_01: f32,
    /// Frame time (ms) at the median and the 99th percentile, and the worst.
    p50_ms: f32,
    p99_ms: f32,
    worst_ms: f32,
    /// Share (%) of the time spent in frames under 60 / 100 FPS.
    under_60_pct: f32,
    under_100_pct: f32,
    /// Frames under 30 FPS, per minute.
    hitches_per_min: f32,
}

impl Stats {
    fn of(mut dts: Vec<f32>) -> Stats {
        dts.retain(|t| *t > 0.0);
        let n = dts.len();
        if n == 0 {
            return Stats::default();
        }
        let secs: f64 = dts.iter().map(|&t| t as f64).sum();
        let time_over = |limit: f32| dts.iter().filter(|&&t| t > limit).map(|&t| t as f64).sum::<f64>();
        let under_60 = time_over(UNDER_60);
        let under_100 = time_over(UNDER_100);
        let hitches = dts.iter().filter(|&&t| t > HITCH).count();
        // Slowest first.
        dts.sort_by(|a, b| b.total_cmp(a));
        let low = |share: f64| {
            let k = ((n as f64 * share).ceil() as usize).clamp(1, n);
            (k as f64 / dts[..k].iter().map(|&t| t as f64).sum::<f64>()) as f32
        };
        let slowest = |share: f64| dts[((n as f64 * share) as usize).min(n - 1)] * 1000.0;
        Stats {
            secs: secs as f32,
            frames: n as u32,
            avg_fps: (n as f64 / secs) as f32,
            low_1: low(0.01),
            low_01: low(0.001),
            p50_ms: slowest(0.5),
            p99_ms: slowest(0.01),
            worst_ms: dts[0] * 1000.0,
            under_60_pct: (under_60 / secs * 100.0) as f32,
            under_100_pct: (under_100 / secs * 100.0) as f32,
            hitches_per_min: (hitches as f64 / (secs / 60.0)) as f32,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct RoundStats {
    round: u32,
    stats: Stats,
    entities_at_start: Option<u32>,
    mem_gb_at_start: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct BucketStats {
    /// Enemies on the map: `from..from + 10` (the last, `from` and up).
    from: u32,
    stats: Stats,
}

impl BucketStats {
    fn label(&self) -> String {
        if self.from >= ((BUCKETS - 1) * BUCKET) as u32 {
            format!("{}+ ENEMIES", self.from)
        } else {
            format!("{}–{} ENEMIES", self.from, self.from + BUCKET as u32 - 1)
        }
    }
}

/// A saved run.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub(crate) struct PerfReport {
    unix_secs: u64,
    version: String,
    map: String,
    map_label: String,
    mode: String,
    /// The round a `Zombies` game started on (0 in other modes).
    start_round: u32,
    full_power: bool,
    end_round: u32,
    /// Left before the results screen.
    ended_early: bool,
    resolution: String,
    vsync: bool,
    frame_limit: f32,
    shadow_quality: String,
    overall: Stats,
    exfil: Option<Stats>,
    by_enemies: Vec<BucketStats>,
    by_round: Vec<RoundStats>,
    peak_entities: u32,
    peak_mem_gb: Option<f32>,
}

impl PerfReport {
    /// The settings that change the numbers, differing from `other`'s.
    fn setting_changes(&self, other: &PerfReport) -> Vec<&'static str> {
        let mut changed = Vec::new();
        if self.resolution != other.resolution {
            changed.push("RESOLUTION");
        }
        if self.shadow_quality != other.shadow_quality {
            changed.push("SHADOWS");
        }
        if self.vsync != other.vsync || self.frame_limit != other.frame_limit {
            changed.push("FPS CAP");
        }
        changed
    }
}

/// What's needed of the outside world to sum a run up.
struct Env {
    resolution: String,
    vsync: bool,
    frame_limit: f32,
    shadow_quality: String,
}

fn build_report(rec: &PerfRecorder, meta: &Meta, env: Env, ended_early: bool) -> PerfReport {
    let all: Vec<f32> = rec.frames.iter().map(|f| f.dt).collect();
    let exfil: Vec<f32> = rec.frames.iter().filter(|f| f.exfil).map(|f| f.dt).collect();
    let mut rounds: BTreeMap<u16, Vec<f32>> = BTreeMap::new();
    let mut buckets: BTreeMap<u16, Vec<f32>> = BTreeMap::new();
    for f in &rec.frames {
        if f.round > 0 {
            rounds.entry(f.round).or_default().push(f.dt);
        }
        buckets.entry((f.enemies / BUCKET).min(BUCKETS - 1)).or_default().push(f.dt);
    }
    let by_round = rounds
        .into_iter()
        .map(|(round, dts)| {
            let start = rec.round_starts.iter().find(|(r, ..)| *r == round);
            RoundStats {
                round: round as u32,
                stats: Stats::of(dts),
                entities_at_start: start.map(|s| s.1),
                mem_gb_at_start: start.and_then(|s| s.2),
            }
        })
        .collect();
    let by_enemies = buckets
        .into_iter()
        .map(|(b, dts)| BucketStats {
            from: (b * BUCKET) as u32,
            stats: Stats::of(dts),
        })
        .filter(|b| b.stats.secs >= MIN_BUCKET_SECS)
        .collect();
    PerfReport {
        unix_secs: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        version: env!("CARGO_PKG_VERSION").to_string(),
        map: meta.map.clone(),
        map_label: meta.map_label.clone(),
        mode: meta.mode.clone(),
        start_round: meta.start_round,
        full_power: meta.full_power,
        end_round: rec.last_round.unwrap_or(0) as u32,
        ended_early,
        resolution: env.resolution,
        vsync: env.vsync,
        frame_limit: env.frame_limit,
        shadow_quality: env.shadow_quality,
        overall: Stats::of(all),
        exfil: (exfil.iter().sum::<f32>() >= 1.0).then(|| Stats::of(exfil)),
        by_enemies,
        by_round,
        peak_entities: rec.timeline.iter().map(|s| s.entities).max().unwrap_or(0),
        peak_mem_gb: rec.timeline.iter().filter_map(|s| s.mem_gb).reduce(f32::max),
    }
}

fn perf_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("bevy-trickshot").join("perf"))
}

/// The newest finished run before `report` on the same map and mode, from
/// the same start — with the same settings if there is one (else the newest,
/// flagged as different on the panel).
fn previous_run(dir: &Path, report: &PerfReport) -> Option<PerfReport> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| serde_json::from_str::<PerfReport>(&std::fs::read_to_string(p).ok()?).ok())
        .filter(|r| {
            !r.ended_early
                && r.unix_secs < report.unix_secs
                && r.map == report.map
                && r.mode == report.mode
                && r.start_round == report.start_round
                && r.full_power == report.full_power
        })
        .max_by_key(|r| (r.setting_changes(report).is_empty(), r.unix_secs))
}

fn opt(v: Option<f32>, digits: usize) -> String {
    v.map_or(String::new(), |v| format!("{v:.digits$}"))
}

/// Write the report and its timeline; where the report went.
fn save(dir: &Path, report: &PerfReport, timeline: &[Sample]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mode = report.mode.to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    let stem = format!("{}_{}_{}", report.unix_secs, report.map.to_lowercase(), mode);
    let json = dir.join(format!("{stem}.json"));
    std::fs::write(&json, serde_json::to_string_pretty(report).map_err(std::io::Error::other)?)?;
    let mut csv = String::from("secs,fps,worst_ms,enemies,entities,draw_calls,mem_gb,cpu_pct,ping_ms,round,exfil\n");
    for s in timeline {
        csv.push_str(&format!(
            "{:.0},{:.1},{:.1},{},{},{},{},{},{},{},{}\n",
            s.at,
            s.fps,
            s.worst_ms,
            s.enemies,
            s.entities,
            s.draw_calls,
            opt(s.mem_gb, 3),
            opt(s.cpu_pct, 0),
            opt(s.ping_ms, 0),
            s.round,
            s.exfil as u8,
        ));
    }
    std::fs::write(dir.join(format!("{stem}.csv")), csv)?;
    Ok(json)
}

/// `path`, with the home directory written `~`.
fn display_path(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn env(settings: &Settings, window: Option<&Window>) -> Env {
    Env {
        resolution: window.map_or(String::new(), |w| format!("{}x{}", w.physical_width(), w.physical_height())),
        vsync: settings.vsync,
        frame_limit: settings.frame_limit,
        shadow_quality: settings.shadow_quality.label().to_string(),
    }
}

/// Sum the run up, save it and (unless it was left early) find what to
/// compare it with.
fn finish(rec: &mut PerfRecorder, env: Env, ended_early: bool) -> Option<Finished> {
    rec.done = true;
    let meta = rec.meta.clone()?;
    if rec.frames.is_empty() {
        return None;
    }
    let report = build_report(rec, &meta, env, ended_early);
    let dir = perf_dir();
    let saved = dir.as_deref().map(|dir| save(dir, &report, &rec.timeline));
    let saved = match saved {
        Some(Ok(path)) => {
            info!("perf report saved to {}", path.display());
            Some(display_path(&path))
        }
        Some(Err(e)) => {
            warn!("couldn't save the perf report: {e}");
            None
        }
        None => None,
    };
    let previous = if ended_early { None } else { dir.as_deref().and_then(|d| previous_run(d, &report)) };
    // (The frames are summed up; free them now rather than with the game.)
    rec.frames = Vec::new();
    Some(Finished { report, previous, saved })
}

/// The results screen's up: sum the game up for the panel.
fn finish_on_results(
    menu: Res<Menu>,
    settings: Res<Settings>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut rec: ResMut<PerfRecorder>,
    mut panel: ResMut<PerfPanel>,
) {
    if menu.screen != Screen::MatchResults || rec.done {
        return;
    }
    panel.finished = finish(&mut rec, env(&settings, window.single().ok()), false);
}

/// Left mid-game: still save what was recorded.
fn save_unfinished(
    settings: Res<Settings>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut rec: ResMut<PerfRecorder>,
) {
    if !rec.done {
        finish(&mut rec, env(&settings, window.single().ok()), true);
    }
}

// --- the panel -------------------------------------------------------------------

struct Finished {
    report: PerfReport,
    previous: Option<PerfReport>,
    /// Where the report was saved, if it could be.
    saved: Option<String>,
}

/// The results screen's PERFORMANCE panel. Reset on entering and leaving a
/// game.
#[derive(Resource, Default)]
pub(crate) struct PerfPanel {
    pub(crate) open: bool,
    finished: Option<Finished>,
}

#[derive(Component)]
struct PerfPanelRoot;

#[derive(Component)]
struct PerfClose;

const SHADE: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
const PANEL: Color = Color::srgba(0.07, 0.07, 0.08, 0.95);
const HEADER: Color = Color::srgb(0.6, 0.04, 0.05);
const SECTION: Color = Color::srgba(1.0, 1.0, 1.0, 0.04);
const LINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);
const DIM: Color = Color::srgba(1.0, 1.0, 1.0, 0.5);
const GOLD: Color = Color::srgb(0.98, 0.79, 0.24);
const GOOD: Color = Color::srgb(0.38, 0.86, 0.47);
const BAD: Color = Color::srgb(0.95, 0.32, 0.27);
const WARN: Color = Color::srgb(0.98, 0.66, 0.2);
const CLOSE_BG: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);
const CLOSE_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.18);
/// The round chart's height (px).
const CHART_H: f32 = 120.0;

/// Esc or the mouse back button closes it (the results screen itself
/// ignores both — only its CONTINUE leaves), as does its CLOSE button.
fn close_panel(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    close: Query<&Interaction, (With<PerfClose>, Changed<Interaction>)>,
    mut panel: ResMut<PerfPanel>,
) {
    if !panel.open {
        return;
    }
    let clicked = close.iter().any(|i| *i == Interaction::Pressed);
    if clicked || keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Back) {
        panel.open = false;
    }
}

fn hover_close(mut buttons: Query<(&Interaction, &mut BackgroundColor), (With<PerfClose>, Changed<Interaction>)>) {
    for (interaction, mut bg) in &mut buttons {
        bg.0 = if *interaction == Interaction::None { CLOSE_BG } else { CLOSE_HOVER };
    }
}

/// Show the panel while it's open over the results screen in debug mode.
fn sync_panel(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<Settings>,
    menu: Res<Menu>,
    mut panel: ResMut<PerfPanel>,
    existing: Query<Entity, With<PerfPanelRoot>>,
) {
    if panel.open && (menu.screen != Screen::MatchResults || !settings.debug_mode) {
        panel.open = false;
    }
    let shown = !existing.is_empty();
    if shown == panel.open {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    if panel.open {
        spawn_panel(&mut commands, &asset_server, panel.finished.as_ref());
    }
}

fn hud_text(asset_server: &AssetServer, text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: asset_server.load(HUD_FONT),
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

fn body_text(asset_server: &AssetServer, text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: asset_server.load(BODY_FONT),
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

/// Which way a number is better.
#[derive(Clone, Copy)]
enum Better {
    Higher,
    Lower,
}

/// How a change is written.
#[derive(Clone, Copy)]
enum Change {
    /// As a percentage of the old value.
    Percent,
    /// As the difference, in percentage points.
    Points,
    /// As the plain difference.
    Plain,
}

/// The change from `prev` to `now`, written, in green (better), red (worse)
/// or dim (about the same).
fn delta(now: f32, prev: Option<f32>, better: Better, change: Change) -> Option<(String, Color)> {
    let prev = prev?;
    let (text, amount, noise) = match change {
        Change::Percent => {
            if prev <= 0.0 {
                return None;
            }
            let pct = (now - prev) / prev * 100.0;
            (format!("{pct:+.0}%"), pct, NOISE_PCT)
        }
        Change::Points => {
            let d = now - prev;
            (format!("{d:+.1} PTS"), d, 0.5)
        }
        Change::Plain => {
            let d = now - prev;
            (format!("{d:+.1}"), d, 0.2)
        }
    };
    let color = if amount.abs() < noise {
        DIM
    } else if matches!(better, Better::Higher) == (amount > 0.0) {
        GOOD
    } else {
        BAD
    };
    Some((if amount.abs() < noise { "±0".into() } else { text }, color))
}

/// A metric's line: its name, its value in gold, and its change.
fn metric_row(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    label: &str,
    value: String,
    change: Option<(String, Color)>,
) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(30.0),
                align_items: AlignItems::Center,
                border: UiRect::bottom(Val::Px(1.0)),
                ..default()
            },
            BorderColor(LINE),
        ))
        .with_children(|row| {
            row.spawn(Node {
                flex_grow: 1.0,
                ..default()
            })
            .with_child(hud_text(asset_server, label, 19.0, TEXT));
            row.spawn(Node {
                width: Val::Px(96.0),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_child(hud_text(asset_server, value, 23.0, GOLD));
            let (text, color) = change.unwrap_or_else(|| ("".into(), DIM));
            row.spawn(Node {
                width: Val::Px(84.0),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_child(hud_text(asset_server, text, 18.0, color));
        });
}

/// A titled block of metric rows.
fn section(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    title: &str,
    note: &str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::new(Val::Px(14.0), Val::Px(14.0), Val::Px(10.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(SECTION),
            BorderRadius::all(Val::Px(4.0)),
        ))
        .with_children(|s| {
            s.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Baseline,
                margin: UiRect::bottom(Val::Px(4.0)),
                ..default()
            })
            .with_children(|head| {
                head.spawn(hud_text(asset_server, title, 22.0, crate::ui::ACCENT));
                head.spawn(body_text(asset_server, note, 11.0, DIM));
            });
            body(s);
        });
}

/// The rows every `Stats` block has (or, `short`, just the headline ones).
fn stats_rows(
    s: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    now: &Stats,
    prev: Option<&Stats>,
    short: bool,
) {
    let p = |f: fn(&Stats) -> f32| prev.map(f);
    metric_row(s, asset_server, "AVERAGE FPS", format!("{:.0}", now.avg_fps), delta(now.avg_fps, p(|s| s.avg_fps), Better::Higher, Change::Percent));
    metric_row(s, asset_server, "1% LOW", format!("{:.0}", now.low_1), delta(now.low_1, p(|s| s.low_1), Better::Higher, Change::Percent));
    metric_row(s, asset_server, "0.1% LOW", format!("{:.0}", now.low_01), delta(now.low_01, p(|s| s.low_01), Better::Higher, Change::Percent));
    if !short {
        metric_row(s, asset_server, "MEDIAN FRAME", format!("{:.1} MS", now.p50_ms), delta(now.p50_ms, p(|s| s.p50_ms), Better::Lower, Change::Percent));
        metric_row(s, asset_server, "99TH PERCENTILE FRAME", format!("{:.1} MS", now.p99_ms), delta(now.p99_ms, p(|s| s.p99_ms), Better::Lower, Change::Percent));
        metric_row(s, asset_server, "WORST FRAME", format!("{:.0} MS", now.worst_ms), delta(now.worst_ms, p(|s| s.worst_ms), Better::Lower, Change::Percent));
    }
    metric_row(s, asset_server, "TIME UNDER 60 FPS", format!("{:.1}%", now.under_60_pct), delta(now.under_60_pct, p(|s| s.under_60_pct), Better::Lower, Change::Points));
    if !short {
        metric_row(s, asset_server, "TIME UNDER 100 FPS", format!("{:.1}%", now.under_100_pct), delta(now.under_100_pct, p(|s| s.under_100_pct), Better::Lower, Change::Points));
        metric_row(s, asset_server, "HITCHES PER MIN (UNDER 30 FPS)", format!("{:.1}", now.hitches_per_min), delta(now.hitches_per_min, p(|s| s.hitches_per_min), Better::Lower, Change::Plain));
    }
}

/// "12:34" (or "1:02:03").
fn clock(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// How long ago `then` (unix seconds) was before `now`.
fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..60 => "JUST NOW".into(),
        60..3600 => format!("{} MIN AGO", secs / 60),
        3600..86_400 => format!("{} H AGO", secs / 3600),
        _ => format!("{} DAYS AGO", secs / 86_400),
    }
}

/// A bar's colour by its 1% low.
fn fps_color(fps: f32) -> Color {
    if fps < 60.0 {
        BAD
    } else if fps < 100.0 {
        WARN
    } else {
        GOOD
    }
}

fn spawn_panel(commands: &mut Commands, asset_server: &AssetServer, finished: Option<&Finished>) {
    commands
        .spawn((
            PerfPanelRoot,
            StateScoped(AppState::InGame),
            // Over the results screen (50).
            GlobalZIndex(60),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(SHADE),
            // (Clicks don't fall through to the results screen.)
            FocusPolicy::Block,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(960.0),
                    max_width: Val::Percent(96.0),
                    max_height: Val::Percent(96.0),
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderRadius::all(Val::Px(6.0)),
            ))
            .with_children(|panel| {
                // The header bar.
                panel
                    .spawn((
                        Node {
                            height: Val::Px(46.0),
                            flex_shrink: 0.0,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::SpaceBetween,
                            padding: UiRect::horizontal(Val::Px(18.0)),
                            ..default()
                        },
                        BackgroundColor(HEADER),
                    ))
                    .with_children(|head| {
                        head.spawn(hud_text(asset_server, "PERFORMANCE REPORT", 30.0, TEXT));
                        head.spawn(hud_text(asset_server, "DEBUG", 18.0, Color::srgba(1.0, 1.0, 1.0, 0.6)));
                    });
                panel
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(16.0)),
                        row_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|body| match finished {
                        Some(f) => report_body(body, asset_server, f),
                        None => {
                            body.spawn(hud_text(asset_server, "NOTHING RECORDED THIS GAME", 30.0, TEXT));
                            body.spawn(body_text(
                                asset_server,
                                "The recorder only runs while debug mode is on — turn it on before the next game starts.",
                                14.0,
                                DIM,
                            ));
                        }
                    });
                // The footer: where it's saved, and CLOSE.
                panel
                    .spawn((
                        Node {
                            flex_shrink: 0.0,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::SpaceBetween,
                            padding: UiRect::new(Val::Px(18.0), Val::Px(16.0), Val::Px(10.0), Val::Px(14.0)),
                            border: UiRect::top(Val::Px(1.0)),
                            column_gap: Val::Px(16.0),
                            ..default()
                        },
                        BorderColor(LINE),
                    ))
                    .with_children(|foot| {
                        let saved = match finished.map(|f| f.saved.as_deref()) {
                            Some(Some(path)) => format!("Saved to {path} (and the per-second timeline beside it, .csv)"),
                            Some(None) => "Couldn't save this report — see the log.".into(),
                            None => String::new(),
                        };
                        foot.spawn(body_text(asset_server, saved, 12.0, DIM));
                        foot.spawn((
                            PerfClose,
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(26.0), Val::Px(8.0)),
                                flex_shrink: 0.0,
                                ..default()
                            },
                            BackgroundColor(CLOSE_BG),
                            BorderRadius::all(Val::Px(3.0)),
                        ))
                        .with_child(hud_text(asset_server, "CLOSE", 22.0, TEXT));
                    });
            });
        });
}

fn report_body(body: &mut ChildSpawnerCommands, asset_server: &AssetServer, f: &Finished) {
    let r = &f.report;
    let prev = f.previous.as_ref();

    // What was played, and what it's compared with.
    let mut what = vec![r.map_label.clone(), r.mode.clone()];
    if r.start_round > 0 {
        what.push(format!("STARTED ROUND {}", r.start_round));
    }
    if r.full_power {
        what.push("FULL POWER".into());
    }
    what.push(format!("{} RECORDED", clock(r.overall.secs)));
    body.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(2.0),
        ..default()
    })
    .with_children(|top| {
        top.spawn(hud_text(asset_server, what.join("  ·  "), 24.0, TEXT));
        match prev {
            Some(p) => {
                top.spawn(Node {
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|line| {
                    line.spawn(body_text(
                        asset_server,
                        format!(
                            "Compared with your last run like it ({}) — green is better, red is worse.",
                            ago(r.unix_secs, p.unix_secs).to_lowercase()
                        ),
                        13.0,
                        DIM,
                    ));
                    let changed = r.setting_changes(p);
                    if !changed.is_empty() {
                        line.spawn(body_text(
                            asset_server,
                            format!("Settings differ: {}", changed.join(", ").to_lowercase()),
                            13.0,
                            WARN,
                        ));
                    }
                });
            }
            None => {
                top.spawn(body_text(
                    asset_server,
                    "First run on this map and mode from this start — the next one will be compared with it.",
                    13.0,
                    DIM,
                ));
            }
        }
    });

    // Two columns: the whole game | the exfil and by horde size.
    body.spawn(Node {
        column_gap: Val::Px(12.0),
        align_items: AlignItems::FlexStart,
        ..default()
    })
    .with_children(|cols| {
        cols.spawn(Node {
            flex_basis: Val::Px(0.0),
            flex_grow: 1.0,
            ..default()
        })
        .with_children(|left| {
            section(left, asset_server, "WHOLE GAME", "", |s| {
                stats_rows(s, asset_server, &r.overall, prev.map(|p| &p.overall), false);
            });
        });
        cols.spawn(Node {
            flex_basis: Val::Px(0.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            ..default()
        })
        .with_children(|right| {
            if let Some(exfil) = &r.exfil {
                section(right, asset_server, "EXFIL", &clock(exfil.secs), |s| {
                    stats_rows(s, asset_server, exfil, prev.and_then(|p| p.exfil.as_ref()), true);
                });
            }
            if !r.by_enemies.is_empty() {
                section(right, asset_server, "BY ENEMIES ON MAP", "AVERAGE / 1% LOW  ·  CHANGE IN 1% LOW", |s| {
                    for b in &r.by_enemies {
                        let before = prev.and_then(|p| p.by_enemies.iter().find(|pb| pb.from == b.from));
                        metric_row(
                            s,
                            asset_server,
                            &b.label(),
                            format!("{:.0} / {:.0}", b.stats.avg_fps, b.stats.low_1),
                            delta(b.stats.low_1, before.map(|pb| pb.stats.low_1), Better::Higher, Change::Percent),
                        );
                    }
                });
            }
        });
    });

    if !r.by_round.is_empty() {
        section(body, asset_server, "1% LOW BY ROUND", "RED UNDER 60  ·  AMBER UNDER 100", |s| {
            round_chart(s, asset_server, &r.by_round);
            leak_rows(s, asset_server, r);
        });
    }
}

/// A bar per round, its height the round's 1% low, a dim line at 60 FPS.
fn round_chart(s: &mut ChildSpawnerCommands, asset_server: &AssetServer, rounds: &[RoundStats]) {
    let top = rounds.iter().map(|r| r.stats.low_1).fold(60.0_f32, f32::max) * 1.1;
    let every = if rounds.len() <= 30 { 1 } else { 5 };
    s.spawn(Node {
        height: Val::Px(CHART_H + 34.0),
        margin: UiRect::vertical(Val::Px(6.0)),
        ..default()
    })
    .with_children(|chart| {
        // The 60 FPS line, behind the bars.
        chart.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(16.0 + CHART_H * (1.0 - 60.0 / top)),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.95, 0.32, 0.27, 0.45)),
        ));
        for (i, r) in rounds.iter().enumerate() {
            let low = r.stats.low_1;
            chart
                .spawn(Node {
                    flex_basis: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::FlexEnd,
                    margin: UiRect::horizontal(Val::Px(2.0)),
                    ..default()
                })
                .with_children(|col| {
                    if rounds.len() <= 24 {
                        col.spawn(body_text(asset_server, format!("{low:.0}"), 10.0, DIM));
                    }
                    col.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            max_width: Val::Px(34.0),
                            height: Val::Px((CHART_H * low / top).max(2.0)),
                            ..default()
                        },
                        BackgroundColor(fps_color(low)),
                        BorderRadius::top(Val::Px(2.0)),
                    ));
                    let label = if i % every == 0 || i + 1 == rounds.len() { r.round.to_string() } else { String::new() };
                    col.spawn((
                        Node {
                            height: Val::Px(16.0),
                            margin: UiRect::top(Val::Px(2.0)),
                            ..default()
                        },
                        children![hud_text(asset_server, label, 14.0, DIM)],
                    ));
                });
        }
    });
}

/// Entities and memory as the first round started vs the last: a steady
/// climb between them is something piling up over the game.
fn leak_rows(s: &mut ChildSpawnerCommands, asset_server: &AssetServer, r: &PerfReport) {
    let starts: Vec<&RoundStats> = r.by_round.iter().filter(|rs| rs.entities_at_start.is_some()).collect();
    let (Some(first), Some(last)) = (starts.first(), starts.last()) else {
        return;
    };
    if first.round == last.round {
        return;
    }
    s.spawn(Node {
        column_gap: Val::Px(24.0),
        margin: UiRect::top(Val::Px(4.0)),
        ..default()
    })
    .with_children(|line| {
        let growth = |a: f32, b: f32| if a > 0.0 { (b - a) / a * 100.0 } else { 0.0 };
        let (a, b) = (first.entities_at_start.unwrap_or(0), last.entities_at_start.unwrap_or(0));
        let g = growth(a as f32, b as f32);
        line.spawn(hud_text(
            asset_server,
            format!(
                "ENTITIES AT ROUND START  R{} {} → R{} {}  ({g:+.0}%)",
                first.round,
                crate::util::grouped(a),
                last.round,
                crate::util::grouped(b)
            ),
            18.0,
            if g > 10.0 { WARN } else { TEXT },
        ));
        if let (Some(a), Some(b)) = (first.mem_gb_at_start, last.mem_gb_at_start) {
            let g = growth(a, b);
            line.spawn(hud_text(
                asset_server,
                format!("MEMORY  {a:.2} → {b:.2} GB  ({g:+.0}%)"),
                18.0,
                if g > 10.0 { WARN } else { TEXT },
            ));
        }
        line.spawn(hud_text(
            asset_server,
            format!("PEAK ENTITIES  {}", crate::util::grouped(r.peak_entities)),
            18.0,
            DIM,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_of_a_steady_run_with_two_hitches() {
        // 998 frames at 100 FPS and two at 20 FPS.
        let dts: Vec<f32> = std::iter::repeat_n(0.01, 998).chain([0.05, 0.05]).collect();
        let s = Stats::of(dts);
        assert_eq!(s.frames, 1000);
        // The slowest 1% (10 frames) is the two hitches and eight steady ones.
        assert!((s.low_1 - 10.0 / 0.18).abs() < 0.01);
        assert!((s.low_01 - 20.0).abs() < 0.01);
        assert!((s.p50_ms - 10.0).abs() < 0.01);
        assert!((s.worst_ms - 50.0).abs() < 0.01);
        // 0.1 s of 10.08 s spent under 60 (and under 30, two hitches).
        assert!((s.under_60_pct - 0.1 / 10.08 * 100.0).abs() < 0.01);
        assert!((s.hitches_per_min - 2.0 / (10.08 / 60.0)).abs() < 0.01);
    }

    #[test]
    fn changes_are_coloured_by_which_way_is_better() {
        assert_eq!(delta(110.0, Some(100.0), Better::Higher, Change::Percent).unwrap().1, GOOD);
        assert_eq!(delta(110.0, Some(100.0), Better::Lower, Change::Percent).unwrap().1, BAD);
        assert_eq!(delta(101.0, Some(100.0), Better::Higher, Change::Percent).unwrap().1, DIM);
        assert!(delta(1.0, None, Better::Higher, Change::Percent).is_none());
    }
}
