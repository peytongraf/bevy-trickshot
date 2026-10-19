//! `Zombies` level layouts: where everything a `Zombies` game puts on a map
//! stands — the perk machines, Der Wunderfizz, the Pack-a-Punch, the ammo
//! crate, the power switch, the wall buys, the Mystery Box, the armor
//! station, the crafting table, the exfil (its radio and area), the Rampage
//! Inducer, the lights the power turns on and where the players start.
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

use bevy::math::{Quat, Vec2, Vec3};
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

/// One of the map's power lights: on once someone turns the power on in a
/// `Zombies` game (and always on in the other modes) — the client fades it
/// in (`client::power`). Only a night map's are ever lit
/// ([`MapId::is_night`]): the day version of a place shares the layout but
/// leaves them off. Placed and tuned in the level editor.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PowerLight {
    /// Where it hangs (m, world).
    pub pos: Vec3,
    /// Its colour (sRGB, 0..1 each).
    pub color: [f32; 3],
    /// How bright (lumens) at full power.
    pub intensity: f32,
    /// How far (m) it reaches.
    pub range: f32,
    /// The size (m) of its glowing source — softens highlights and shadows.
    #[serde(default = "default_light_radius")]
    pub radius: f32,
    /// Whether it casts shadows (costly — keep it to the few that need it).
    #[serde(default)]
    pub shadows: bool,
    /// A spotlight's aim and cone — `None` for a light shining all round.
    #[serde(default)]
    pub spot: Option<SpotCone>,
    /// A glowing bulb drawn where it hangs, lighting just around itself
    /// (lumens, at full power) — 0 for none.
    #[serde(default)]
    pub glow: f32,
}

/// Which way a spotlight [`PowerLight`] shines and how wide.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SpotCone {
    /// Its turn about the vertical (degrees).
    pub yaw_deg: f32,
    /// How far it tips up (+) or down (−) (degrees).
    pub pitch_deg: f32,
    /// Half-angles (degrees) of its fully-bright core and its edge.
    pub inner_angle_deg: f32,
    pub outer_angle_deg: f32,
}

impl Default for SpotCone {
    fn default() -> Self {
        Self {
            yaw_deg: 0.0,
            pitch_deg: -45.0,
            inner_angle_deg: 30.0,
            outer_angle_deg: 40.0,
        }
    }
}

impl SpotCone {
    /// Its aim as a rotation (a spotlight shines along its local −Z).
    pub fn rotation(self) -> Quat {
        Quat::from_euler(
            bevy::math::EulerRot::YXZ,
            self.yaw_deg.to_radians(),
            self.pitch_deg.to_radians(),
            0.0,
        )
    }
}

fn default_light_radius() -> f32 {
    0.3
}

/// Most power lights a map keeps.
pub const MAX_POWER_LIGHTS: usize = 16;

impl PowerLight {
    /// A fresh one at `pos`: warm white, a room's worth of light.
    pub fn new(pos: Vec3) -> Self {
        Self {
            pos,
            color: [1.0, 0.95, 0.85],
            intensity: 2_000_000.0,
            range: 30.0,
            radius: default_light_radius(),
            shadows: false,
            spot: None,
            glow: 0.0,
        }
    }

    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32, per: f64| ((v as f64 * per).round() / per) as f32;
        let [cr, cg, cb] = self.color.map(|c| r(c, 1000.0));
        let mut out = format!(
            "(pos: ({:?}, {:?}, {:?}), color: ({cr:?}, {cg:?}, {cb:?}), intensity: {:?}, range: {:?}, radius: {:?}, shadows: {}",
            r(self.pos.x, 1000.0),
            r(self.pos.y, 1000.0),
            r(self.pos.z, 1000.0),
            self.intensity.round(),
            r(self.range, 100.0),
            r(self.radius, 1000.0),
            self.shadows,
        );
        if let Some(c) = self.spot {
            out += &format!(
                ", spot: Some((yaw_deg: {:?}, pitch_deg: {:?}, inner_angle_deg: {:?}, outer_angle_deg: {:?}))",
                r(c.yaw_deg, 10.0),
                r(c.pitch_deg, 10.0),
                r(c.inner_angle_deg, 10.0),
                r(c.outer_angle_deg, 10.0),
            );
        }
        if self.glow > 0.0 {
            out += &format!(", glow: {:?}", self.glow.round());
        }
        out + ")"
    }
}

/// A fire burning on the map — in a wall-mounted bowl, say — the whole time,
/// power on or off, day or night: flames and a flickering light (the
/// client's `flames`). Placed and tuned in the level editor.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Flame {
    /// The bottom of the flames (m, world).
    pub pos: Vec3,
    /// How big the fire is (m, across).
    pub size: f32,
    /// How tall, as a multiple of `size`.
    #[serde(default = "default_flame_height")]
    pub height: f32,
    /// How bright the flames themselves are.
    #[serde(default = "default_flame_brightness")]
    pub brightness: f32,
    /// Its light: how bright (lumens) and how far (m) it reaches...
    pub light: f32,
    pub range: f32,
    /// ...and how much it flickers (0 = steady).
    #[serde(default = "default_flame_flicker")]
    pub flicker: f32,
}

fn default_flame_height() -> f32 {
    1.6
}

fn default_flame_brightness() -> f32 {
    1.0
}

fn default_flame_flicker() -> f32 {
    0.35
}

/// Most fires a map keeps.
pub const MAX_FLAMES: usize = 32;

impl Flame {
    /// A fresh one at `pos`: a bowl's worth of fire.
    pub fn new(pos: Vec3) -> Self {
        Self {
            pos,
            size: 0.45,
            height: default_flame_height(),
            brightness: default_flame_brightness(),
            light: 60_000.0,
            range: 9.0,
            flicker: default_flame_flicker(),
        }
    }

    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32, per: f64| ((v as f64 * per).round() / per) as f32;
        format!(
            "(pos: ({:?}, {:?}, {:?}), size: {:?}, height: {:?}, brightness: {:?}, light: {:?}, range: {:?}, flicker: {:?})",
            r(self.pos.x, 1000.0),
            r(self.pos.y, 1000.0),
            r(self.pos.z, 1000.0),
            r(self.size, 1000.0),
            r(self.height, 100.0),
            r(self.brightness, 100.0),
            self.light.round(),
            r(self.range, 100.0),
            r(self.flicker, 100.0),
        )
    }
}

/// A pool of still water on the map — a flooded floor, a basin — its
/// surface a flat rectangle (the client's `water`: rippling, see-through and
/// bending what's under it). Only how it looks: shallow enough to wade, it
/// doesn't slow or stop anything. Placed and tuned in the level editor.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Water {
    /// The middle of its surface (m, world) and its turn.
    pub at: Placement,
    /// Its surface's `width × length` (m, along its own x and z).
    pub size: Vec2,
    /// The colour the water tints what's under it, deeper down (sRGB).
    #[serde(default = "default_water_color")]
    pub color: [f32; 3],
    /// How far (m) down light goes before it takes on [`Self::color`] —
    /// bigger, clearer water.
    #[serde(default = "default_water_clarity")]
    pub clarity: f32,
    /// How deep it looks (m): how far the floor under it seems bent.
    #[serde(default = "default_water_depth")]
    pub depth: f32,
    /// How rough its surface is (0 = a mirror-flat millpond).
    #[serde(default = "default_water_waves")]
    pub waves: f32,
    /// How strongly the light it bends dances on the floor under it.
    #[serde(default = "default_water_caustics")]
    pub caustics: f32,
}

fn default_water_color() -> [f32; 3] {
    [0.1, 0.62, 0.6]
}

fn default_water_clarity() -> f32 {
    1.2
}

fn default_water_depth() -> f32 {
    0.6
}

fn default_water_waves() -> f32 {
    0.5
}

fn default_water_caustics() -> f32 {
    0.6
}

/// Most pools a map keeps.
/// Deepest (m) under a [`Water`]'s surface that feet count as wading in it.
pub const WADE_DEPTH: f32 = 1.5;

pub const MAX_WATERS: usize = 16;

impl Water {
    /// A fresh one, its surface's middle at `at`.
    pub fn new(at: Placement) -> Self {
        Self {
            at: at.with_scale(1.0),
            size: Vec2::new(6.0, 6.0),
            color: default_water_color(),
            clarity: default_water_clarity(),
            depth: default_water_depth(),
            waves: default_water_waves(),
            caustics: default_water_caustics(),
        }
    }

    /// Whether `p` is over (or under) its surface, across the ground.
    pub fn covers(&self, p: Vec3) -> bool {
        let local = self.at.rotation().inverse() * (p - self.at.pos);
        local.x.abs() <= self.size.x * 0.5 && local.z.abs() <= self.size.y * 0.5
    }

    /// Whether someone with their feet at `feet` is wading in it: over it
    /// and under its surface — but not a floor well below it, under a
    /// bridge, say.
    pub fn wading(&self, feet: Vec3) -> bool {
        let below = self.at.pos.y - feet.y;
        self.covers(feet) && below > 0.0 && below < WADE_DEPTH
    }

    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32, per: f64| ((v as f64 * per).round() / per) as f32;
        format!(
            "(at: {}, size: ({:?}, {:?}), color: ({:?}, {:?}, {:?}), clarity: {:?}, depth: {:?}, waves: {:?}, caustics: {:?})",
            self.at.with_scale(1.0).to_ron(),
            r(self.size.x, 1000.0),
            r(self.size.y, 1000.0),
            r(self.color[0], 1000.0),
            r(self.color[1], 1000.0),
            r(self.color[2], 1000.0),
            r(self.clarity, 100.0),
            r(self.depth, 100.0),
            r(self.waves, 100.0),
            r(self.caustics, 100.0),
        )
    }
}

/// A waterfall pouring off a wall — out of a spout or over a ledge — down
/// into a pool (a [`Water`]) or onto the floor: a falling sheet of water
/// that breaks up into strands as it drops, and a churning, misty splash
/// where it lands (the client's `water`). Only how it looks. Placed and
/// tuned in the level editor.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Waterfall {
    /// The middle of its lip — where the water leaves the wall — and its
    /// turn: it pours out along its front (+Z).
    pub at: Placement,
    /// How wide (m) it is at the lip.
    pub width: f32,
    /// How far (m) it falls — where it lands is this far below the lip.
    pub drop: f32,
    /// How fast (m/s) the water comes off the lip — how far out from the
    /// wall it arcs (0 = straight down the wall).
    #[serde(default = "default_waterfall_flow")]
    pub flow: f32,
    /// How much it churns and mists up where it lands (0 = none).
    #[serde(default = "default_waterfall_spray")]
    pub spray: f32,
    /// How solid the sheet looks (0 = see-through strands only).
    #[serde(default = "default_waterfall_opacity")]
    pub opacity: f32,
}

fn default_waterfall_flow() -> f32 {
    1.2
}

fn default_waterfall_spray() -> f32 {
    1.0
}

fn default_waterfall_opacity() -> f32 {
    0.8
}

/// Most waterfalls a map keeps.
pub const MAX_WATERFALLS: usize = 16;

/// Gravity (m/s²) a waterfall falls with.
const WATERFALL_G: f32 = 9.81;

impl Waterfall {
    /// A fresh one, its lip at `at`.
    pub fn new(at: Placement) -> Self {
        Self {
            at: at.with_scale(1.0),
            width: 1.2,
            drop: 3.0,
            flow: default_waterfall_flow(),
            spray: default_waterfall_spray(),
            opacity: default_waterfall_opacity(),
        }
    }

    pub fn rotation(&self) -> Quat {
        self.at.rotation()
    }

    /// How long (s) the water takes to fall from the lip to where it lands.
    pub fn fall_time(&self) -> f32 {
        (2.0 * self.drop.max(0.0) / WATERFALL_G).sqrt()
    }

    /// Where the water is `t` seconds after leaving the lip, in its own
    /// frame (x across it, y up, z out from the wall).
    pub fn local_at(&self, t: f32) -> Vec3 {
        Vec3::new(0.0, -0.5 * WATERFALL_G * t * t, self.flow * t)
    }

    /// Where (the middle of) it lands, in the world.
    pub fn foot(&self) -> Vec3 {
        self.at.pos + self.rotation() * self.local_at(self.fall_time())
    }

    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32, per: f64| ((v as f64 * per).round() / per) as f32;
        format!(
            "(at: {}, width: {:?}, drop: {:?}, flow: {:?}, spray: {:?}, opacity: {:?})",
            self.at.with_scale(1.0).to_ron(),
            r(self.width, 1000.0),
            r(self.drop, 1000.0),
            r(self.flow, 100.0),
            r(self.spray, 100.0),
            r(self.opacity, 100.0),
        )
    }
}

impl crate::doors::Door {
    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32| ((v as f64 * 1000.0).round() / 1000.0) as f32;
        let main = if self.main { ", main: true" } else { "" };
        format!(
            "(at: {}, size: ({:?}, {:?}, {:?}), cost: {}, requires_power: {}, slide: {:?}{main})",
            self.at.with_scale(1.0).to_ron(),
            r(self.size.x),
            r(self.size.y),
            r(self.size.z),
            self.cost,
            self.requires_power,
            self.slide,
        )
    }
}

impl crate::windows::Window {
    /// As RON, on one line, rounded like a [`Placement`].
    fn to_ron(self) -> String {
        let r = |v: f32| ((v as f64 * 1000.0).round() / 1000.0) as f32;
        format!(
            "(at: {}, size: ({:?}, {:?}), sill: {:?}, depth: {:?})",
            self.at.with_scale(1.0).to_ron(),
            r(self.size.x),
            r(self.size.y),
            r(self.sill),
            r(self.depth),
        )
    }
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
    /// The ammo crates — one per zone, say — up to [`MAX_AMMO_CRATES`].
    #[serde(default)]
    pub ammo_crates: Vec<Placement>,
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
    /// The crafting table ([`crate::crafting`]), if the place has one.
    #[serde(default)]
    pub crafting_table: Option<Placement>,
    /// The exfil radio (`crate::exfil`), if the place has one — exfil can
    /// only be called on a place with both it and [`Self::exfil_area`].
    #[serde(default)]
    pub exfil_radio: Option<Placement>,
    /// The area the party has to hold during an exfil.
    #[serde(default)]
    pub exfil_area: Option<ExfilArea>,
    /// The Rampage Inducer ([`crate::rampage`]), if the place has one.
    #[serde(default)]
    pub rampage_inducer: Option<Placement>,
    /// The power station ([`crate::power_station`]) — scenery the power
    /// lever's put on — if the place has one.
    #[serde(default)]
    pub power_station: Option<Placement>,
    /// The lights the power turns on — up to [`MAX_POWER_LIGHTS`].
    #[serde(default)]
    pub power_lights: Vec<PowerLight>,
    /// The fires burning on the map, power or no power — up to
    /// [`MAX_FLAMES`].
    #[serde(default)]
    pub flames: Vec<Flame>,
    /// The pools of water on the map — up to [`MAX_WATERS`].
    #[serde(default)]
    pub waters: Vec<Water>,
    /// The waterfalls pouring into them — up to [`MAX_WATERFALLS`].
    #[serde(default)]
    pub waterfalls: Vec<Waterfall>,
    /// The doors in the map's doorways ([`crate::doors`]) — up to
    /// [`crate::doors::MAX_DOORS`].
    #[serde(default)]
    pub doors: Vec<crate::doors::Door>,
    /// The windows zombies climb in through ([`crate::windows`]) — up to
    /// [`crate::windows::MAX_WINDOWS`].
    #[serde(default)]
    pub windows: Vec<crate::windows::Window>,
    /// Where the players start a game: each is put on a different one, at
    /// random (`crate::spawns::zombies_start`) — up to
    /// [`MAX_PLAYER_SPAWNS`], one per player a lobby can hold. None, and
    /// they start where the other modes would put them.
    #[serde(default)]
    pub player_spawns: Vec<Placement>,
}

/// Most ammo crates a layout keeps.
pub const MAX_AMMO_CRATES: usize = 8;

/// Most player start spots a layout keeps — as many players as a lobby
/// holds.
pub const MAX_PLAYER_SPAWNS: usize = 8;

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
        out.extend(self.ammo_crates.iter_mut().map(|at| ("ammo_crate".to_string(), at)));
        for (key, slot) in [
            ("pack_a_punch", &mut self.pack_a_punch),
            ("power_switch", &mut self.power_switch),
            ("mystery_box", &mut self.mystery_box),
            ("armor_station", &mut self.armor_station),
            ("exfil_radio", &mut self.exfil_radio),
            ("crafting_table", &mut self.crafting_table),
            ("rampage_inducer", &mut self.rampage_inducer),
            ("power_station", &mut self.power_station),
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
        out += "    ammo_crates: [\n";
        for at in &self.ammo_crates {
            out += &format!("        {},\n", at.to_ron());
        }
        out += "    ],\n";
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
        out += &format!("    crafting_table: {},\n", optional(self.crafting_table));
        out += &format!("    exfil_radio: {},\n", optional(self.exfil_radio));
        out += &format!(
            "    exfil_area: {},\n",
            self.exfil_area.map_or("None".to_string(), |a| format!("Some({})", a.to_ron()))
        );
        out += &format!("    rampage_inducer: {},\n", optional(self.rampage_inducer));
        out += &format!("    power_station: {},\n", optional(self.power_station));
        out += "    power_lights: [\n";
        for light in &self.power_lights {
            out += &format!("        {},\n", light.to_ron());
        }
        out += "    ],\n";
        out += "    flames: [\n";
        for flame in &self.flames {
            out += &format!("        {},\n", flame.to_ron());
        }
        out += "    ],\n";
        out += "    waters: [\n";
        for water in &self.waters {
            out += &format!("        {},\n", water.to_ron());
        }
        out += "    ],\n";
        out += "    waterfalls: [\n";
        for fall in &self.waterfalls {
            out += &format!("        {},\n", fall.to_ron());
        }
        out += "    ],\n";
        out += "    doors: [\n";
        for door in &self.doors {
            out += &format!("        {},\n", door.to_ron());
        }
        out += "    ],\n";
        out += "    windows: [\n";
        for window in &self.windows {
            out += &format!("        {},\n", window.to_ron());
        }
        out += "    ],\n";
        out += "    player_spawns: [\n";
        for at in &self.player_spawns {
            out += &format!("        {},\n", at.with_scale(1.0).to_ron());
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
        MapId::BasicMapNight => MapId::BasicMap,
        MapId::DerTempelDay => MapId::DerTempel,
        other => other,
    }
}

/// The layout file's name for `map`'s place.
pub fn file_name(map: MapId) -> &'static str {
    match place(map) {
        MapId::Shipment | MapId::ShipmentDay => "shipment.ron",
        MapId::BreakPoint | MapId::BreakPointNight => "break_point.ron",
        MapId::DerTempel | MapId::DerTempelDay => "der_tempel.ron",
        MapId::BasicMap | MapId::BasicMapNight => "basic_map.ron",
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
        MapId::DerTempel | MapId::DerTempelDay => include_str!("../levels/der_tempel.ron"),
        MapId::BasicMap | MapId::BasicMapNight => include_str!("../levels/basic_map.ron"),
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
    fn several_ammo_crates_survive_a_save_and_load() {
        let mut layout = layout(MapId::BreakPoint).clone();
        assert_eq!(layout.ammo_crates.len(), 1, "Break Point keeps its crate");
        layout.ammo_crates.push(Placement::new(Vec3::new(3.0, 0.0, -7.5), 90.0));
        layout.ammo_crates.push(Placement::new(Vec3::new(-12.25, 1.5, 4.0), -45.0));
        let back = ZombiesLayout::from_ron(&layout.to_ron()).unwrap().sized(sizes());
        assert_eq!(back.ammo_crates.len(), 3);
        for (a, b) in layout.ammo_crates.iter().zip(&back.ammo_crates) {
            assert!(a.pos.distance(b.pos) < 1e-3 && (a.yaw_deg - b.yaw_deg).abs() < 1e-3);
        }
        assert_eq!(crate::ammo::crate_boxes(MapId::BreakPoint).count(), 1);
    }

    #[test]
    fn power_lights_survive_a_save_and_load() {
        let mut layout = layout(MapId::BreakPoint).clone();
        assert!(!layout.power_lights.is_empty(), "Break Point keeps its power light");
        let mut light = PowerLight::new(Vec3::new(1.5, 6.25, -3.0));
        light.color = [0.9, 0.5, 0.25];
        light.shadows = true;
        layout.power_lights.push(light);
        let mut spot = PowerLight::new(Vec3::new(-16.0, 18.5, 0.0));
        spot.spot = Some(SpotCone {
            yaw_deg: -75.0,
            pitch_deg: -35.0,
            inner_angle_deg: 89.0,
            outer_angle_deg: 89.0,
        });
        spot.glow = 5_000_000.0;
        layout.power_lights.push(spot);
        let back = ZombiesLayout::from_ron(&layout.to_ron()).unwrap();
        assert_eq!(back.power_lights.last(), Some(&spot));
        assert_eq!(back.power_lights[back.power_lights.len() - 2], light);
        assert_eq!(back.power_lights.len(), layout.power_lights.len());
    }

    #[test]
    fn doors_survive_a_save_and_load() {
        let mut layout = layout(MapId::DerTempel).clone();
        let mut door = crate::doors::Door::new(Placement::new(Vec3::new(-4.5, 0.0, 22.25), -90.0));
        door.size = Vec3::new(5.5, 6.0, 0.75);
        door.cost = 1250;
        door.requires_power = true;
        door.slide = crate::doors::Slide::Up;
        door.main = true;
        layout.doors.push(door);
        layout.doors.push(crate::doors::Door::new(Placement::new(Vec3::new(3.0, -4.0, 1.0), 0.0)));
        let back = ZombiesLayout::from_ron(&layout.to_ron()).unwrap();
        assert_eq!(back.doors, layout.doors);
    }

    #[test]
    fn windows_survive_a_save_and_load() {
        let mut layout = layout(MapId::DerTempel).clone();
        let mut window = crate::windows::Window::new(Placement::new(Vec3::new(-20.5, 0.0, 33.25), 90.0));
        window.size = bevy::math::Vec2::new(1.5, 1.25);
        window.sill = 1.1;
        window.depth = 0.75;
        layout.windows.push(window);
        layout.windows.push(crate::windows::Window::new(Placement::new(Vec3::new(3.0, -4.0, 1.0), -45.0)));
        let back = ZombiesLayout::from_ron(&layout.to_ron()).unwrap();
        assert_eq!(back.windows, layout.windows);
    }

    #[test]
    fn flames_survive_a_save_and_load() {
        let mut layout = layout(MapId::DerTempel).clone();
        let mut flame = Flame::new(Vec3::new(-12.5, 3.25, 40.0));
        flame.size = 0.6;
        flame.light = 85_000.0;
        flame.flicker = 0.5;
        layout.flames.push(flame);
        layout.flames.push(Flame::new(Vec3::new(4.0, 2.0, -7.5)));
        let back = ZombiesLayout::from_ron(&layout.to_ron()).unwrap();
        assert_eq!(back.flames, layout.flames);
    }

    #[test]
    fn water_and_waterfalls_survive_a_save_and_load() {
        let mut layout = layout(MapId::DerTempel).clone();
        let mut water = Water::new(Placement::new(Vec3::new(-10.0, 0.4, 22.5), 30.0));
        water.size = Vec2::new(8.5, 12.25);
        water.color = [0.2, 0.5, 0.45];
        water.waves = 0.8;
        layout.waters.push(water);
        let mut fall = Waterfall::new(Placement::new(Vec3::new(-10.0, 3.5, 17.0), 0.0));
        fall.width = 2.0;
        fall.drop = 3.1;
        fall.spray = 1.5;
        layout.waterfalls.push(fall);
        let back = ZombiesLayout::from_ron(&layout.to_ron()).unwrap();
        assert_eq!(back.waters, layout.waters);
        assert_eq!(back.waterfalls, layout.waterfalls);
    }

    #[test]
    fn wading_is_only_in_the_water() {
        let water = Water::new(Placement::new(Vec3::new(10.0, 1.0, 0.0), 0.0));
        // Ankle-deep, in the middle and near an edge.
        assert!(water.wading(Vec3::new(10.0, 0.75, 0.0)));
        assert!(water.wading(Vec3::new(12.9, 0.75, -2.9)));
        // Beside it, above it, and on a floor well below it.
        assert!(!water.wading(Vec3::new(13.5, 0.75, 0.0)));
        assert!(!water.wading(Vec3::new(10.0, 1.2, 0.0)));
        assert!(!water.wading(Vec3::new(10.0, -2.0, 0.0)));
    }

    #[test]
    fn a_waterfall_lands_out_from_its_wall() {
        let fall = Waterfall::new(Placement::new(Vec3::new(0.0, 4.0, 0.0), 90.0));
        let foot = fall.foot();
        assert!((foot.y - (4.0 - fall.drop)).abs() < 1e-4);
        // (Turned to face +X, it lands out along +X.)
        assert!(foot.x > 0.1 && foot.z.abs() < 1e-4);
        let water = Water::new(Placement::new(Vec3::new(foot.x, 1.0, 0.0), 0.0));
        assert!(water.covers(foot));
        assert!(!water.covers(Vec3::new(10.0, 1.0, 0.0)));
    }

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

