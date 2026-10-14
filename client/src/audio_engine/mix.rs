//! The game's mix: every sound's level, which bus it's on and whether it
//! feeds the room's reverb — tuned in the audio mixer ([`super::mixer`])
//! and saved to `assets/audio/mix.ron`, which ships with the game.
//!
//! A clip's settings come from its own entry and the entries of the folders
//! it's in (keyed by path under `assets/audio/`, e.g.
//! `weapons/sniper/shot.wav` sits in `weapons/sniper` and `weapons`):
//! - level: its bus's level + every enclosing folder's + its own (all dB),
//!   plus — when it's played positionally — every `positional_db` too;
//! - bus / reverb / profile: its own setting, else the nearest folder's
//!   that has one.
//!
//! Positional sounds also go by a [`Profile`]: how they fade with distance
//! (and lose their highs to the air), how widely they pan — one shared
//! definition for every sound that uses it.
//!
//! Applied by the engine on top of whatever volume the sound was played at
//! — live, to sounds already playing too (reverb only takes effect on sounds
//! started after the change).

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// A group of sounds sharing one level fader (and mute / solo).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum Bus {
    Weapons,
    Movement,
    Zombies,
    World,
    Explosions,
    Quotes,
    Announcer,
    Interface,
    Music,
    Ambience,
}

impl Bus {
    pub(crate) const ALL: [Bus; 10] = [
        Bus::Weapons,
        Bus::Movement,
        Bus::Zombies,
        Bus::World,
        Bus::Explosions,
        Bus::Quotes,
        Bus::Announcer,
        Bus::Interface,
        Bus::Music,
        Bus::Ambience,
    ];

    /// Where a clip goes when neither it nor any folder it's in picks one.
    pub(crate) const DEFAULT: Bus = Bus::World;

    pub(crate) fn label(self) -> &'static str {
        match self {
            Bus::Weapons => "Weapons",
            Bus::Movement => "Movement",
            Bus::Zombies => "Zombies",
            Bus::World => "World",
            Bus::Explosions => "Explosions",
            Bus::Quotes => "Quotes",
            Bus::Announcer => "Announcer",
            Bus::Interface => "Interface (UI & HUD)",
            Bus::Music => "Music",
            Bus::Ambience => "Ambience",
        }
    }
}

/// One clip's or folder's settings. Everything defaults to "no change".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MixEntry {
    /// Added to the level (dB).
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) volume_db: f32,
    /// Added to the level (dB) when it's played positionally (e.g. another
    /// player's rechamber, far quieter than our own).
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) positional_db: f32,
    /// Its positional [`Profile`] (by name). `None`: whatever the enclosing
    /// folder says — and if nothing does, [`DEFAULT_PROFILE`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<String>,
    /// `None`: whatever the enclosing folder says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bus: Option<Bus>,
    /// `None`: whatever the enclosing folder says — and if nothing does,
    /// on for positional sounds, off for the rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reverb: Option<bool>,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

impl MixEntry {
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What a clip ends up with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resolved {
    /// Bus + folders + clip (dB), before mute / solo.
    pub(crate) volume_db: f32,
    /// Extra (dB) when played positionally.
    pub(crate) positional_db: f32,
    pub(crate) bus: Bus,
    pub(crate) reverb: Option<bool>,
    pub(crate) profile: String,
}

/// The profile a positional sound gets when nothing picks one.
pub(crate) const DEFAULT_PROFILE: &str = "Default";

/// How a positional sound carries: its fade with distance, the highs it
/// loses to the air, how widely it pans. Shared by every sound that uses it,
/// and applied live.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Profile {
    /// Added to the level of every sound using it (dB).
    pub(crate) volume_db: f32,
    /// Full volume within this far (m)...
    pub(crate) near: f32,
    /// ...fading to silence here.
    pub(crate) far: f32,
    /// The fade's shape between them: `1` linear, higher drops off sooner.
    pub(crate) curve: f32,
    /// How hard it pans to the side it's on (`0` both ears alike … `1` the
    /// far ear silent).
    pub(crate) width: f32,
    /// Air absorption: the low-pass cutoff (Hz) at `far`, eased in from
    /// `near` — distant sounds go dull. 20 kHz or more is off.
    pub(crate) air_hz: f32,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            near: 0.0,
            far: 60.0,
            curve: 2.0,
            width: 0.75,
            air_hz: AIR_OFF_HZ,
        }
    }
}

/// Cutoffs at or above this leave the sound alone.
pub(crate) const AIR_OFF_HZ: f32 = 20_000.0;

impl Profile {
    /// How far between `near` (0) and `far` (1) `distance` is.
    fn t(&self, distance: f32) -> f32 {
        let span = (self.far - self.near).max(1e-3);
        ((distance - self.near) / span).clamp(0.0, 1.0)
    }

    /// The linear gain `distance` m off (its level included).
    pub(crate) fn gain(&self, distance: f32) -> f32 {
        if distance >= self.far {
            return 0.0;
        }
        db_to_linear(self.volume_db) * (1.0 - self.t(distance)).powf(self.curve.max(0.01))
    }

    /// The air-absorption cutoff `distance` m off, or `None` when it's off
    /// there. Eased geometrically, so it falls evenly by ear.
    pub(crate) fn air_cutoff(&self, distance: f32) -> Option<f32> {
        if self.air_hz >= AIR_OFF_HZ {
            return None;
        }
        let t = self.t(distance);
        (t > 0.0).then(|| AIR_OFF_HZ * (self.air_hz.max(20.0) / AIR_OFF_HZ).powf(t))
    }
}

/// The saved mix (`assets/audio/mix.ron`).
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Mix {
    /// Each bus's level (dB); a missing one is 0.
    pub(crate) buses: BTreeMap<Bus, f32>,
    /// Positional profiles by name; [`DEFAULT_PROFILE`] if missing is
    /// [`Profile::default`].
    pub(crate) profiles: BTreeMap<String, Profile>,
    /// By folder path under `assets/audio/` (`weapons/sniper`).
    pub(crate) folders: BTreeMap<String, MixEntry>,
    /// By clip path under `assets/audio/` (`weapons/sniper/shot.wav`).
    pub(crate) clips: BTreeMap<String, MixEntry>,
}

impl Mix {
    pub(crate) fn bus_db(&self, bus: Bus) -> f32 {
        self.buses.get(&bus).copied().unwrap_or(0.0)
    }

    pub(crate) fn resolve(&self, clip: &str) -> Resolved {
        let mut volume_db = 0.0;
        let mut positional_db = 0.0;
        let mut bus = None;
        let mut reverb = None;
        let mut profile = None;
        // Innermost first, so the nearest setting wins.
        let own = self.clips.get(clip);
        let folders = enclosing_folders(clip).filter_map(|f| self.folders.get(f));
        for entry in own.into_iter().chain(folders) {
            volume_db += entry.volume_db;
            positional_db += entry.positional_db;
            bus = bus.or(entry.bus);
            reverb = reverb.or(entry.reverb);
            profile = profile.or(entry.profile.as_deref());
        }
        let bus = bus.unwrap_or(Bus::DEFAULT);
        Resolved {
            volume_db: volume_db + self.bus_db(bus),
            positional_db,
            bus,
            reverb,
            profile: profile.unwrap_or(DEFAULT_PROFILE).to_owned(),
        }
    }

    /// The profile called `name` (a missing one is the default).
    pub(crate) fn profile(&self, name: &str) -> Profile {
        self.profiles.get(name).cloned().unwrap_or_default()
    }

    /// A copy without entries left at "no change" — what's saved, and what
    /// two mixes are compared by.
    pub(crate) fn pruned(&self) -> Self {
        let mut pruned = self.clone();
        pruned.buses.retain(|_, db| *db != 0.0);
        pruned.folders.retain(|_, e| !e.is_default());
        pruned.clips.retain(|_, e| !e.is_default());
        pruned
    }

    pub(crate) fn path() -> PathBuf {
        bevy::asset::io::file::FileAssetReader::get_base_path().join("assets/audio/mix.ron")
    }

    /// The saved mix, or an empty one (every sound at its own level) if
    /// there isn't one or it doesn't parse.
    pub(crate) fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
                warn!("{} doesn't parse, using an empty mix: {e}", path.display());
                Self::default()
            }),
            Err(e) => {
                warn!("no mix at {} ({e}), using an empty one", path.display());
                Self::default()
            }
        }
    }

    pub(crate) fn save(&self) -> Result<PathBuf, String> {
        let text = ron::ser::to_string_pretty(&self.pruned(), ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        let path = Self::path();
        std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }
}

/// `a/b/c.mp3` → `a/b`, `a`.
fn enclosing_folders(clip: &str) -> impl Iterator<Item = &str> {
    let mut rest = clip;
    std::iter::from_fn(move || {
        let (folder, _) = rest.rsplit_once('/')?;
        rest = folder;
        Some(folder)
    })
}

/// Mute / solo — for listening while mixing; never saved.
#[derive(Resource, Default)]
pub(crate) struct BusMonitor {
    pub(crate) muted: HashSet<Bus>,
    pub(crate) soloed: HashSet<Bus>,
}

impl BusMonitor {
    fn audible(&self, bus: Bus) -> bool {
        !self.muted.contains(&bus) && (self.soloed.is_empty() || self.soloed.contains(&bus))
    }
}

/// The linear gain the mix gives `clip` right now (mute / solo included;
/// `positional` adds its positional offset).
pub(crate) fn clip_gain(mix: &Mix, monitor: &BusMonitor, clip: &str, positional: bool) -> (f32, Resolved) {
    let resolved = mix.resolve(clip);
    let db = resolved.volume_db + if positional { resolved.positional_db } else { 0.0 };
    let gain = if monitor.audible(resolved.bus) { db_to_linear(db) } else { 0.0 };
    (gain, resolved)
}

pub(crate) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// `audio/weapons/sniper/shot.wav` (an asset path) → the mix's key for it,
/// `weapons/sniper/shot.wav`.
pub(crate) fn clip_key(asset_path: &str) -> &str {
    asset_path.strip_prefix("audio/").unwrap_or(asset_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_add_up_and_the_nearest_wins() {
        let mut mix = Mix::default();
        mix.buses.insert(Bus::Weapons, -2.0);
        mix.folders.insert(
            "weapons".into(),
            MixEntry { volume_db: -1.0, bus: Some(Bus::Weapons), reverb: Some(true), ..default() },
        );
        mix.folders.insert(
            "weapons/sniper".into(),
            MixEntry { volume_db: 3.0, reverb: Some(false), profile: Some("Close".into()), ..default() },
        );
        mix.clips.insert(
            "weapons/sniper/shot.wav".into(),
            MixEntry { volume_db: 0.5, ..default() },
        );
        let r = mix.resolve("weapons/sniper/shot.wav");
        assert_eq!(r.volume_db, -2.0 - 1.0 + 3.0 + 0.5);
        assert_eq!(r.bus, Bus::Weapons);
        assert_eq!(r.reverb, Some(false));
        assert_eq!(r.profile, "Close");
        let other = mix.resolve("ui/denied.mp3");
        assert_eq!((other.volume_db, other.bus, other.reverb), (0.0, Bus::DEFAULT, None));
        assert_eq!(other.profile, DEFAULT_PROFILE);
    }

    #[test]
    fn profile_fades_and_dulls_with_distance() {
        let p = Profile { near: 2.0, far: 12.0, curve: 1.0, air_hz: 2_000.0, ..default() };
        assert_eq!(p.gain(1.0), 1.0);
        assert!((p.gain(7.0) - 0.5).abs() < 1e-6);
        assert_eq!(p.gain(12.0), 0.0);
        assert_eq!(p.air_cutoff(1.0), None);
        assert!((p.air_cutoff(12.0).unwrap() - 2_000.0).abs() < 1.0);
        assert_eq!(Profile::default().air_cutoff(30.0), None);
    }

    /// The shipped mix parses, saves back to the same thing, and only names
    /// folders and clips that exist.
    #[test]
    fn shipped_mix_is_sound() {
        let audio = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/audio");
        let text = std::fs::read_to_string(audio.join("mix.ron")).unwrap();
        let mix: Mix = ron::from_str(&text).unwrap();
        let saved = ron::ser::to_string_pretty(&mix.pruned(), ron::ser::PrettyConfig::default()).unwrap();
        assert_eq!(ron::from_str::<Mix>(&saved).unwrap(), mix);
        for folder in mix.folders.keys() {
            assert!(audio.join(folder).is_dir(), "no folder {folder}");
        }
        for clip in mix.clips.keys() {
            assert!(audio.join(clip).is_file(), "no clip {clip}");
        }
        for entry in mix.folders.values().chain(mix.clips.values()) {
            if let Some(profile) = &entry.profile {
                assert!(mix.profiles.contains_key(profile), "no profile {profile}");
            }
        }
    }
}
