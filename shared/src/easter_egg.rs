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
    /// He's dead: his body falls and his spirit flees screaming up into the
    /// pyramid ([`DEATH_SECS`]) — the map still gone wrong and the round
    /// still held until it's in.
    HisDeath,
    /// He's dead; the squad's souls go up into the pyramid.
    Ascension,
    /// They're back with the wonder weapon — over for this game.
    Complete,
}

impl EggStep {
    pub const ALL: [EggStep; 8] = [
        EggStep::Gathering,
        EggStep::Ritual,
        EggStep::Unleashed,
        EggStep::Awakening,
        EggStep::BossFight,
        EggStep::HisDeath,
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
            EggStep::HisDeath => "His death",
            EggStep::Ascension => "Into the pyramid",
            EggStep::Complete => "Complete",
        }
    }

    /// Whether he's loose — from the ritual going wrong until he's dead: the
    /// map's changed (green fires, red water, dark sky, thick fog,
    /// flickering lights) and the round's own zombies are held back
    /// ([`holds_spawns`] — which holds them through [`EggStep::HisDeath`] too).
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

/// Who the Hierophant is in a game, the health he came in with (for his
/// health bar), and which phase of the fight he's in (1; 2 from
/// [`PHASE_2_AT`] of his health; 3, enraged, from [`PHASE_3_AT`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hierophant {
    pub peer: PeerId,
    pub max_health: f32,
    #[serde(default = "first_phase")]
    pub phase: u8,
    /// Where his feet were as he fell — his spirit tears out of him there
    /// ([`EggStep::HisDeath`]).
    #[serde(default)]
    pub fell_at: Option<Vec3>,
}

fn first_phase() -> u8 {
    1
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
pub const WAKE_SCREAM_SECS: f32 = 8.28;
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
pub const TAUNT_GAP_SECS: (f32, f32) = (16.0, 26.0);
pub const TAUNT_HOLD_SECS: f32 = 2.5;

/// What the Hierophant says in the fight (`hierophant/<file>.mp3`, see
/// `client/notes/quotes/hierophant-quotes.md`) — the server picks, every
/// client plays it from where he stands ([`crate::HierophantSays`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierophantLine {
    /// `0..TAUNTS`.
    Taunt(u8),
    /// He's downed a player — `0..PLAYER_DOWN_LINES`.
    PlayerDown(u8),
    /// A big hit — `0..HURT_LINES`.
    Hurt(u8),
    /// Phase 2: raising the dead.
    Phase2,
    /// Phase 3: enraged.
    Phase3,
    /// Killed.
    Death,
}

/// The debug panel's pokes at the Hierophant ([`crate::HierophantDebug`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum HierophantDebugAction {
    /// Set his health to this share (0..=1) of the most he had — his phase
    /// follows it.
    SetHealth(f32),
    /// Raise a ring of the dead now (phase 2 or not).
    RaiseRing,
    /// Teleport now (phase 2 or not).
    Teleport,
    /// Kill him now.
    Kill,
}

pub const PLAYER_DOWN_LINES: u8 = 3;
pub const HURT_LINES: u8 = 3;

impl HierophantLine {
    /// Its file, under `audio/zombies/easter_egg/hierophant/voice/` (made from the clean
    /// recording in `client/voice_src/hierophant/` by `client/tools/voice_layers.sh`).
    pub fn file(self) -> String {
        match self {
            HierophantLine::Taunt(i) => format!("taunt_{}.mp3", i + 1),
            HierophantLine::PlayerDown(i) => format!("player_down_{}.mp3", i + 1),
            HierophantLine::Hurt(i) => format!("hurt_{}.mp3", i + 1),
            HierophantLine::Phase2 => "phase_2.mp3".into(),
            HierophantLine::Phase3 => "phase_3.mp3".into(),
            HierophantLine::Death => "death.mp3".into(),
        }
    }

    /// How long (s) the recording runs — nothing else of his starts over it.
    pub fn secs(self) -> f32 {
        const TAUNT: [f32; 6] = [5.56, 6.03, 4.44, 5.15, 5.15, 4.83];
        const DOWN: [f32; 3] = [2.04, 3.24, 3.24];
        const HURT: [f32; 3] = [1.41, 1.31, 4.28];
        match self {
            HierophantLine::Taunt(i) => TAUNT.get(i as usize).copied().unwrap_or(6.0),
            HierophantLine::PlayerDown(i) => DOWN.get(i as usize).copied().unwrap_or(3.5),
            HierophantLine::Hurt(i) => HURT.get(i as usize).copied().unwrap_or(4.5),
            HierophantLine::Phase2 => 7.31,
            HierophantLine::Phase3 => 5.41,
            HierophantLine::Death => 6.27,
        }
    }

    /// It cuts off whatever he was saying (his phase changing, his death);
    /// anything else waits for him to finish, or isn't said.
    pub fn interrupts(self) -> bool {
        matches!(self, HierophantLine::Phase2 | HierophantLine::Phase3 | HierophantLine::Death)
    }
}

/// A big hit, for a hurt line: this much of his health (of the most he
/// had) gone within [`HURT_WINDOW_SECS`]...
pub const HURT_SHARE: f32 = 0.05;
pub const HURT_WINDOW_SECS: f32 = 0.75;
/// ...at most once this often (s).
pub const HURT_COOLDOWN_SECS: f32 = 9.0;
/// After downing someone, his gloat at most once this often (s).
pub const PLAYER_DOWN_COOLDOWN_SECS: f32 = 6.0;

// --- phase 2 ---------------------------------------------------------------------------

/// Phase 2 starts once he's down to this much of his health...
pub const PHASE_2_AT: f32 = 2.0 / 3.0;
/// ...and he stands roaring this long (s) as it does — raising the first
/// ring of the dead this far (s) in.
pub const PHASE_ROAR_SECS: f32 = 3.0;
pub const RING_RISE_AT_SECS: f32 = 0.8;

/// A ring of the dead he raises (phase 2 on): how many with `players` in
/// the game...
pub fn ring_size(players: usize) -> usize {
    (6 + 2 * players.max(1).saturating_sub(1)).min(RING_MAX)
}
pub const RING_MAX: usize = 12;
/// ...rising this far (m) around him...
pub const RING_RADIUS: (f32, f32) = (3.5, 6.5);
/// ...and another once at most this many of the last are left, at most
/// this often (s, not counting pauses).
pub const RING_REFILL_LEFT: usize = 2;
pub const RING_EVERY_SECS: f32 = 25.0;

/// His teleport (phase 2 on): every so often (a random gap, s — shorter
/// once he's enraged)...
pub const TELEPORT_GAP_SECS: (f32, f32) = (12.0, 18.0);
pub const TELEPORT_GAP_ENRAGED_SECS: (f32, f32) = (8.0, 12.0);
/// ...or once no one's been within this far (m) of him for this long (s)...
pub const TELEPORT_ALONE_DIST: f32 = 10.0;
pub const TELEPORT_ALONE_SECS: f32 = 4.0;
/// ...he sinks into the ground in green smoke over this long (s), is gone
/// for this long, then rises up out of it near a player, over this long —
/// nothing hurting him from the moment he starts sinking until he's up.
pub const TELEPORT_SINK_SECS: f32 = 1.2;
pub const TELEPORT_UNDER_SECS: f32 = 0.8;
pub const TELEPORT_RISE_SECS: f32 = 1.4;
/// He comes up this far (m, across the ground) from whoever he picks.
pub const TELEPORT_NEAR: (f32, f32) = (4.0, 7.0);

/// The whole teleport (s): sinking, gone, rising.
pub const fn teleport_secs() -> f32 {
    TELEPORT_SINK_SECS + TELEPORT_UNDER_SECS + TELEPORT_RISE_SECS
}

// --- phase 3 ---------------------------------------------------------------------------

/// Phase 3 starts once he's down to this much of his health: he stops and
/// roars (as phase 2's, [`PHASE_ROAR_SECS`]) and comes on enraged — faster
/// (m/s, vs [`HIEROPHANT_SPEED`]), teleporting more often
/// ([`TELEPORT_GAP_ENRAGED_SECS`]), still raising his dead. The map goes
/// blood-red, and so does he.
pub const PHASE_3_AT: f32 = 1.0 / 3.0;
pub const HIEROPHANT_ENRAGED_SPEED: f32 = 2.6;

/// His phase (1..=3) with `share` (0..=1) of his health left.
pub fn phase_at(share: f32) -> u8 {
    if share <= PHASE_3_AT {
        3
    } else if share <= PHASE_2_AT {
        2
    } else {
        1
    }
}

// --- his death ([`EggStep::HisDeath`]), in s from it ---------------------------------

/// His body falls (the boss's death), and his spirit tears up out of it
/// then, rising to hang over him, shrieking...
pub const DEATH_SPIRIT_AT: f32 = 1.3;
pub const DEATH_SPIRIT_RISE_SECS: f32 = 1.1;
/// ...until it flies, screaming, up into the pyramid (at its altar, inside
/// it) over this long...
pub const DEATH_FLY_AT: f32 = 2.9;
pub const DEATH_FLY_SECS: f32 = 2.3;
/// ...and once his scream's died away (`death.mp3`, 6.27 s), it's over
/// ([`EggStep::Ascension`]): the map comes back and the round with it.
pub const DEATH_SECS: f32 = 6.8;

/// When his spirit reaches the pyramid.
pub const fn death_spirit_in_secs() -> f32 {
    DEATH_FLY_AT + DEATH_FLY_SECS
}

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
    available(lobby) && (lobby.easter_egg.step.is_unleashed() || lobby.easter_egg.step == EggStep::HisDeath)
}

// --- its pieces (placeholders for now — sizes as the models will be made) ---------

/// The sarcophagus on the upper altar: 3.2 m long (along its own z), 1.6 m
/// wide and 1 m tall, standing on the ground — half extents. Big enough for
/// the Hierophant's body (the boss model) to lie in, arms out, head at -z.
pub const SARCOPHAGUS_HALF_EXTENTS: Vec3 = Vec3::new(0.8, 0.5, 1.6);
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
    fn his_lines_are_where_the_recordings_are() {
        assert_eq!(HierophantLine::Taunt(0).file(), "taunt_1.mp3");
        assert_eq!(HierophantLine::PlayerDown(2).file(), "player_down_3.mp3");
        assert_eq!(HierophantLine::Hurt(1).file(), "hurt_2.mp3");
        assert!(HierophantLine::Death.interrupts() && !HierophantLine::Taunt(0).interrupts());
        assert!(ring_size(1) >= 6 && ring_size(8) == RING_MAX);
    }

    #[test]
    fn his_spirit_is_in_the_pyramid_before_its_over() {
        assert!(DEATH_SPIRIT_AT + DEATH_SPIRIT_RISE_SECS <= DEATH_FLY_AT);
        assert!(death_spirit_in_secs() < DEATH_SECS);
        assert!(DEATH_SECS >= HierophantLine::Death.secs(), "his scream's cut off");
        assert!(!EggStep::HisDeath.is_unleashed(), "the fight's bar, music and red are gone as he falls");
    }

    #[test]
    fn his_phase_follows_his_health() {
        assert_eq!(phase_at(1.0), 1);
        assert_eq!(phase_at(0.67), 1);
        assert_eq!(phase_at(0.66), 2);
        assert_eq!(phase_at(0.34), 2);
        assert_eq!(phase_at(0.33), 3);
        assert_eq!(phase_at(0.01), 3);
        assert!(HIEROPHANT_ENRAGED_SPEED > HIEROPHANT_SPEED);
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
