//! Der Tempel's easter egg — the plan's in
//! `client/notes/der-tempel-easter-egg/` (`story.md` for what happens,
//! `todo.md` for what's built).
//!
//! How far the squad's got ([`EasterEgg`]) is the server's, kept on the lobby
//! (`Lobby::easter_egg`) so every client sees it; it's reset whenever a game
//! starts or ends. Where its pieces stand — the sarcophagus, the society's
//! case, the altar's ritual area and, inside the pyramid, its altar and the
//! spots the squad's souls arrive on — is the map's layout
//! ([`crate::level::ZombiesLayout`]), placed in the level editor.

use bevy::math::{Quat, Vec3};
use lightyear::prelude::PeerId;
use serde::{Deserialize, Serialize};

use crate::level::Placement;
use crate::{GameMode, Lobby, MapId};

/// How far the squad's got, in order (see `story.md`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EggStep {
    /// Gathering Barrett's blood and the Hierophant's dagger
    /// ([`EasterEgg::vial`], [`EasterEgg::dagger`]) — then all of them at the
    /// altar, accepting together.
    #[default]
    Gathering,
    /// The ritual's playing.
    Ritual,
    /// It's gone wrong: the map changes, the dead drop, the sarcophagus
    /// speaks.
    Unleashed,
    /// The Hierophant's waking up in his body.
    Awakening,
    /// The fight.
    BossFight,
    /// He's dead; the squad's souls go up into the pyramid.
    Ascension,
    /// They're back with the wonder weapon — over for this game.
    Complete,
}

impl EggStep {
    pub const ALL: [EggStep; 7] = [
        EggStep::Gathering,
        EggStep::Ritual,
        EggStep::Unleashed,
        EggStep::Awakening,
        EggStep::BossFight,
        EggStep::Ascension,
        EggStep::Complete,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EggStep::Gathering => "Gathering",
            EggStep::Ritual => "Ritual",
            EggStep::Unleashed => "It goes wrong",
            EggStep::Awakening => "He wakes",
            EggStep::BossFight => "Boss fight",
            EggStep::Ascension => "Into the pyramid",
            EggStep::Complete => "Complete",
        }
    }
}

/// The easter egg's progress in a game.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EasterEgg {
    pub step: EggStep,
    /// The squad has Barrett's blood (the first Big Steve's drop).
    pub vial: bool,
    /// The squad has the Hierophant's dagger (from the society's case).
    pub dagger: bool,
    /// Barrett's blood lying where a Big Steve fell, waiting to be picked up
    /// ([`EggAction::PickUpVial`]) — the first one killed while it's still
    /// to be found drops it.
    #[serde(default)]
    pub vial_at: Option<Vec3>,
    /// At the altar, both things gathered: who's accepted the ritual so far
    /// ([`EggAction::Accept`]) — once every living player has, standing in
    /// the ritual area, it starts. Anyone stepping out or declining clears
    /// it.
    #[serde(default)]
    pub accepted: Vec<PeerId>,
}

/// What a player does with the easter egg ([`crate::EggInteract`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EggAction {
    /// Pick up Barrett's blood where it lies.
    PickUpVial,
    /// Take the dagger out of the society's case (open once the power's on).
    TakeDagger,
    /// At the altar: accept the ritual...
    Accept,
    /// ...or back out of it (everyone's acceptance goes).
    Decline,
}

/// How long (s) the ritual takes ([`EggStep::Ritual`]): the players locked
/// in place, the zombies holding off — then it goes wrong
/// ([`EggStep::Unleashed`]).
pub const RITUAL_SECS: f32 = 15.0;

/// How close (m, across the ground) feet have to be to the vial to pick it
/// up...
pub const VIAL_REACH: f32 = 1.6;
/// ...and to the society's case to take the dagger (from its middle, at
/// scale 1 — a bigger case reaches further).
pub const CASE_REACH: f32 = 1.8;
/// How far above or below a thing (m) feet can be and still reach it.
pub const REACH_HEIGHT: f32 = 2.0;

/// Whether feet at `feet` reach something on the ground at `at` within
/// `reach` (plus `slack` — the server allows for the pose being a moment
/// old).
pub fn within(at: Vec3, feet: Vec3, reach: f32, slack: f32) -> bool {
    let d = feet - at;
    Vec3::new(d.x, 0.0, d.z).length() <= reach + slack && d.y.abs() <= REACH_HEIGHT + slack
}

/// Whether feet at `feet` can take the dagger from `map`'s case.
pub fn at_case(map: MapId, feet: Vec3, slack: f32) -> bool {
    crate::level::layout(map)
        .society_case
        .is_some_and(|at| within(at.pos, feet, CASE_REACH + SOCIETY_CASE_HALF_EXTENTS.z * (at.scale - 1.0).max(0.0), slack))
}

/// Whether feet at `feet` are in `map`'s ritual area (standing on it — not
/// on a floor far above or below).
pub fn in_ritual_area(map: MapId, feet: Vec3) -> bool {
    crate::level::layout(map)
        .ritual_area
        .is_some_and(|area| area.contains(feet) && (feet.y - area.at.pos.y).abs() <= REACH_HEIGHT)
}

/// Whether the society's case is open in `lobby`'s game — once the power's
/// on (always, on a map without a switch).
pub fn case_open(lobby: &Lobby) -> bool {
    crate::power::has_power(lobby.map, lobby.power_on)
}

impl EasterEgg {
    /// Both things gathered — the altar's ready for them.
    pub fn gathered(&self) -> bool {
        self.vial && self.dagger
    }

    /// As it would be at `step`: anything past gathering has both things
    /// (the debug panel's jumps).
    pub fn at(step: EggStep) -> Self {
        let past = step > EggStep::Gathering;
        Self {
            step,
            vial: past,
            dagger: past,
            ..Default::default()
        }
    }

    /// Kept consistent: past gathering, both things are had; a vial that's
    /// had isn't lying anywhere; only gathering with both has anyone
    /// accepting.
    pub fn normalized(mut self) -> Self {
        if self.step > EggStep::Gathering {
            self.vial = true;
            self.dagger = true;
        }
        if self.vial {
            self.vial_at = None;
        }
        if self.step != EggStep::Gathering || !self.gathered() {
            self.accepted.clear();
        }
        self
    }

    /// Whether the squad's at the altar step: both things gathered, the
    /// ritual not yet begun.
    pub fn at_altar(&self) -> bool {
        self.step == EggStep::Gathering && self.gathered()
    }
}

/// Whether `map` has the easter egg — Der Tempel (night or day).
pub fn on_map(map: MapId) -> bool {
    crate::level::place(map) == MapId::DerTempel
}

/// Whether `lobby`'s running a game the easter egg's part of: `Zombies` on
/// its map.
pub fn available(lobby: &Lobby) -> bool {
    lobby.started && lobby.mode == GameMode::Zombies && on_map(lobby.map)
}

// --- its pieces (placeholders for now — sizes as the models will be made) ---------

/// The sarcophagus on the upper altar: about 2.2 m long (along its own z),
/// 0.9 m wide and 1 m tall, standing on the ground — half extents.
pub const SARCOPHAGUS_HALF_EXTENTS: Vec3 = Vec3::new(0.45, 0.5, 1.1);
/// The society's locked case (the dagger's): a display case on a stand,
/// 0.8 m wide, 1.3 m tall, 0.5 m deep — its front facing the way it's turned.
pub const SOCIETY_CASE_HALF_EXTENTS: Vec3 = Vec3::new(0.4, 0.65, 0.25);
/// The altar inside the pyramid (where the wonder weapon lies): 1.8 m wide,
/// 1 m tall, 1 m deep.
pub const PYRAMID_ALTAR_HALF_EXTENTS: Vec3 = Vec3::new(0.9, 0.5, 0.5);
/// A fresh ritual area's size (m): the top of the altar, roughly.
pub const DEFAULT_RITUAL_AREA_SIZE: (f32, f32) = (5.0, 5.0);

/// A solid box from a standing placement and its half extents as made.
fn standing_box(at: Placement, half: Vec3) -> (Vec3, Quat, Vec3) {
    let half = half * at.scale;
    (at.pos + Vec3::Y * half.y, at.rotation(), half)
}

/// The sarcophagus' and case's solid boxes on `map` (and the pyramid altar's),
/// for what it has: `(centre, rotation, half extents)`.
pub fn solid_boxes(map: MapId) -> Vec<(Vec3, Quat, Vec3)> {
    let layout = crate::level::layout(map);
    [
        layout.sarcophagus.map(|at| standing_box(at, SARCOPHAGUS_HALF_EXTENTS)),
        layout.society_case.map(|at| standing_box(at, SOCIETY_CASE_HALF_EXTENTS)),
        layout.pyramid_altar.map(|at| standing_box(at, PYRAMID_ALTAR_HALF_EXTENTS)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_in_order() {
        for pair in EggStep::ALL.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn past_gathering_has_both_things() {
        assert!(!EasterEgg::at(EggStep::Gathering).gathered());
        for step in &EggStep::ALL[1..] {
            assert!(EasterEgg::at(*step).gathered());
            let partial = EasterEgg { step: *step, vial: true, ..Default::default() };
            assert!(partial.normalized().gathered());
        }
        let gathering = EasterEgg { step: EggStep::Gathering, vial: true, ..Default::default() };
        assert_eq!(gathering.clone().normalized(), gathering);
    }

    #[test]
    fn a_had_vial_isnt_lying_anywhere_and_only_the_altar_has_acceptances() {
        let peer = PeerId::Netcode(7);
        let egg = EasterEgg {
            vial: true,
            vial_at: Some(Vec3::ONE),
            accepted: vec![peer],
            ..Default::default()
        }
        .normalized();
        assert_eq!(egg.vial_at, None);
        assert!(egg.accepted.is_empty(), "no dagger yet — not at the altar");
        let ready = EasterEgg { vial: true, dagger: true, accepted: vec![peer], ..Default::default() }.normalized();
        assert!(ready.at_altar());
        assert_eq!(ready.accepted, vec![peer]);
    }

    #[test]
    fn reach_is_across_the_ground_and_a_little_up_or_down() {
        let at = Vec3::new(2.0, 0.0, 2.0);
        assert!(within(at, Vec3::new(3.0, 0.0, 2.5), VIAL_REACH, 0.0));
        assert!(!within(at, Vec3::new(4.0, 0.0, 2.0), VIAL_REACH, 0.0));
        assert!(!within(at, Vec3::new(2.0, 5.0, 2.0), VIAL_REACH, 0.0), "a floor up");
    }

    #[test]
    fn only_der_tempel_has_it() {
        assert!(on_map(MapId::DerTempel));
        assert!(on_map(MapId::DerTempelDay));
        assert!(!on_map(MapId::Shipment));
    }
}
