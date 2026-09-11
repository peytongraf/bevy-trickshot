//! Player settings, persisted to the per-user config directory as JSON so they
//! survive both `cargo run` and the shipped build.
//!
//! * Linux:   `~/.config/bevy-trickshot/settings.json`
//! * Windows: `%APPDATA%\bevy-trickshot\settings.json`
//! * macOS:   `~/Library/Application Support/bevy-trickshot/settings.json`

use std::path::PathBuf;

use bevy::app::AppExit;
use bevy::prelude::*;
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
            .add_systems(Update, (arm_save, flush_save).chain())
            .add_systems(Last, save_on_exit);
    }
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
