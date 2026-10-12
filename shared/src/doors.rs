//! `Zombies` doors: sliding doors in a map's doorways, bought open with
//! points (Call of Duty zombies' debris and doors). Each map's are in its
//! layout ([`crate::level::ZombiesLayout::doors`], placed in the level
//! editor); which are open in a game is [`crate::Lobby::doors_open`], a bit
//! per door, owned by the server (`server::zombies::on_buy_door`).
//!
//! A closed door is solid — to players (the client's collider), to zombies
//! and everything thrown (the server's `collision::LobbyWorld`) — and zombies
//! neither path through it nor come up beyond it: the server splits the map's
//! walkable ground into zones at the doors and only spawns in the ones the
//! party can reach (`server::nav`). Doors are only shut in `Zombies`; in the
//! other modes every doorway's open.

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::level::Placement;
use crate::MapId;

/// `models/props/door.glb`'s size as made (m): `(width, height, depth)` once
/// it's turned to face +Z (its width runs along its own z as modelled).
pub const MODEL_SIZE: Vec3 = Vec3::new(2.0, 3.0, 0.5);

/// Points a new door costs.
pub const DEFAULT_COST: u32 = 1000;

/// Most doors a map keeps (one bit each in [`crate::Lobby::doors_open`]).
pub const MAX_DOORS: usize = 32;

/// How far (m) in front of or behind a door's face a player can buy it
/// from, and past its edges.
pub const USE_REACH: f32 = 1.5;

/// Seconds a door takes to slide open.
pub const OPEN_SECS: f32 = 1.2;

/// Which way a door slides out of its doorway as it opens.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Slide {
    /// Down into the floor.
    #[default]
    Down,
    /// Up into the ceiling.
    Up,
    /// Sideways — to the left or right as seen looking at its front.
    Left,
    Right,
}

impl Slide {
    pub const ALL: [Slide; 4] = [Slide::Down, Slide::Up, Slide::Left, Slide::Right];

    pub fn label(self) -> &'static str {
        match self {
            Slide::Down => "Down",
            Slide::Up => "Up",
            Slide::Left => "Left",
            Slide::Right => "Right",
        }
    }
}

/// One door: standing in a doorway at `at` (the floor under its middle,
/// facing the way it's turned), `size` metres `(width, height, depth)`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Door {
    pub at: Placement,
    pub size: Vec3,
    /// Points it costs to open.
    pub cost: u32,
    /// Whether it can only be bought once the power's on.
    #[serde(default)]
    pub requires_power: bool,
    #[serde(default)]
    pub slide: Slide,
    /// The map's main door — opening it plays the main-door ambience from
    /// it, heard well beyond it (the client's `doors`).
    #[serde(default)]
    pub main: bool,
}

impl Door {
    /// A fresh one at `at`, the model's own size.
    pub fn new(at: Placement) -> Self {
        Self {
            at: at.with_scale(1.0),
            size: MODEL_SIZE,
            cost: DEFAULT_COST,
            requires_power: false,
            slide: Slide::default(),
            main: false,
        }
    }

    pub fn rotation(&self) -> Quat {
        self.at.rotation()
    }

    /// Its solid box: `(centre, rotation, half extents)`.
    pub fn solid_box(&self) -> (Vec3, Quat, Vec3) {
        let half = self.size * 0.5;
        (self.at.pos + Vec3::Y * half.y, self.rotation(), half)
    }

    /// `p` in the door's own frame (x across it, y up from its floor, z out
    /// of its front).
    pub fn local(&self, p: Vec3) -> Vec3 {
        self.rotation().inverse() * (p - self.at.pos)
    }

    /// Whether feet position `feet` is close enough to buy it, from either
    /// side. `slack` widens it (the server allows for the client's pose
    /// being a moment old).
    pub fn in_range(&self, feet: Vec3, slack: f32) -> bool {
        let l = self.local(feet);
        l.x.abs() <= self.size.x * 0.5 + slack
            && l.z.abs() <= self.size.z * 0.5 + USE_REACH + slack
            && l.y >= -1.0 - slack
            && l.y <= self.size.y + slack
    }

    /// How far (in its own frame) it's slid, `t` (0..1) of the way open —
    /// all the way out of its doorway, and a little more.
    pub fn slide_offset(&self, t: f32) -> Vec3 {
        let t = t.clamp(0.0, 1.0);
        match self.slide {
            Slide::Down => Vec3::NEG_Y * (self.size.y + 0.05) * t,
            Slide::Up => Vec3::Y * (self.size.y + 0.05) * t,
            // (Looking at its front — from +z, back along −z — left is −x.)
            Slide::Left => Vec3::NEG_X * (self.size.x + 0.05) * t,
            Slide::Right => Vec3::X * (self.size.x + 0.05) * t,
        }
    }
}

/// `map`'s doors, from its layout.
pub fn doors(map: MapId) -> &'static [Door] {
    &crate::level::layout(map).doors
}

/// Whether door `index` is open in `doors_open` ([`crate::Lobby::doors_open`]).
pub fn is_open(doors_open: u32, index: usize) -> bool {
    index < MAX_DOORS && doors_open & (1 << index) != 0
}

/// The bits of the doors still shut on `map`, given `doors_open`.
pub fn closed_mask(map: MapId, doors_open: u32) -> u32 {
    let all = match doors(map).len().min(MAX_DOORS) {
        32 => u32::MAX,
        n => (1u32 << n) - 1,
    };
    all & !doors_open
}

/// Why door `index` on `map` can't be bought now, if it can't: already
/// open, or waiting on the power.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locked {
    Open,
    NeedsPower,
}

/// Whether door `index` on `map` can be bought, with the game's power and
/// open doors as they are.
pub fn can_open(map: MapId, index: usize, doors_open: u32, power_on: bool) -> Result<&'static Door, Locked> {
    let door = doors(map).get(index).ok_or(Locked::Open)?;
    if is_open(doors_open, index) {
        return Err(Locked::Open);
    }
    if door.requires_power && !crate::power::has_power(map, power_on) {
        return Err(Locked::NeedsPower);
    }
    Ok(door)
}

/// The shut door on `map` that feet position `feet` can buy, if any — the
/// nearest when it's between two.
pub fn door_at(map: MapId, feet: Vec3, doors_open: u32, slack: f32) -> Option<usize> {
    doors(map)
        .iter()
        .enumerate()
        .filter(|&(i, d)| !is_open(doors_open, i) && d.in_range(feet, slack))
        .min_by(|a, b| a.1.local(feet).z.abs().total_cmp(&b.1.local(feet).z.abs()))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_door_is_bought_from_either_side_but_not_from_across_the_room() {
        let door = Door::new(Placement::new(Vec3::new(10.0, 0.0, 5.0), 90.0));
        // Turned 90°: its face looks along +x, so in front / behind is ±x.
        assert!(door.in_range(Vec3::new(11.5, 0.0, 5.0), 0.0));
        assert!(door.in_range(Vec3::new(8.6, 0.0, 5.5), 0.0));
        assert!(!door.in_range(Vec3::new(14.0, 0.0, 5.0), 0.0));
        assert!(!door.in_range(Vec3::new(10.0, 0.0, 8.0), 0.0), "past its edge");
        assert!(!door.in_range(Vec3::new(10.0, 6.0, 5.0), 0.0), "a floor above");
    }

    #[test]
    fn a_door_slides_all_the_way_out_of_its_doorway() {
        let mut door = Door::new(Placement::default());
        assert_eq!(door.slide_offset(0.0), Vec3::ZERO);
        assert!(door.slide_offset(1.0).y <= -door.size.y);
        door.slide = Slide::Right;
        assert!(door.slide_offset(1.0).x >= door.size.x);
    }

    #[test]
    fn open_bits_are_per_door() {
        assert!(is_open(0b100, 2));
        assert!(!is_open(0b100, 1));
        assert!(!is_open(u32::MAX, MAX_DOORS));
    }
}
