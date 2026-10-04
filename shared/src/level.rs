//! `Zombies` level layouts: where everything a `Zombies` game puts on a map
//! stands — the perk machines, Der Wunderfizz, the Pack-a-Punch, the ammo
//! crate, the power switch and the wall buys.
//!
//! Each place has its own file, `shared/levels/<place>.ron` ([`file_name`]),
//! compiled into the client and the server alike ([`layout`]) so they
//! always agree on it (buy ranges, solid boxes, the models). The day and
//! night versions of a place share one ([`place`]).
//!
//! The files are written by the client's level editor (main menu → LEVEL
//! EDITOR, `client::level_editor`), which saves straight over them when it's
//! run from the repo ([`source_path`]) — rebuild the client and the server
//! to play with the new layout. They're plain RON, so a hand edit works too.
//!
//! **Anything new a `Zombies` game puts on a map goes in here**, so the
//! editor can place it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::perks::Perk;
use crate::weapon::{WeaponId, WALL_BUY_WEAPONS};
use crate::MapId;

/// Where one thing stands: the ground under its middle, which way it faces
/// — its turn about the vertical axis (degrees, counter-clockwise seen from
/// above; `0` faces +Z) — and how big it is (`1` = as made; its model and
/// solid box both scale, its use range doesn't).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub pos: Vec3,
    pub yaw_deg: f32,
    #[serde(default = "unscaled")]
    pub scale: f32,
}

fn unscaled() -> f32 {
    1.0
}

impl Default for Placement {
    fn default() -> Self {
        Self::new(Vec3::ZERO, 0.0)
    }
}

impl Placement {
    pub const fn new(pos: Vec3, yaw_deg: f32) -> Self {
        Self { pos, yaw_deg, scale: 1.0 }
    }

    pub const fn with_scale(self, scale: f32) -> Self {
        Self { scale, ..self }
    }

    /// Its turn as a rotation.
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw_deg.to_radians())
    }

    /// Rounded to a millimetre and a tenth of a degree, so a saved file
    /// diffs cleanly.
    fn rounded(self) -> Self {
        let r = |v: f32, per: f64| ((v as f64 * per).round() / per) as f32;
        Self {
            pos: Vec3::new(r(self.pos.x, 1000.0), r(self.pos.y, 1000.0), r(self.pos.z, 1000.0)),
            yaw_deg: r(self.yaw_deg, 10.0),
            scale: r(self.scale, 1000.0),
        }
    }

    /// As RON, on one line.
    fn to_ron(self) -> String {
        let p = self.rounded();
        // (`{:?}` always writes a float with a decimal point; the scale only
        // when it isn't 1.)
        let scale = if p.scale == 1.0 { String::new() } else { format!(", scale: {:?}", p.scale) };
        format!(
            "(pos: ({:?}, {:?}, {:?}), yaw_deg: {:?}{scale})",
            p.pos.x, p.pos.y, p.pos.z, p.yaw_deg
        )
    }
}

/// A wall buy: the sign selling `weapon` (`crate::wall_buy`), standing at
/// `at` — the foot of its post, its front (with the weapon's outline) facing
/// the way it's turned.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WallBuy {
    pub weapon: WeaponId,
    pub at: Placement,
}

/// One place's `Zombies` layout — see the module docs.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ZombiesLayout {
    /// Each perk's machine ([`Perk::has_machine`]) — both sets', so a
    /// lobby can play either ([`crate::perks::PerkSet`]).
    pub perks: BTreeMap<Perk, Placement>,
    /// Der Wunderfizz (only in a classic-perks game).
    pub wunderfizz: Placement,
    /// The Pack-a-Punch machine, if the place has one.
    #[serde(default)]
    pub pack_a_punch: Option<Placement>,
    /// The ammo crate, if the place has one.
    #[serde(default)]
    pub ammo_crate: Option<Placement>,
    /// The power switch, if the place has one — without one, the power's
    /// always on there ([`crate::power::has_power`]).
    #[serde(default)]
    pub power_switch: Option<Placement>,
    /// The wall buys — at most one per gun ([`WALL_BUY_WEAPONS`]).
    #[serde(default)]
    pub wall_buys: Vec<WallBuy>,
}

impl ZombiesLayout {
    /// Where `perk`'s machine stands. (A perk missing from the file stands
    /// at the origin — the editor always writes every one.)
    pub fn perk(&self, perk: Perk) -> Placement {
        self.perks.get(&perk).copied().unwrap_or_default()
    }

    /// Where the wall buy selling `gun` stands, if the place has one — at
    /// every sign's one size ([`crate::wall_buy::SIGN_SCALE`]).
    pub fn wall_buy(&self, gun: WeaponId) -> Option<Placement> {
        self.wall_buys
            .iter()
            .find(|w| w.weapon == gun)
            .map(|w| w.at.with_scale(crate::wall_buy::SIGN_SCALE))
    }

    /// Put the wall buy selling `gun` at `at` (adding it if it isn't there).
    /// (Its scale's every sign's, so it isn't kept.)
    pub fn set_wall_buy(&mut self, gun: WeaponId, at: Placement) {
        let at = at.with_scale(1.0);
        match self.wall_buys.iter_mut().find(|w| w.weapon == gun) {
            Some(w) => w.at = at,
            None => self.wall_buys.push(WallBuy { weapon: gun, at }),
        }
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    /// The file's text: a header, then the layout — one thing a line, every
    /// value rounded ([`Placement::rounded`]).
    pub fn to_ron(&self) -> String {
        let optional = |p: Option<Placement>| p.map_or("None".to_string(), |p| format!("Some({})", p.to_ron()));
        let mut out = String::from(
            "// Zombies level layout — written by the client's level editor (main menu →\n\
             // LEVEL EDITOR). `pos` is the ground under each thing's middle (m), `yaw_deg`\n\
             // its turn about the vertical (degrees). See shared/src/level.rs.\n\
             (\n    perks: {\n",
        );
        for (perk, p) in &self.perks {
            out += &format!("        {perk:?}: {},\n", p.to_ron());
        }
        out += "    },\n";
        out += &format!("    wunderfizz: {},\n", self.wunderfizz.to_ron());
        out += &format!("    pack_a_punch: {},\n", optional(self.pack_a_punch));
        out += &format!("    ammo_crate: {},\n", optional(self.ammo_crate));
        out += &format!("    power_switch: {},\n", optional(self.power_switch));
        out += "    wall_buys: [\n";
        for gun in WALL_BUY_WEAPONS {
            if let Some(at) = self.wall_buys.iter().find(|w| w.weapon == gun).map(|w| w.at.with_scale(1.0)) {
                out += &format!("        (weapon: {gun:?}, at: {}),\n", at.to_ron());
            }
        }
        out += "    ],\n";
        out += ")\n";
        out
    }
}

/// The place `map` is — the day and night versions of one share a layout.
pub fn place(map: MapId) -> MapId {
    match map {
        MapId::ShipmentDay => MapId::Shipment,
        MapId::BreakPointNight => MapId::BreakPoint,
        other => other,
    }
}

/// The layout file's name for `map`'s place.
pub fn file_name(map: MapId) -> &'static str {
    match place(map) {
        MapId::Shipment | MapId::ShipmentDay => "shipment.ron",
        MapId::BreakPoint | MapId::BreakPointNight => "break_point.ron",
        MapId::AshesOfTheDamned => "ashes_of_the_damned.ron",
        MapId::BasicMap => "basic_map.ron",
    }
}

/// Where the layout files are in the repo this was built from. The editor
/// saves there when it exists (a dev build) — a downloaded release has no
/// repo, so it only prints the file instead.
pub const LEVELS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/levels");

/// `map`'s layout file in the repo ([`LEVELS_DIR`]).
pub fn source_path(map: MapId) -> PathBuf {
    PathBuf::from(LEVELS_DIR).join(file_name(map))
}

/// Each place's file, as built in.
fn built_in_text(map: MapId) -> &'static str {
    match place(map) {
        MapId::Shipment | MapId::ShipmentDay => include_str!("../levels/shipment.ron"),
        MapId::BreakPoint | MapId::BreakPointNight => include_str!("../levels/break_point.ron"),
        MapId::AshesOfTheDamned => include_str!("../levels/ashes_of_the_damned.ron"),
        MapId::BasicMap => include_str!("../levels/basic_map.ron"),
    }
}

static LAYOUTS: LazyLock<Vec<(MapId, ZombiesLayout)>> = LazyLock::new(|| {
    MapId::PLACES
        .iter()
        .map(|&p| {
            let layout = ZombiesLayout::from_ron(built_in_text(p))
                .unwrap_or_else(|e| panic!("shared/levels/{}: {e}", file_name(p)));
            (place(p), layout)
        })
        .collect()
});

/// `map`'s layout, as built in (the same on the client and the server).
pub fn layout(map: MapId) -> &'static ZombiesLayout {
    let place = place(map);
    &LAYOUTS
        .iter()
        .find(|(p, _)| *p == place)
        .expect("every place has a layout file")
        .1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_place_has_a_layout_with_every_machine() {
        for map in MapId::PLACES {
            let l = layout(map);
            for perk in Perk::ALL.into_iter().filter(|p| p.has_machine()) {
                assert!(l.perks.contains_key(&perk), "{perk:?} missing on {map:?}");
            }
        }
    }

    #[test]
    fn a_scale_is_kept_and_an_unscaled_one_left_out() {
        let mut l = layout(MapId::BreakPoint).clone();
        l.wunderfizz = l.wunderfizz.with_scale(1.25);
        let text = l.to_ron();
        assert!(text.contains("scale: 1.25"));
        assert_eq!(text.matches("scale:").count(), 1);
        assert_eq!(ZombiesLayout::from_ron(&text).unwrap().wunderfizz.scale, 1.25);
    }

    #[test]
    fn every_wall_buy_is_one_size_and_the_files_dont_say() {
        let mut l = layout(MapId::BreakPoint).clone();
        l.set_wall_buy(WeaponId::Ak74, Placement::new(Vec3::ONE, 0.0).with_scale(3.0));
        assert_eq!(l.wall_buy(WeaponId::Ak74).unwrap().scale, crate::wall_buy::SIGN_SCALE);
        assert!(!l.to_ron().contains("scale:"));
    }

    #[test]
    fn day_and_night_share_a_layout() {
        assert_eq!(layout(MapId::BreakPoint), layout(MapId::BreakPointNight));
        assert_eq!(layout(MapId::Shipment), layout(MapId::ShipmentDay));
    }

    #[test]
    fn a_saved_layout_reads_back_the_same() {
        for map in MapId::PLACES {
            let l = layout(map);
            let back = ZombiesLayout::from_ron(&l.to_ron()).unwrap();
            assert_eq!(back.to_ron(), l.to_ron());
        }
    }
}

