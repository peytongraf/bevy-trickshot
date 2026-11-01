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

    /// Whether he's loose — from the ritual going wrong until he's dead: the
    /// map's changed (green fires, red water, dark sky, thick fog,
    /// flickering lights) and the round's own zombies are held back
    /// ([`holds_spawns`]).
    pub fn is_unleashed(self) -> bool {
        matches!(self, EggStep::Unleashed | EggStep::Awakening | EggStep::BossFight)
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
    /// The Hierophant in this fight ([`EggStep::BossFight`]) — set by the
    /// server once he's in, and kept (as who he was) once he's dead, until
    /// the egg's set back before the fight.
    #[serde(default)]
    pub hierophant: Option<Hierophant>,
}

/// Who the Hierophant is in a game, and the health he came in with (for his
/// health bar).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hierophant {
    pub peer: PeerId,
    pub max_health: f32,
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

/// How far (s) into the ritual the Hierophant starts on his line from the
/// sarcophagus (`during_ritual.mp3`), and how long it is.
pub const RITUAL_VOICE_AT_SECS: f32 = 13.0;
pub const RITUAL_VOICE_SECS: f32 = 5.64;

/// Once it's gone wrong ([`EggStep::Unleashed`], counted without pauses):
/// every zombie up drops dead this long after (just after the change, so
/// every client already knows why — no death cries)...
pub const UNLEASHED_ZOMBIES_DROP_SECS: f32 = 0.2;
/// ...the sarcophagus speaks this long after (`ritual_gone_wrong.mp3`, once
/// his ritual line has finished, and a beat)...
pub const UNLEASHED_VOICE_SECS: f32 = RITUAL_VOICE_AT_SECS + RITUAL_VOICE_SECS + 0.6 - RITUAL_SECS;
/// ...which runs this long...
pub const UNLEASHED_VOICE_LEN_SECS: f32 = 8.44;
/// ...and the man on the radio follows once he's done
/// (`RadioLine::EggRitualGoesWrong`).
pub const UNLEASHED_RADIO_SECS: f32 = UNLEASHED_VOICE_SECS + UNLEASHED_VOICE_LEN_SECS + 0.5;

/// How long (s, not counting pauses) it stays gone wrong before he wakes
/// ([`EggStep::Awakening`]): the man on the radio's said his piece, and a
/// breath after.
pub fn unleashed_secs() -> f32 {
    UNLEASHED_RADIO_SECS + crate::radio::RadioLine::EggRitualGoesWrong.airtime() + 2.5
}

// The wake-up ([`EggStep::Awakening`]), in s from its start:
/// the sarcophagus' lid blasts off, flying this long before it lands...
pub const WAKE_LID_SECS: f32 = 1.1;
/// ...his spirit pours down into the body from then till then...
pub const WAKE_SPIRIT_FROM: f32 = 1.2;
pub const WAKE_SPIRIT_TO: f32 = 4.2;
/// ...a beat of stillness, then he snaps upright, screaming (`wake.mp3`,
/// this long)...
pub const WAKE_SNAP_AT: f32 = 6.2;
pub const WAKE_SCREAM_SECS: f32 = 5.86;
/// ...and once he's done, the fight's on ([`EggStep::BossFight`]).
pub const AWAKENING_SECS: f32 = WAKE_SNAP_AT + WAKE_SCREAM_SECS + 0.5;

// --- the fight ([`EggStep::BossFight`]) ------------------------------------------------

/// Where his feet are once he's snapped upright, in the sarcophagus' own
/// frame (unscaled): on top of its chest — the fight's Hierophant starts
/// there, and steps down off it.
pub const HIEROPHANT_FEET: Vec3 = Vec3::new(0.0, 0.88, 0.85);

/// Where the Hierophant stands up on `map`, and the yaw he faces (the way
/// the sarcophagus does — `+z` in its frame).
pub fn hierophant_start(map: MapId) -> Option<(Vec3, f32)> {
    let at = crate::level::layout(map).sarcophagus?;
    let feet = at.pos + at.rotation() * (HIEROPHANT_FEET * at.scale);
    let facing = at.rotation() * Vec3::Z;
    Some((feet, f32::atan2(-facing.x, -facing.z)))
}

/// His health in `round` with `players` in the game: three of that round's
/// bosses' (`crate::boss::boss_health`).
pub fn hierophant_health(round: u32, players: usize) -> f32 {
    crate::boss::boss_health(round.max(1), players) * HIEROPHANT_HEALTH_MULT
}
pub const HIEROPHANT_HEALTH_MULT: f32 = 3.0;

/// He walks a little quicker than a boss (`crate::boss::BOSS_SPEED`), m/s...
pub const HIEROPHANT_SPEED: f32 = 1.95;
/// ...hits harder (before perks — a boss's is `crate::boss::MELEE_DAMAGE`),
/// and never hurls a blast: he's a melee stalker.
pub const HIEROPHANT_MELEE_DAMAGE: f32 = 85.0;

/// His taunts (`hierophant/taunt_1.mp3`…): how many there are, the first
/// this long (s, not counting pauses) into the fight, then one every so
/// often (a random gap in this range) — each time he stops where he is
/// for this long, glaring at who he's after, while he says it.
pub const TAUNTS: u8 = 6;
pub const TAUNT_FIRST_SECS: f32 = 6.0;
pub const TAUNT_GAP_SECS: (f32, f32) = (14.0, 24.0);
pub const TAUNT_HOLD_SECS: f32 = 2.0;

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

    /// Whether `peer` is (or was) this game's Hierophant.
    pub fn is_hierophant(&self, peer: PeerId) -> bool {
        self.hierophant.is_some_and(|h| h.peer == peer)
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

/// Whether `lobby`'s round is on hold for the easter egg: no zombie, dog or
/// boss of its own comes (and its clocks wait) while he's loose
/// ([`EggStep::is_unleashed`]) — the round carries on where it was after.
pub fn holds_spawns(lobby: &Lobby) -> bool {
    available(lobby) && lobby.easter_egg.step.is_unleashed()
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
    fn he_stands_up_on_the_sarcophagus() {
        let at = crate::level::layout(MapId::DerTempel).sarcophagus.expect("Der Tempel has its sarcophagus placed");
        let (feet, _) = hierophant_start(MapId::DerTempel).unwrap();
        let d = feet - at.pos;
        assert!(Vec3::new(d.x, 0.0, d.z).length() <= SARCOPHAGUS_HALF_EXTENTS.z * at.scale, "on it, not off its end");
        assert!(d.y > 0.0 && d.y <= SARCOPHAGUS_HALF_EXTENTS.y * 2.0 * at.scale, "on top of it");
        assert!(hierophant_start(MapId::Shipment).is_none());
    }

    #[test]
    fn hes_tougher_than_a_boss() {
        for players in 1..=4 {
            assert!(hierophant_health(10, players) > crate::boss::boss_health(10, players));
        }
        assert!(HIEROPHANT_MELEE_DAMAGE > crate::boss::MELEE_DAMAGE);
    }

    #[test]
    fn only_der_tempel_has_it() {
        assert!(on_map(MapId::DerTempel));
        assert!(on_map(MapId::DerTempelDay));
        assert!(!on_map(MapId::Shipment));
    }
}
