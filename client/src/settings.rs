//! Player settings, persisted to the per-user config directory as JSON so they
//! survive both `cargo run` and the shipped build.
//!
//! * Linux:   `~/.config/bevy-trickshot/settings.json`
//! * Windows: `%APPDATA%\bevy-trickshot\settings.json`
//! * macOS:   `~/Library/Application Support/bevy-trickshot/settings.json`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow};
use serde::{Deserialize, Serialize};

use crate::keybinds::KeyBindings;

pub const FOV_MIN: f32 = 60.0;
pub const FOV_MAX: f32 = 120.0;
pub const FOV_DEFAULT: f32 = 90.0;

pub const SENS_MIN: f32 = 0.10;
pub const SENS_MAX: f32 = 3.0;
pub const SENS_DEFAULT: f32 = 1.0;

/// ADS sensitivity is a multiplier on the look speed at full aim-down-sight,
/// relative to the hip. `1.0` = no slow-down; the default eases the zoomed view.
pub const ADS_SENS_MIN: f32 = 0.10;
pub const ADS_SENS_MAX: f32 = 2.0;
pub const ADS_SENS_DEFAULT: f32 = 0.4;

pub const VOLUME_MIN: f32 = 0.0;
pub const VOLUME_MAX: f32 = 1.0;
pub const VOLUME_DEFAULT: f32 = 1.0;

/// Custom FPS cap, used only while `Settings::vsync` is off — see
/// `apply_vsync` / `limit_frame_rate`.
pub const FRAME_LIMIT_MIN: f32 = 60.0;
pub const FRAME_LIMIT_MAX: f32 = 360.0;
pub const FRAME_LIMIT_DEFAULT: f32 = 240.0;

/// Shadow map quality, named and tiered the way Call of Duty's "Shadow Map"
/// graphics option is (Disabled / Low / Normal / High / Extra).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ShadowQuality {
    Disabled,
    Low,
    Normal,
    #[default]
    High,
    Extra,
}

impl ShadowQuality {
    pub const ALL: [ShadowQuality; 5] = [
        ShadowQuality::Disabled,
        ShadowQuality::Low,
        ShadowQuality::Normal,
        ShadowQuality::High,
        ShadowQuality::Extra,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ShadowQuality::Disabled => "DISABLED",
            ShadowQuality::Low => "LOW",
            ShadowQuality::Normal => "NORMAL",
            ShadowQuality::High => "HIGH",
            ShadowQuality::Extra => "EXTRA",
        }
    }
}

/// Which reticle texture the scope shows — the Loadout screen's only current
/// option (see `menu::build_loadout`). The actual `assets/textures/*.png`
/// path for each is `weapons::scope::crosshair_asset_path` — kept out of this
/// enum the same way `MapId` keeps its `.glb` paths out in `environment::map`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum CrosshairId {
    /// Hash-mark bullet-drop-compensator reticle, no dot.
    #[default]
    HashReticle,
    /// Circular scope vignette with a simple duplex crosshair.
    DuplexReticle,
    /// The hash-mark reticle with a glowing red dot at centre.
    HashReticleRedDot,
}

impl CrosshairId {
    pub const ALL: [CrosshairId; 3] = [
        CrosshairId::HashReticle,
        CrosshairId::DuplexReticle,
        CrosshairId::HashReticleRedDot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CrosshairId::HashReticle => "HASH",
            CrosshairId::DuplexReticle => "DUPLEX",
            CrosshairId::HashReticleRedDot => "RED DOT",
        }
    }
}

/// How readily [`crate::player::try_mantle`] catches the player on a ledge —
/// Call of Duty's own "Automatic Mantle" option, same three settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum AutoMantle {
    /// Never auto-mantles.
    Off,
    /// Only while actively jumping (`player::Jumping`) toward a ledge — a
    /// deliberate running jump at it, not just walking or falling into one.
    SemiAuto,
    /// Any time you're airborne and moving toward a mantleable ledge, jumped
    /// or not (e.g. walking or falling off a ledge onto a lower one).
    #[default]
    FullAuto,
}

impl AutoMantle {
    pub const ALL: [AutoMantle; 3] = [AutoMantle::Off, AutoMantle::SemiAuto, AutoMantle::FullAuto];

    pub fn label(self) -> &'static str {
        match self {
            AutoMantle::Off => "OFF",
            AutoMantle::SemiAuto => "SEMI-AUTO",
            AutoMantle::FullAuto => "FULL-AUTO",
        }
    }
}


/// Non-keybind settings. Keybinds live in [`KeyBindings`] and are saved to the
/// same file (see [`SettingsFile`]).
#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub username: Option<String>,
    /// Multiplier applied to the base mouse sensitivity.
    pub sensitivity: f32,
    /// Multiplier on look sensitivity at full ADS, relative to the hip.
    pub ads_sensitivity: f32,
    /// Hip (non-ADS) vertical FOV, degrees.
    pub fov: f32,
    /// Show the dev tuning panels in the top-right.
    pub debug_mode: bool,
    /// Dev convenience: on reaching the main menu, if no lobby exists, create one.
    pub dev_auto_create_lobby: bool,
    /// Dev convenience: on reaching the main menu, if a lobby exists, join it.
    pub dev_auto_join_lobby: bool,
    /// Shadow map resolution / cascade tier — the performance/quality trade-off.
    pub shadow_quality: ShadowQuality,
    /// Master volume, linear `0.0` (silent) .. `1.0` (full).
    pub master_volume: f32,
    /// Roll straight into the reload animation after the shot that empties the
    /// magazine, instead of leaving the sniper on an empty chamber until the
    /// player presses reload.
    pub auto_reload: bool,
    /// Vsync (`PresentMode::Fifo`). Off by default: this is a fast-aim
    /// trickshot shooter, and vsync's queued frames add noticeable
    /// input-to-screen latency and frame-pacing judder on top of capping the
    /// render rate to the display's refresh rate. See `apply_vsync`.
    pub vsync: bool,
    /// Custom FPS cap applied while `vsync` is off — see `limit_frame_rate`.
    /// Ignored while `vsync` is on (the display's own vblank paces frames).
    pub frame_limit: f32,
    /// Selected scope reticle — Loadout screen. See [`CrosshairId`].
    pub crosshair: CrosshairId,
    /// Automatic ledge-mantle behavior — Controls tab. See [`AutoMantle`].
    pub auto_mantle: AutoMantle,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            username: None,
            sensitivity: SENS_DEFAULT,
            ads_sensitivity: ADS_SENS_DEFAULT,
            fov: FOV_DEFAULT,
            debug_mode: false,
            dev_auto_create_lobby: false,
            dev_auto_join_lobby: false,
            shadow_quality: ShadowQuality::default(),
            master_volume: VOLUME_DEFAULT,
            auto_reload: true,
            vsync: false,
            frame_limit: FRAME_LIMIT_DEFAULT,
            crosshair: CrosshairId::default(),
            auto_mantle: AutoMantle::default(),
        }
    }
}

impl Settings {
    pub fn has_username(&self) -> bool {
        self.username.as_deref().is_some_and(|u| !u.trim().is_empty())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SettingsFile {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    keybinds: KeyBindings,
}

fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("bevy-trickshot").join("settings.json"))
}

fn load_file() -> SettingsFile {
    let Some(path) = config_path() else {
        return SettingsFile::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("[settings] {} is invalid ({e}); using defaults", path.display());
            SettingsFile::default()
        }),
        Err(_) => SettingsFile::default(), // no file yet — first run
    }
}

fn save_file(settings: &Settings, keybinds: &KeyBindings) {
    let Some(path) = config_path() else {
        eprintln!("[settings] no config directory available; not saving");
        return;
    };
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[settings] cannot create {}: {e}", dir.display());
            return;
        }
    }
    let file = SettingsFile {
        settings: settings.clone(),
        keybinds: keybinds.clone(),
    };
    match serde_json::to_string_pretty(&file) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("[settings] cannot write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[settings] serialize failed: {e}"),
    }
}

/// Coalesces rapid changes (slider drags) into one write.
#[derive(Resource)]
struct SaveDebounce(Timer);

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let loaded = load_file();
        let mut timer = Timer::from_seconds(0.6, TimerMode::Once);
        timer.pause();

        app.insert_resource(loaded.settings)
            .insert_resource(loaded.keybinds)
            .insert_resource(SaveDebounce(timer))
            .add_systems(Update, ((arm_save, flush_save).chain(), apply_vsync))
            // `save_on_exit` first so quitting isn't delayed by a pending sleep.
            .add_systems(Last, (save_on_exit, limit_frame_rate).chain());
    }
}

/// Push `Settings::vsync` onto the primary window's `PresentMode` whenever it
/// changes.
fn apply_vsync(
    settings: Res<Settings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut applied: Local<Option<bool>>,
) {
    if applied.is_some_and(|v| v == settings.vsync) {
        return;
    }
    *applied = Some(settings.vsync);
    window.present_mode = if settings.vsync {
        PresentMode::Fifo
    } else {
        PresentMode::AutoNoVsync
    };
}

/// Caps the frame rate to `Settings::frame_limit` while vsync is off, by
/// sleeping out whatever's left of the frame budget — with vsync on, the
/// display's own vblank already paces frames, so a manual cap on top of that
/// would just make frames miss it. Runs last in `Last`, so the wall-clock gap
/// between two consecutive calls to this system *is* one full frame's time
/// (this frame's work plus whatever this system slept last time), with
/// nothing else needed to mark "frame start".
fn limit_frame_rate(settings: Res<Settings>, mut last: Local<Option<Instant>>) {
    if settings.vsync || settings.frame_limit <= 0.0 {
        // Reset so re-enabling the cap later doesn't measure a gap that
        // spans however long it was off for.
        *last = None;
        return;
    }
    let budget = Duration::from_secs_f32(1.0 / settings.frame_limit);
    if let Some(prev) = *last {
        let elapsed = prev.elapsed();
        if elapsed < budget {
            std::thread::sleep(budget - elapsed);
        }
    }
    *last = Some(Instant::now());
}

fn arm_save(
    settings: Res<Settings>,
    keybinds: Res<KeyBindings>,
    mut debounce: ResMut<SaveDebounce>,
) {
    // `is_added()` is true on the first frame; skip that so a pristine default
    // set isn't written before the player touches anything.
    let changed = (settings.is_changed() && !settings.is_added())
        || (keybinds.is_changed() && !keybinds.is_added());
    if changed {
        debounce.0.reset();
        debounce.0.unpause();
    }
}

fn flush_save(
    time: Res<Time>,
    settings: Res<Settings>,
    keybinds: Res<KeyBindings>,
    mut debounce: ResMut<SaveDebounce>,
) {
    if debounce.0.paused() {
        return;
    }
    if debounce.0.tick(time.delta()).just_finished() {
        debounce.0.pause();
        save_file(&settings, &keybinds);
    }
}

fn save_on_exit(
    mut exits: EventReader<AppExit>,
    settings: Res<Settings>,
    keybinds: Res<KeyBindings>,
    debounce: Res<SaveDebounce>,
) {
    if exits.read().next().is_some() && !debounce.0.paused() {
        // A change is pending and the debounce hasn't fired — flush it now.
        save_file(&settings, &keybinds);
    }
}
