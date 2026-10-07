//! `Zombies` level layouts: where everything a `Zombies` game puts on a map
//! stands — the perk machines, Der Wunderfizz, the Pack-a-Punch, the ammo
//! crate, the power switch, the wall buys, the Mystery Box, the armor
//! station and the exfil (its radio and area).
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
//!
//! How big each kind of thing is isn't the map's to say: a Mystery Box is
//! the same size everywhere. Each kind's size is in one file of its own,
//! `shared/levels/sizes.ron` ([`ObjectSizes`], [`sizes`]), and every
//! layout's placements take theirs from it as they're read
//! ([`ZombiesLayout::sized`]) — the places' files leave sizes out.

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
/// solid box both scale, its use range doesn't). The size is its kind's,
/// from [`ObjectSizes`] — the places' files don't keep it.
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

/// The exfil area (`crate::exfil`): a rectangle on the ground, `width` (m,
/// along its own x) by `depth` (along its own z), its middle and turn
/// `at`. Its size is the map's own — not [`ObjectSizes`]'.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ExfilArea {
    pub at: Placement,
    pub width: f32,
    pub depth: f32,
}

impl ExfilArea {
    /// A fresh one at `at`, [`crate::exfil::DEFAULT_AREA_SIZE`].
    pub fn new(at: Placement) -> Self {
        let size = crate::exfil::DEFAULT_AREA_SIZE;
        Self {
            at: at.with_scale(1.0),
            width: size.x,
            depth: size.y,
        }
    }

    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32| ((v as f64 * 1000.0).round() / 1000.0) as f32;
        format!(
            "(at: {}, width: {:?}, depth: {:?})",
            self.at.with_scale(1.0).to_ron(),
            r(self.width),
            r(self.depth)
        )
    }
}

/// How big each kind of thing a layout places is (`1` = as made, the
/// default) — the same on every map. Keyed by [`ZombiesLayout`]'s slot
/// names (`"mystery_box"`, `"armor_station"`, ...) and each perk machine's
/// perk ([`perk_size_key`]). Wall buys aren't in it: every sign's
/// [`crate::wall_buy::SIGN_SCALE`].
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(transparent)]
pub struct ObjectSizes(pub BTreeMap<String, f32>);

/// A perk machine's key in [`ObjectSizes`].
pub fn perk_size_key(perk: Perk) -> String {
    format!("{perk:?}")
}

impl ObjectSizes {
    /// The size for `key` (`1` when it isn't set).
    pub fn get(&self, key: &str) -> f32 {
        self.0.get(key).copied().unwrap_or(1.0)
    }

    /// Set `key`'s size (one of `1` isn't kept — it's the default).
    pub fn set(&mut self, key: &str, size: f32) {
        let size = (size as f64 * 1000.0).round() as f32 / 1000.0;
        if size == 1.0 {
            self.0.remove(key);
        } else {
            self.0.insert(key.to_string(), size);
        }
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    /// The file's text: a header, then one size a line.
    pub fn to_ron(&self) -> String {
        let mut out = String::from(
            "// How big each kind of thing in a Zombies layout is, on every map (1 = as\n\
             // made) — written by the client's level editor. See shared/src/level.rs.\n\
             {\n",
        );
        for (key, size) in &self.0 {
            out += &format!("    {key:?}: {size:?},\n");
        }
        out += "}\n";
        out
    }
}

/// The sizes file in the repo ([`LEVELS_DIR`]).
pub const SIZES_FILE: &str = "sizes.ron";

pub fn sizes_source_path() -> PathBuf {
    PathBuf::from(LEVELS_DIR).join(SIZES_FILE)
}

static SIZES: LazyLock<ObjectSizes> = LazyLock::new(|| {
    ObjectSizes::from_ron(include_str!("../levels/sizes.ron")).unwrap_or_else(|e| panic!("shared/levels/{SIZES_FILE}: {e}"))
});

/// Each kind of thing's size, as built in (the same on the client and the
/// server).
pub fn sizes() -> &'static ObjectSizes {
    &SIZES
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
    /// The Mystery Box ([`crate::mystery_box`]), if the place has one.
    #[serde(default)]
    pub mystery_box: Option<Placement>,
    /// The armor station ([`crate::armor`]), if the place has one.
    #[serde(default)]
    pub armor_station: Option<Placement>,
    /// The exfil radio (`crate::exfil`), if the place has one — exfil can
    /// only be called on a place with both it and [`Self::exfil_area`].
    #[serde(default)]
    pub exfil_radio: Option<Placement>,
    /// The area the party has to hold during an exfil.
    #[serde(default)]
    pub exfil_area: Option<ExfilArea>,
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

    /// Every sized placement (each but the wall buys), with its key in
    /// [`ObjectSizes`].
    fn sized_slots(&mut self) -> Vec<(String, &mut Placement)> {
        let mut out: Vec<(String, &mut Placement)> =
            self.perks.iter_mut().map(|(perk, at)| (perk_size_key(*perk), at)).collect();
        out.push(("wunderfizz".into(), &mut self.wunderfizz));
        for (key, slot) in [
            ("pack_a_punch", &mut self.pack_a_punch),
            ("ammo_crate", &mut self.ammo_crate),
            ("power_switch", &mut self.power_switch),
            ("mystery_box", &mut self.mystery_box),
            ("armor_station", &mut self.armor_station),
            ("exfil_radio", &mut self.exfil_radio),
        ] {
            if let Some(at) = slot.as_mut() {
                out.push((key.into(), at));
            }
        }
        out
    }

    /// This layout with every placement at its kind's size in `sizes`.
    pub fn sized(mut self, sizes: &ObjectSizes) -> Self {
        for (key, at) in self.sized_slots() {
            at.scale = sizes.get(&key);
        }
        self
    }

    /// The sizes this layout's placements are at — of the kinds it has.
    pub fn sizes_used(&self) -> Vec<(String, f32)> {
        self.clone().sized_slots().into_iter().map(|(key, at)| (key, at.scale)).collect()
    }

    /// The file's text: a header, then the layout — one thing a line, every
    /// value rounded ([`Placement::rounded`]), and no sizes (they're
    /// [`ObjectSizes`]').
    pub fn to_ron(&self) -> String {
        let layout = self.clone().sized(&ObjectSizes::default());
        layout.write_ron()
    }

    fn write_ron(&self) -> String {
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
        out += &format!("    mystery_box: {},\n", optional(self.mystery_box));
        out += &format!("    armor_station: {},\n", optional(self.armor_station));
        out += &format!("    exfil_radio: {},\n", optional(self.exfil_radio));
        out += &format!(
            "    exfil_area: {},\n",
            self.exfil_area.map_or("None".to_string(), |a| format!("Some({})", a.to_ron()))
        );
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
                .unwrap_or_else(|e| panic!("shared/levels/{}: {e}", file_name(p)))
                .sized(sizes());
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
    fn sizes_are_every_maps_and_the_places_files_dont_keep_them() {
        let mut sizes = ObjectSizes::default();
        sizes.set("wunderfizz", 1.25);
        sizes.set("mystery_box", 1.0);
        assert_eq!(sizes.0.len(), 1);
        for map in MapId::PLACES {
            let l = layout(map).clone().sized(&sizes);
            assert_eq!(l.wunderfizz.scale, 1.25);
            assert!(!l.to_ron().contains("scale:"));
        }
        let back = ObjectSizes::from_ron(&sizes.to_ron()).unwrap();
        assert_eq!(back, sizes);
    }

    #[test]
    fn the_built_in_layouts_are_at_the_built_in_sizes() {
        for map in MapId::PLACES {
            for (key, scale) in layout(map).sizes_used() {
                assert_eq!(scale, sizes().get(&key), "{key} on {map:?}");
            }
        }
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

